package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.Levelling
import io.github.jamezuk.lyrionanywhere.core.LocalPlayer
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerConfig
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerObserver
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OutputKind
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.Session
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** This phone as an LMS player ("Play on this phone"). */
data class LocalPlayerState(
    /** The user wants this phone to be a player. */
    val enabled: Boolean = false,
    val status: LocalPlayerStatus? = null,
    val signalPath: SignalPathInfo? = null,
    val error: AppError? = null,
) {
    /** The player's id in LMS, once connected. */
    val playerId: String? get() = status?.playerId?.takeIf { it.isNotEmpty() }

    /** Connected (or reconnecting) to LMS as a player. */
    val running: Boolean
        get() = enabled && status != null && status.connection !is PlayerConnection.Stopped
}

interface LocalPlayerRepository {
    val state: StateFlow<LocalPlayerState>

    /** Turn "Play on this phone" on or off (remembered across restarts). */
    suspend fun setEnabled(on: Boolean): ApiResult<Unit>

    /** Silence the output at once (headphones unplugged, focus lost); see the Rust docs. */
    fun setHold(hold: Boolean)

    /** Lower the volume while another app speaks. */
    fun setDuck(duck: Boolean)

    /** What Android says about the output route, for the signal path. */
    fun setOutputRoute(route: OutputRoute)
}

/** The parts of the Rust [LocalPlayer] the repository uses (a seam for tests). */
interface LocalPlayerHandle {
    fun start(observer: LocalPlayerObserver)

    fun stop()

    fun setHold(hold: Boolean)

    fun setDuck(duck: Boolean)

    fun setOutputRoute(route: OutputRoute)

    fun close()
}

/** Bit-perfect by default: every DSP stage off. */
val DEFAULT_DSP = DspSettings(
    enabled = false,
    preampDb = 0.0,
    levelling = Levelling.OFF,
    targetLufs = -18.0,
    limiter = true,
)

/** Highest rate offered to LMS; it transcodes anything above. */
const val MAX_SAMPLE_RATE = 192_000

private class RustHandle(session: Session) : LocalPlayerHandle {
    private val player = LocalPlayer(
        session,
        LocalPlayerConfig(OutputKind.A_AUDIO, MAX_SAMPLE_RATE.toUInt(), DEFAULT_DSP),
    )

    override fun start(observer: LocalPlayerObserver) = player.start(observer)

    override fun stop() = player.stop()

    override fun setHold(hold: Boolean) = player.setHold(hold)

    override fun setDuck(duck: Boolean) = player.setDuck(duck)

    override fun setOutputRoute(route: OutputRoute) = player.setOutputRoute(route)

    override fun close() = player.close()
}

/**
 * Runs the Rust player while signed in and enabled. The engine has its own thread; its
 * updates land in [state] from there.
 */
class RustLocalPlayerRepository(
    /** The signed-in session (null while signed out). */
    private val sessions: StateFlow<Session?>,
    private val settings: SettingsStore,
    scope: CoroutineScope,
    private val io: CoroutineDispatcher,
    private val factory: (Session) -> LocalPlayerHandle = { RustHandle(it) },
) : LocalPlayerRepository {
    private val _state = MutableStateFlow(LocalPlayerState())
    override val state: StateFlow<LocalPlayerState> = _state.asStateFlow()

    @Volatile private var handle: LocalPlayerHandle? = null
    private var handleSession: Session? = null
    private var route = OutputRoute("", 0u, false)

    /** Ignores updates from a player that has since been replaced. */
    private inner class Observer(private val owner: LocalPlayerHandle) : LocalPlayerObserver {
        override fun onStatus(status: LocalPlayerStatus) {
            if (owner === handle) _state.update { it.copy(status = status) }
        }

        override fun onSignalPath(path: SignalPathInfo?) {
            if (owner === handle) _state.update { it.copy(signalPath = path) }
        }
    }

    init {
        val wanted = settings.settings.map { it.playOnPhone }.distinctUntilChanged()
        scope.launch {
            combine(sessions, wanted) { s, on -> s to on }.collectLatest { (s, on) ->
                reconcile(s.takeIf { on }, on)
            }
        }
    }

    private suspend fun reconcile(session: Session?, enabled: Boolean) = withContext(io) {
        if (session != null && session === handleSession && handle != null) return@withContext
        handle?.let {
            it.stop()
            it.close()
        }
        handle = null
        handleSession = null
        _state.value = LocalPlayerState(enabled = enabled)
        if (session == null) return@withContext
        try {
            val h = factory(session)
            handle = h
            handleSession = session
            h.setOutputRoute(route)
            h.start(Observer(h))
        } catch (e: Exception) {
            handle = null
            handleSession = null
            _state.update { it.copy(error = e.toAppError()) }
        }
    }

    override suspend fun setEnabled(on: Boolean): ApiResult<Unit> {
        if (on && sessions.value == null) return ApiResult.Err(AppError.SignedOut)
        settings.setPlayOnPhone(on)
        return ApiResult.Ok(Unit)
    }

    override fun setHold(hold: Boolean) {
        handle?.setHold(hold)
    }

    override fun setDuck(duck: Boolean) {
        handle?.setDuck(duck)
    }

    override fun setOutputRoute(route: OutputRoute) {
        this.route = route
        handle?.setOutputRoute(route)
    }
}
