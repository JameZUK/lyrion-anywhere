package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.LocalPlayer
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerConfig
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerObserver
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OutputKind
import io.github.jamezuk.lyrionanywhere.core.OutputNegotiator
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.Session
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.checkEqProfile
import io.github.jamezuk.lyrionanywhere.core.removeStreamSpools
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
    /** The Sound settings (DSP and bit-perfect output). */
    val sound: SoundSettings = SoundSettings(),
    /** The live stream's level now (by the network and the Streaming quality setting). */
    val streamQuality: StreamQuality = StreamQuality.ORIGINAL,
    /** On mobile data (a metered network). */
    val metered: Boolean = false,
    /** Where the sound goes now (null: not known yet), for per-output EQ (Wave 2 §5.3). */
    val output: AudioOutput? = null,
    /** The EQ is bypassed for a comparison (A/B), until released. */
    val eqBypass: Boolean = false,
    /** Reading the next track ahead is wanted now (the settings, on this network). */
    val readAhead: Boolean = true,
    /** The phone has a network (as Android says). */
    val networkAvailable: Boolean = true,
) {
    /**
     * How much listening is on the phone ahead of what is heard (ms), to show as "4 min
     * buffered ahead": only while the player reads ahead, and null when nothing is.
     */
    val cushionMs: Long?
        get() = status?.takeIf { it.readAhead && it.connection !is PlayerConnection.Stopped }
            ?.cushionMs?.toLong()?.takeIf { it >= 1_000 }

    /** The Sound settings as they apply on the current output. */
    val effectiveSound: SoundSettings get() = sound.forOutput(output)

    /** The EQ profile in use now (null: none, off, or bypassed). */
    val eqInUse: EqProfile? get() = if (eqBypass) null else effectiveSound.let { if (it.eqEnabled) it.eq else null }

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

    /**
     * Fade the output out before the sleep timer ends, [endsInSecs] from now, over
     * [fadeSecs] (0–120; 0: no fade). Null cancels. The engine runs the fade, so it carries
     * on in the background.
     */
    fun setSleepFade(endsInSecs: Double?, fadeSecs: Int) {}

    /** What Android says about the output route, for the signal path. */
    fun setOutputRoute(route: OutputRoute)

    /** Change the Sound settings (remembered; they apply live, to audio processed from now). */
    suspend fun setSound(sound: SoundSettings) {}

    /** Check an AutoEq profile before importing it. */
    fun checkEq(text: String): EqCheck = EqCheck.Invalid("not available")

    /** Android's side of bit-perfect USB output (null: none). */
    fun setOutputNegotiator(negotiator: OutputNegotiator?) {}

    /** The output device changed (a USB DAC came or went): negotiate the output again. */
    fun renegotiateOutput() {}

    /**
     * Analyse what this phone plays for a visualiser (Wave 2 §4.1), or stop. Off by default;
     * on only while a visualiser is on screen. Remembered across restarts of the engine.
     */
    fun setVisualiser(on: Boolean) {}

    /** The newest visualiser frame since the last call (null: none, or off). */
    fun visualFrame(): VisualFrame? = null

    /** The output changed (Wave 2 §5.3): the EQ bound to it, if any, applies from now. */
    fun setOutput(output: AudioOutput?) {}

    /** Bypass the EQ for a comparison (A/B), or stop. Never remembered. */
    fun setEqBypass(on: Boolean) {}

    /**
     * The phone's media volume as an attenuation in dB (0: full), which loudness
     * compensation follows while this phone plays; null when the phone's volume says
     * nothing about the level heard (a Bluetooth headset with its own volume), so the
     * player's level in LMS alone counts. Kept across restarts of the engine.
     */
    fun setLoudnessReferenceDb(db: Double?) {}
}

/** The engine's DSP for [sound] on [output], with the EQ left out while [bypass]. */
fun dspOn(sound: SoundSettings, output: AudioOutput?, bypass: Boolean): DspSettings {
    val s = sound.forOutput(output)
    return dspFor(if (bypass) s.copy(eqEnabled = false) else s)
}

/** The parts of the Rust [LocalPlayer] the repository uses (a seam for tests). */
interface LocalPlayerHandle {
    fun start(observer: LocalPlayerObserver)

    fun stop()

    fun setHold(hold: Boolean)

    fun setDuck(duck: Boolean)

    fun setSleepFade(endsInSecs: Double?, fadeSecs: UInt) {}

    fun setOutputRoute(route: OutputRoute)

    fun setDsp(dsp: DspSettings)

    fun setOutputNegotiator(negotiator: OutputNegotiator?)

    fun renegotiateOutput()

    fun setStreamQuality(quality: StreamQuality) {}

    fun setVisualiser(on: Boolean) {}

    /** The engine's packed frame (see [VisualFrame.parse]). */
    fun visualFrame(): ByteArray? = null

    fun setLoudnessReferenceDb(db: Double?) {}

    /** Read the next track ahead (rolling cache, Phase A). */
    fun setReadAhead(on: Boolean) {}

    /** Whether the phone has a network (no reconnecting without one). */
    fun setNetworkAvailable(available: Boolean) {}

    fun close()
}

/** Whether the player is told the phone has a network: anything but offline. */
fun networkAvailable(kind: NetworkKind): Boolean = kind != NetworkKind.OFFLINE

/** The engine's spool clean-up (every server's and user's), ignoring what can't be done. */
fun rustClearSpools(cacheDir: String) {
    runCatching { removeStreamSpools(cacheDir) }
}

/** The stream level for the network: the mobile-data setting on a metered network. */
fun streamQualityFor(metered: Boolean, s: StreamingSettings): StreamQuality = if (metered) s.mobile else s.wifi

/** A sleep timer end that moved less than this is the same timer (polling drift). */
private const val FADE_SAME_MS = 2_000L

/** Highest rate offered to LMS; it transcodes anything above. */
const val MAX_SAMPLE_RATE = 192_000

private class RustHandle(session: Session, cacheDir: String?, readAhead: Boolean) : LocalPlayerHandle {
    private val player = LocalPlayer(
        session,
        LocalPlayerConfig(OutputKind.A_AUDIO, MAX_SAMPLE_RATE.toUInt(), DEFAULT_DSP, cacheDir, readAhead),
    )

    override fun start(observer: LocalPlayerObserver) = player.start(observer)

    override fun stop() = player.stop()

    override fun setHold(hold: Boolean) = player.setHold(hold)

    override fun setDuck(duck: Boolean) = player.setDuck(duck)

    override fun setSleepFade(endsInSecs: Double?, fadeSecs: UInt) = player.setSleepFade(endsInSecs, fadeSecs)

    override fun setOutputRoute(route: OutputRoute) = player.setOutputRoute(route)

    override fun setDsp(dsp: DspSettings) = player.setDsp(dsp)

    override fun setOutputNegotiator(negotiator: OutputNegotiator?) = player.setOutputNegotiator(negotiator)

    override fun renegotiateOutput() = player.renegotiateOutput()

    override fun setStreamQuality(quality: StreamQuality) = player.setStreamQuality(quality)

    override fun setVisualiser(on: Boolean) = player.setVisualiser(on)

    override fun visualFrame(): ByteArray? = player.visualFrame()

    override fun setLoudnessReferenceDb(db: Double?) = player.setLoudnessReferenceDb(db)

    override fun setReadAhead(on: Boolean) = player.setReadAhead(on)

    override fun setNetworkAvailable(available: Boolean) = player.setNetworkAvailable(available)

    override fun close() = player.close()
}

/** The engine's own AutoEq check (bounded parser, fuzzed in Rust). */
fun rustEqCheck(text: String): EqCheck =
    try {
        val s = checkEqProfile(text)
        EqCheck.Ok(s.filters.toInt(), s.preampDb, s.flat)
    } catch (e: ApiException) {
        EqCheck.Invalid((e.toAppError() as? AppError.InvalidInput)?.detail ?: "not a profile")
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
    private val eqCheck: (String) -> EqCheck = ::rustEqCheck,
    /** Tests: makes the engine (null: the Rust player, spooling to [cacheDir]). */
    private val factory: ((Session) -> LocalPlayerHandle)? = null,
    /** A monotonic clock (ms), for the sleep fade. */
    private val nowMs: () -> Long = { System.nanoTime() / 1_000_000 },
    /** Wi-Fi or mobile data, for the stream level (none: as on Wi-Fi). */
    private val network: StateFlow<NetworkKind>? = null,
    /**
     * An app-private folder (under `noBackupFilesDir`, beside downloads) where the player
     * spools live streams; null keeps them in memory (and reads nothing ahead).
     */
    private val cacheDir: String? = null,
    /** Deletes every spool under the folder (at sign-out, revocation and server change). */
    private val clearSpools: (String) -> Unit = ::rustClearSpools,
) : LocalPlayerRepository {
    private val _state = MutableStateFlow(LocalPlayerState())
    override val state: StateFlow<LocalPlayerState> = _state.asStateFlow()

    @Volatile private var handle: LocalPlayerHandle? = null
    private var handleSession: Session? = null
    private var route = OutputRoute("", 0u)
    @Volatile private var sound = SoundSettings()
    @Volatile private var negotiator: OutputNegotiator? = null
    @Volatile private var quality = StreamQuality.ORIGINAL
    @Volatile private var visualiser = false
    @Volatile private var output: AudioOutput? = null
    @Volatile private var bypass = false
    @Volatile private var loudnessReference: Double? = null
    @Volatile private var readAhead = true
    @Volatile private var networkUp = true

    /** The session the player last followed (null: signed out), to notice a change of it. */
    private var lastSession: Session? = null

    /** The DSP last given to the engine (or that it started with). */
    @Volatile private var applied: DspSettings = DEFAULT_DSP

    private fun dspNow() = dspOn(sound, output, bypass)

    /** Give the engine the DSP for the settings, output and bypass now, if it changed. */
    @Synchronized
    private fun applyDsp() {
        val h = handle ?: return
        val dsp = dspNow()
        if (dsp != applied) {
            applied = dsp
            h.setDsp(dsp)
        }
    }

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
                reconcile(s.takeIf { on }, on, s)
            }
        }
        scope.launch {
            settings.settings.map { it.sound }.distinctUntilChanged().collect { s ->
                val before = sound
                sound = s
                _state.update { it.copy(sound = s) }
                withContext(io) {
                    val h = handle ?: return@withContext
                    applyDsp()
                    // Turning bit-perfect USB on or off changes what the output may ask for.
                    if (s.bitPerfectUsb != before.bitPerfectUsb) h.renegotiateOutput()
                }
            }
        }
        scope.launch {
            // The stream level follows the network: the setting for Wi-Fi or for mobile data.
            val metered = (network ?: MutableStateFlow(NetworkKind.WIFI)).map { it == NetworkKind.MOBILE }
            combine(settings.settings.map { it.streaming }, metered) { s, m -> m to streamQualityFor(m, s) }
                .distinctUntilChanged()
                .collect { (m, q) ->
                    quality = q
                    _state.update { it.copy(streamQuality = q, metered = m) }
                    withContext(io) { handle?.setStreamQuality(q) }
                }
        }
        scope.launch {
            // Read ahead by the settings and the network (Gaps in the signal), applied live.
            val kinds = network ?: MutableStateFlow(NetworkKind.WIFI)
            combine(settings.settings.map { it.readAhead }, kinds) { s, k -> s.effective(k == NetworkKind.MOBILE) }
                .distinctUntilChanged()
                .collect { on ->
                    readAhead = on
                    _state.update { it.copy(readAhead = on) }
                    withContext(io) { handle?.setReadAhead(on) }
                }
        }
        scope.launch {
            // No network: the player stops trying to reconnect until it is back.
            (network ?: MutableStateFlow(NetworkKind.WIFI)).map(::networkAvailable).distinctUntilChanged().collect { up ->
                networkUp = up
                _state.update { it.copy(networkAvailable = up) }
                withContext(io) { handle?.setNetworkAvailable(up) }
            }
        }
    }

    private suspend fun reconcile(session: Session?, enabled: Boolean, signedIn: Session?) = withContext(io) {
        val changed = lastSession != null && signedIn !== lastSession
        lastSession = signedIn
        if (session != null && session === handleSession && handle != null) return@withContext
        handle?.let {
            it.stop()
            it.close()
        }
        handle = null
        handleSession = null
        // Signed out, revoked, or another server or user: nothing of the old stream stays.
        if (changed) cacheDir?.let(clearSpools)
        _state.value = LocalPlayerState(
            enabled = enabled,
            sound = sound,
            streamQuality = quality,
            metered = _state.value.metered,
            output = output,
            eqBypass = bypass,
            readAhead = readAhead,
            networkAvailable = networkUp,
        )
        if (session == null) return@withContext
        try {
            val h = factory?.invoke(session) ?: RustHandle(session, cacheDir, readAhead)
            handle = h
            handleSession = session
            fade = null // a new engine has no fade yet
            h.setOutputRoute(route)
            negotiator?.let { h.setOutputNegotiator(it) }
            // The engine starts bit-perfect; any DSP is set before any audio.
            val dsp = dspNow()
            applied = dsp
            if (dsp != DEFAULT_DSP) h.setDsp(dsp)
            h.setStreamQuality(quality)
            h.setVisualiser(visualiser)
            h.setLoudnessReferenceDb(loudnessReference)
            h.setReadAhead(readAhead)
            h.setNetworkAvailable(networkUp)
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

    /** The fade last passed to the engine: when it ends (ms, [nowMs]'s clock) and its length. */
    private var fade: Pair<Long?, Int>? = null

    override fun setSleepFade(endsInSecs: Double?, fadeSecs: Int) {
        val ends = endsInSecs?.takeIf { it.isFinite() && it > 0 && it <= MAX_SLEEP_SECONDS }
        val length = fadeSecs.coerceIn(0, SLEEP_FADE_MAX)
        val endsAt = ends?.let { nowMs() + (it * 1000).toLong() }
        // Every status repeats the timer: pass it on only when it has changed (by more than
        // the drift between polls), so the engine's fade isn't restarted each time.
        val last = fade
        if (last != null && last.second == length &&
            (last.first == null && endsAt == null || last.first != null && endsAt != null && kotlin.math.abs(last.first!! - endsAt) < FADE_SAME_MS)
        ) {
            return
        }
        fade = endsAt to length
        handle?.setSleepFade(ends, length.toUInt())
    }

    override fun setOutputRoute(route: OutputRoute) {
        this.route = route
        handle?.setOutputRoute(route)
    }

    override suspend fun setSound(sound: SoundSettings) {
        settings.setSound(sound.sanitised())
    }

    override fun checkEq(text: String): EqCheck =
        if (text.length > SoundSettings.MAX_EQ_CHARS) {
            EqCheck.Invalid("the profile is larger than 64 KiB")
        } else {
            eqCheck(text)
        }

    override fun setOutputNegotiator(negotiator: OutputNegotiator?) {
        this.negotiator = negotiator
        handle?.setOutputNegotiator(negotiator)
    }

    override fun renegotiateOutput() {
        handle?.renegotiateOutput()
    }

    override fun setVisualiser(on: Boolean) {
        visualiser = on
        handle?.setVisualiser(on)
    }

    override fun visualFrame(): VisualFrame? = if (visualiser) VisualFrame.parse(handle?.visualFrame()) else null

    override fun setOutput(output: AudioOutput?) {
        val o = output?.let { AudioOutput.of(it.kind, it.name) }
        if (o == this.output) return
        this.output = o
        _state.update { it.copy(output = o) }
        applyDsp()
    }

    override fun setLoudnessReferenceDb(db: Double?) {
        val clean = cleanLoudnessReference(db)
        if (clean == loudnessReference) return
        loudnessReference = clean
        handle?.setLoudnessReferenceDb(clean)
    }

    override fun setEqBypass(on: Boolean) {
        if (on == bypass) return
        bypass = on
        _state.update { it.copy(eqBypass = on) }
        applyDsp()
    }
}

/**
 * The phone's media-volume attenuation as the engine takes it: null stays null (not known),
 * NaN is not known, anything else is held to 0–120 dB (muted is the most). The engine checks
 * again; this keeps repeated identical values from being sent.
 */
fun cleanLoudnessReference(db: Double?): Double? = when {
    db == null || db.isNaN() -> null
    else -> db.coerceIn(0.0, MAX_LOUDNESS_REFERENCE_DB)
}

/** The most attenuation the phone's volume is taken to apply (dB); muted counts as this. */
const val MAX_LOUDNESS_REFERENCE_DB = 120.0
