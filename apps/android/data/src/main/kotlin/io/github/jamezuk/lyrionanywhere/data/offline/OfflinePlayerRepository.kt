package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerConfig
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.OfflinePlayer
import io.github.jamezuk.lyrionanywhere.core.OfflinePlayerObserver
import io.github.jamezuk.lyrionanywhere.core.OfflinePlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.OutputKind
import io.github.jamezuk.lyrionanywhere.core.OutputNegotiator
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.MAX_SAMPLE_RATE
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.dspFor
import io.github.jamezuk.lyrionanywhere.data.toAppError
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext

/** Downloads playing on this phone from the app's own queue (SPEC D14). */
data class OfflinePlayerState(
    val status: OfflinePlayerStatus? = null,
    /** The queue's tracks, by stable key (titles, artists, albums). */
    val tracks: Map<String, OfflineTrack> = emptyMap(),
    val signalPath: SignalPathInfo? = null,
    val error: AppError? = null,
    /** The Sound settings (applied to downloads as to streaming). */
    val sound: SoundSettings = SoundSettings(),
) {
    /** A queue is loaded (playing, paused or ended at a track). */
    val active: Boolean get() = status?.index != null

    val playing: Boolean
        get() = status?.playback == LocalPlayback.PLAYING || status?.playback == LocalPlayback.BUFFERING

    val currentIndex: Int? get() = status?.index?.toInt()

    val current: OfflineTrack?
        get() = status?.let { s -> s.index?.toInt()?.let { s.keys.getOrNull(it) }?.let(tracks::get) }

    /** The queue in play order. */
    val queue: List<OfflineTrack>
        get() = status?.let { s -> s.order.mapNotNull { i -> s.keys.getOrNull(i.toInt())?.let(tracks::get) } }.orEmpty()
}

interface OfflinePlayerRepository {
    val state: StateFlow<OfflinePlayerState>

    /** Play downloaded [tracks] from [start] (an index into them). */
    fun play(tracks: List<OfflineTrack>, start: Int = 0, shuffle: Boolean = false)

    fun play()

    fun pause()

    fun toggle() = if (state.value.playing) pause() else play()

    fun next()

    fun previous()

    fun seekTo(ms: Long)

    /** Play the queue's track at [index] (an index into the list given to [play]). */
    fun jumpTo(index: Int)

    fun setShuffle(on: Boolean)

    fun setRepeat(repeat: LocalRepeat)

    /** Stop and forget the queue. */
    fun stop()

    fun setHold(hold: Boolean) {}

    fun setDuck(duck: Boolean) {}

    fun setOutputRoute(route: OutputRoute) {}

    fun setOutputNegotiator(negotiator: OutputNegotiator?) {}

    fun renegotiateOutput() {}
}

/** The parts of the Rust [OfflinePlayer] used here (a seam for tests). */
interface OfflinePlayerHandle {
    fun start(observer: OfflinePlayerObserver)

    fun playTracks(keys: List<String>, start: Int, shuffle: Boolean): Int

    fun play()

    fun pause()

    fun next()

    fun previous()

    fun jumpTo(index: Int)

    fun seekTo(ms: Long)

    fun setShuffle(on: Boolean)

    fun setRepeat(repeat: LocalRepeat)

    fun stop()

    fun setDsp(dsp: DspSettings)

    fun setHold(hold: Boolean)

    fun setDuck(duck: Boolean)

    fun setOutputRoute(route: OutputRoute)

    fun setOutputNegotiator(negotiator: OutputNegotiator?)

    fun renegotiateOutput()

    fun shutdown()
}

private class RustOfflinePlayer(store: OfflineStore, dsp: DspSettings) : OfflinePlayerHandle {
    private val p = OfflinePlayer(
        (store as RustOfflineStore).library,
        LocalPlayerConfig(OutputKind.A_AUDIO, MAX_SAMPLE_RATE.toUInt(), dsp),
    )

    override fun start(observer: OfflinePlayerObserver) = p.start(observer)

    override fun playTracks(keys: List<String>, start: Int, shuffle: Boolean) =
        p.playTracks(keys, start.coerceAtLeast(0).toUInt(), shuffle, true).toInt()

    override fun play() = p.play()

    override fun pause() = p.pause()

    override fun next() = p.next()

    override fun previous() = p.previous()

    override fun jumpTo(index: Int) = p.jumpTo(index.coerceAtLeast(0).toUInt())

    override fun seekTo(ms: Long) = p.seekTo(ms.coerceAtLeast(0).toULong())

    override fun setShuffle(on: Boolean) = p.setShuffle(on)

    override fun setRepeat(repeat: LocalRepeat) = p.setRepeat(repeat)

    override fun stop() = p.stop()

    override fun setDsp(dsp: DspSettings) = p.setDsp(dsp)

    override fun setHold(hold: Boolean) = p.setHold(hold)

    override fun setDuck(duck: Boolean) = p.setDuck(duck)

    override fun setOutputRoute(route: OutputRoute) = p.setOutputRoute(route)

    override fun setOutputNegotiator(negotiator: OutputNegotiator?) = p.setOutputNegotiator(negotiator)

    override fun renegotiateOutput() = p.renegotiateOutput()

    override fun shutdown() {
        p.shutdown()
        p.close()
    }
}

/**
 * Runs the offline player for the open offline library. The engine starts on the first
 * play and ends when the library closes (another user, sign-out); its updates arrive on
 * its own thread.
 */
class RustOfflinePlayerRepository(
    private val store: StateFlow<OfflineStore?>,
    private val settings: SettingsStore,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher,
    private val factory: (OfflineStore, DspSettings) -> OfflinePlayerHandle = { s, d -> RustOfflinePlayer(s, d) },
) : OfflinePlayerRepository {
    private val _state = MutableStateFlow(OfflinePlayerState())
    override val state: StateFlow<OfflinePlayerState> = _state.asStateFlow()

    private val lock = Mutex()
    @Volatile private var handle: OfflinePlayerHandle? = null
    private var handleStore: OfflineStore? = null
    @Volatile private var sound = SoundSettings()
    @Volatile private var route = OutputRoute("", 0u)
    @Volatile private var negotiator: OutputNegotiator? = null
    @Volatile private var hold = false
    @Volatile private var duck = false

    /** Ignores updates from an engine that has since been replaced. */
    private inner class Observer(private val owner: OfflinePlayerHandle) : OfflinePlayerObserver {
        override fun onStatus(status: OfflinePlayerStatus) {
            if (owner === handle) _state.update { it.copy(status = status) }
        }

        override fun onSignalPath(path: SignalPathInfo?) {
            if (owner === handle) _state.update { it.copy(signalPath = path) }
        }
    }

    init {
        scope.launch {
            store.collect { s ->
                lock.withLock { if (s !== handleStore) release() }
            }
        }
        scope.launch {
            settings.settings.map { it.sound }.distinctUntilChanged().collect { s ->
                sound = s
                _state.update { it.copy(sound = s) }
                val h = handle ?: return@collect
                withContext(io) { h.setDsp(dspFor(s)) }
            }
        }
    }

    private suspend fun release() {
        val h = handle
        handle = null
        handleStore = null
        _state.update { OfflinePlayerState(sound = it.sound) }
        if (h != null) withContext(io) { runCatching { h.shutdown() } }
    }

    /** The engine for the open library, started if needed. */
    private suspend fun engine(): OfflinePlayerHandle? = lock.withLock {
        val s = store.value ?: return@withLock null
        handle?.takeIf { handleStore === s }?.let { return@withLock it }
        release()
        val h = withContext(io) {
            runCatching {
                factory(s, dspFor(sound)).also {
                    it.setOutputRoute(route)
                    negotiator?.let(it::setOutputNegotiator)
                    if (duck) it.setDuck(true)
                    it.start(Observer(it))
                }
            }
        }
        h.exceptionOrNull()?.let { e -> _state.update { it.copy(error = (e as? Exception)?.toAppError()) } }
        handle = h.getOrNull()
        handleStore = s.takeIf { handle != null }
        handle
    }

    private fun send(block: (OfflinePlayerHandle) -> Unit) {
        scope.launch {
            val h = handle ?: return@launch
            withContext(io) { runCatching { block(h) } }
        }
    }

    override fun play(tracks: List<OfflineTrack>, start: Int, shuffle: Boolean) {
        if (tracks.isEmpty()) return
        scope.launch {
            val h = engine() ?: return@launch
            _state.update { st -> st.copy(tracks = tracks.associateBy { it.key }, error = null) }
            val r = withContext(io) { runCatching { h.playTracks(tracks.map { it.key }, start, shuffle) } }
            r.exceptionOrNull()?.let { e -> _state.update { it.copy(error = (e as? Exception)?.toAppError()) } }
        }
    }

    override fun play() {
        hold = false
        send { it.play() }
    }

    override fun pause() = send { it.pause() }

    override fun next() = send { it.next() }

    override fun previous() = send { it.previous() }

    override fun seekTo(ms: Long) = send { it.seekTo(ms) }

    override fun jumpTo(index: Int) = send { it.jumpTo(index) }

    override fun setShuffle(on: Boolean) = send { it.setShuffle(on) }

    override fun setRepeat(repeat: LocalRepeat) = send { it.setRepeat(repeat) }

    override fun stop() = send { it.stop() }

    override fun setHold(hold: Boolean) {
        this.hold = hold
        handle?.setHold(hold)
    }

    override fun setDuck(duck: Boolean) {
        this.duck = duck
        handle?.setDuck(duck)
    }

    override fun setOutputRoute(route: OutputRoute) {
        this.route = route
        handle?.setOutputRoute(route)
    }

    override fun setOutputNegotiator(negotiator: OutputNegotiator?) {
        this.negotiator = negotiator
        handle?.setOutputNegotiator(negotiator)
    }

    override fun renegotiateOutput() {
        handle?.renegotiateOutput()
    }
}
