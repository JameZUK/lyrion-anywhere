package io.github.jamezuk.lyrionanywhere.playback

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ServiceInfo
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.net.wifi.WifiManager
import android.os.Handler
import android.os.Looper
import android.os.PowerManager
import android.os.SystemClock
import androidx.core.content.ContextCompat
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaLibraryService
import androidx.media3.session.MediaSession
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoArtGrants
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoCatalogue
import io.github.jamezuk.lyrionanywhere.playback.auto.CallerAccess
import io.github.jamezuk.lyrionanywhere.playback.auto.CallerFactsReader
import io.github.jamezuk.lyrionanywhere.playback.auto.CallerPolicy
import io.github.jamezuk.lyrionanywhere.playback.auto.LibraryCallback
import io.github.jamezuk.lyrionanywhere.playback.auto.ResourceLabels
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.OutputClass
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * The media session for this phone's playback: notification, lock screen, Bluetooth and
 * headset buttons, audio focus, "becoming noisy", and wake and Wi-Fi locks while playing.
 *
 * It serves both of the phone's sources (SPEC D14): this phone as an LMS player ("Play on
 * this phone"), and downloads played from the app's own queue (offline). The session shows
 * whichever is active; the downloads take the session while their queue is loaded.
 *
 * Media3 makes this a foreground service (type `mediaPlayback`) while playback is on.
 *
 * It is also the media library for cars (A6): Android Auto and Android Automotive browse
 * the library, search and play through it ([AutoCatalogue]), always on this phone (SPEC
 * D20), or from the downloads when there's no server. So it is **exported**, and every
 * caller is checked in `onConnect` ([CallerPolicy]): Android Auto and the assistants only
 * with their release certificates, the system's own controllers, and this app; anything
 * else Media3 trusts gets transport controls only, and the rest is refused.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaLibraryService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: MediaLibrarySession? = null
    private var ready = false
    private var catalogue: AutoCatalogue? = null
    private val callers by lazy { CallerFactsReader(this) }

    /** Decisions per caller (uid and package) for this service's life. */
    private val decided = HashMap<Pair<Int, String>, CallerAccess>()

    private fun access(controller: MediaSession.ControllerInfo): CallerAccess {
        if (!ready) return CallerAccess.REJECT
        val key = controller.uid to controller.packageName
        return synchronized(decided) {
            decided.getOrPut(key) {
                val facts = callers.read(controller.packageName, controller.uid, controller.isTrusted)
                CallerPolicy.decide(facts).also { host.carConnections?.record(facts, it) }
            }
        }
    }
    private lateinit var controller: LocalPlaybackController
    private lateinit var host: PlaybackHost
    private lateinit var focus: AudioFocusHandler
    private lateinit var locks: PlaybackLocks
    private val keeper = LockKeeper()
    private lateinit var audio: AudioManager
    private var volumeFollower: PhoneVolumeFollower? = null
    private val policy = AudioFocusPolicy()
    private lateinit var playFocus: PlayFocusGate
    private var noisyRegistered = false
    private var playing = false
    private var lmsPlayer: LmsSessionPlayer? = null
    private var offlineSessionPlayer: OfflineSessionPlayer? = null

    /** Downloads have the session (their queue is loaded). */
    private val offlineActive: Boolean get() = host.offlinePlayer?.state?.value?.active == true

    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                policy.userPaused()
                val offline = host.offlinePlayer
                if (offline != null && offlineActive) {
                    offline.setHold(true)
                    offline.pause()
                } else {
                    scope.launch { controller.becomingNoisy() }
                }
            }
        }
    }

    private val devices = object : AudioDeviceCallback() {
        override fun onAudioDevicesAdded(added: Array<out AudioDeviceInfo>) = devicesChanged()

        override fun onAudioDevicesRemoved(removed: Array<out AudioDeviceInfo>) = devicesChanged()
    }

    /** The USB DAC the output was last negotiated for (null: none). */
    private var usbDac: OutputDevice? = null

    override fun onCreate() {
        super.onCreate()
        host = application as PlaybackHost
        if (!host.playbackReady) {
            // Safe mode (the last run crashed): nothing is set up, and no one may connect.
            stopSelf()
            return
        }
        ready = true
        catalogue = host.autoSource?.let { AutoCatalogue(it, ResourceLabels(this), art = host.autoArt) }
        val requests = catalogue?.let { c -> PlayRequests { id, query -> c.play(id, query) } }
        audio = getSystemService(AudioManager::class.java)
        // Loudness compensation follows the phone's media volume while this phone plays.
        volumeFollower = PhoneVolumeFollower(this, audio) { db ->
            host.localPlayer.setLoudnessReferenceDb(db)
            host.offlinePlayer?.setLoudnessReferenceDb(db)
        }
        locks = PlaybackLocks(this)
        controller = LocalPlaybackController(
            host.localPlayer,
            host.players,
            host.events,
            host.art,
            scope,
            SystemClock::elapsedRealtime,
        )
        controller.start()
        val offline = host.offlinePlayer
        focus = AudioFocusHandler(audio) { change ->
            for (action in policy.onChange(change, playing)) {
                when (action) {
                    FocusAction.Pause -> pauseForFocus()
                    FocusAction.Resume -> if (offline != null && offlineActive) {
                        offline.play()
                    } else {
                        scope.launch { controller.play() }
                    }
                    is FocusAction.Duck -> {
                        host.localPlayer.setDuck(action.on)
                        offline?.setDuck(action.on)
                    }
                }
            }
        }
        // Focus when playback starts: the foreground first, then focus; a refusal the
        // background rule explains is played through (see [PlayFocusGate]).
        playFocus = PlayFocusGate(
            scope = scope,
            // Media3 takes the foreground once the session reports playing; nudge it now.
            bringToForeground = { if (isPlaybackOngoing) triggerNotificationUpdate() },
            request = focus::request,
            onScreen = { host.appForeground?.value == true },
            serviceForeground = { foregroundServiceType != ServiceInfo.FOREGROUND_SERVICE_TYPE_NONE },
            inCall = { audio.mode in CALL_MODES },
            clock = SystemClock::elapsedRealtime,
        ) { decision ->
            when (decision) {
                FocusDecision.GRANTED -> Unit
                // Refused (a call is in progress): don't play over it.
                FocusDecision.PAUSE -> if (playing) pauseForFocus()
                // Refused only because the app is in the background (Android 15+): pausing
                // would pause the whole sync group, so play on, and note it.
                FocusDecision.PLAY_WITHOUT_FOCUS -> host.backgroundPlayback?.focusRefusedInBackground()
            }
        }
        val player = LmsSessionPlayer(Looper.getMainLooper(), controller.snapshot, controller, scope, requests)
        lmsPlayer = player
        if (offline != null) {
            val offlineController = OfflinePlaybackController(offline, host::offlineArtwork, scope, SystemClock::elapsedRealtime)
            offlineController.start()
            offlineSessionPlayer =
                OfflineSessionPlayer(Looper.getMainLooper(), offlineController.snapshot, offline, scope, requests)
        }
        val launch = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
        // Covers in the car's lists: each URI granted to Android Auto alone, read-only.
        val callback = LibraryCallback(::access, { catalogue }, scope, host.autoArt?.let { AutoArtGrants.forContext(this) })
        session = MediaLibrarySession.Builder(this, if (offlineActive) offlineSessionPlayer ?: player else player, callback)
            .apply { launch?.let { setSessionActivity(it) } }
            .build()
        // Android refused the foreground (playback started or came back while the app was
        // in the background, with battery optimisation on): noted, so Settings can offer to
        // let the app run unrestricted. Media3 shows the notification without it meanwhile.
        host.backgroundPlayback?.let { bg ->
            setListener(object : Listener {
                override fun onForegroundServiceStartNotAllowedException() = bg.foregroundRefused()
            })
        }
        host.appForeground?.let { fg ->
            scope.launch {
                // The app opened (Android lets a service go to the foreground then): if
                // playback carried on without the foreground because Android refused it in
                // the background, take it back now. Media3 otherwise tries again only when
                // the player's state next changes.
                fg.collect { open -> if (open && isPlaybackOngoing) triggerNotificationUpdate() }
            }
        }
        // Bit-perfect USB output: the engines ask this before opening their output.
        val negotiator = UsbDacNegotiator(audio) { host.localPlayer.state.value.sound.bitPerfectUsb }
        host.localPlayer.setOutputNegotiator(negotiator)
        offline?.setOutputNegotiator(negotiator)
        usbDac = currentUsbDac()
        audio.registerAudioDeviceCallback(devices, Handler(Looper.getMainLooper()))
        updateRoute()

        val offlineFlow = offline?.state?.map { it.playing to it.active } ?: flowOf(false to false)
        scope.launch {
            combine(
                controller.snapshot.map { mapSnapshot(it).playWhenReady to it.local.enabled },
                offlineFlow,
            ) { (lmsWants, enabled), (offlinePlaying, active) ->
                // Downloads have the session while their queue is loaded.
                val wants = if (active) offlinePlaying else lmsWants
                wants to (enabled || active)
            }
                .distinctUntilChanged()
                .collect { (wantsToPlay, enabled) -> onPlayingChanged(wantsToPlay, enabled) }
        }
        scope.launch {
            // The wake and Wi-Fi locks: while playing, while the engine starts a track (LMS
            // may not have said so yet), and for a while after, so a track change or a
            // queue replaced from elsewhere never finds the phone asleep (see [LockKeeper]).
            combine(
                controller.snapshot.map { (mapSnapshot(it).playWhenReady || it.local.engineActive) to it.local.enabled },
                offlineFlow,
            ) { (lmsActive, enabled), (offlinePlaying, active) ->
                (if (active) offlinePlaying else lmsActive) to (enabled || active)
            }
                .distinctUntilChanged()
                .collectLatest { (active, enabled) ->
                    while (true) {
                        val (hold, recheckIn) = keeper.decide(enabled, active, SystemClock.elapsedRealtime())
                        locks.hold(hold)
                        delay(recheckIn ?: break)
                    }
                }
        }
        scope.launch {
            offlineFlow.map { it.second }.distinctUntilChanged().collect { active ->
                // The session shows (and its buttons drive) whichever source is active.
                val target = if (active) offlineSessionPlayer else lmsPlayer
                session?.let { s -> if (target != null && s.player !== target) s.player = target }
            }
        }
        host.nowPlaying?.let { hub ->
            scope.launch {
                // The widget and tile follow this phone's player from what the session
                // already knows; nothing extra is fetched for them.
                controller.snapshot.map { it.nowPlaying() }.distinctUntilChanged().collect(hub::publishPhone)
            }
        }
        scope.launch {
            // Keep the wake lock's timeout fresh while held (each track change).
            controller.snapshot.map { it.status?.current?.id }.distinctUntilChanged().collect {
                if (locks.held) locks.hold(true)
            }
        }
    }

    /** Silence at once and pause whichever source plays (focus lost or refused). */
    private fun pauseForFocus() {
        val offline = host.offlinePlayer
        if (offline != null && offlineActive) {
            offline.setHold(true)
            offline.pause()
        } else {
            host.localPlayer.setHold(true)
            scope.launch { controller.pause() }
        }
    }

    private fun onPlayingChanged(wantsToPlay: Boolean, enabled: Boolean) {
        if (!enabled) {
            // "Play on this phone" was switched off and no downloads are loaded.
            playing = false
            playFocus.onStop()
            setNoisyReceiver(false)
            volumeFollower?.setActive(false)
            focus.abandon()
            stopSelf()
            return
        }
        playing = wantsToPlay
        volumeFollower?.setActive(wantsToPlay)
        if (wantsToPlay) {
            setNoisyReceiver(true)
            // Once per start of playback; a refusal pauses only for a real conflict.
            playFocus.onPlay()
        } else {
            playFocus.onStop()
            // The locks stay a while yet (see [LockKeeper]).
            setNoisyReceiver(false)
            // Keep focus through a transient loss so we can resume; give it up otherwise.
            if (!policy.resumeOnGain) focus.abandon()
        }
    }

    private fun setNoisyReceiver(on: Boolean) {
        if (on == noisyRegistered) return
        noisyRegistered = on
        if (on) {
            ContextCompat.registerReceiver(
                this,
                noisy,
                IntentFilter(AudioManager.ACTION_AUDIO_BECOMING_NOISY),
                ContextCompat.RECEIVER_NOT_EXPORTED,
            )
        } else {
            unregisterReceiver(noisy)
        }
    }

    private fun updateRoute() {
        val route = describeRoute(this, audio)
        host.localPlayer.setOutputRoute(route)
        host.offlinePlayer?.setOutputRoute(route)
        // Per-output EQ (Wave 2 §5.3): the profile bound to this output, if any, applies.
        val output = audioOutputOf(bestOutput(outputDevices(audio)))
        host.localPlayer.setOutput(output)
        host.offlinePlayer?.setOutput(output)
    }

    private fun currentUsbDac(): OutputDevice? = pickUsbDac(outputDevices(audio))

    /** A device came or went: update the route, and renegotiate if the USB DAC changed. */
    private fun devicesChanged() {
        updateRoute()
        // Another output has its own volume (and may keep it itself, as Bluetooth can).
        volumeFollower?.refresh()
        val now = currentUsbDac()
        if (now?.id != usbDac?.id) {
            usbDac = now
            host.localPlayer.renegotiateOutput()
            host.offlinePlayer?.renegotiateOutput()
        }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaLibrarySession? =
        session.takeIf { ready && access(controllerInfo) != CallerAccess.REJECT }

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Keep playing in the background; otherwise there is no reason to stay.
        if (!playing) stopSelf()
    }

    override fun onDestroy() {
        if (!ready) {
            scope.cancel()
            super.onDestroy()
            return
        }
        audio.unregisterAudioDeviceCallback(devices)
        volumeFollower?.setActive(false)
        // The engine clears any mixer attributes through the negotiator that set them.
        host.localPlayer.setOutputNegotiator(null)
        host.offlinePlayer?.setOutputNegotiator(null)
        playFocus.onStop()
        setNoisyReceiver(false)
        focus.abandon()
        locks.hold(false)
        session?.release()
        lmsPlayer?.release()
        offlineSessionPlayer?.release()
        session = null
        scope.cancel()
        super.onDestroy()
    }
}

/** Audio modes in which a call is ringing or on: a focus refusal then is a real conflict. */
private val CALL_MODES = setOf(
    AudioManager.MODE_RINGTONE,
    AudioManager.MODE_IN_CALL,
    AudioManager.MODE_IN_COMMUNICATION,
)

/**
 * When to hold the wake and Wi-Fi locks: while this phone plays or its engine starts a
 * track, and for [graceMs] after. Letting go the moment playback stops let the phone
 * sleep between LMS's flush and the next track's start (a track change, or a queue
 * replaced from another device), and with the screen off nothing ran until it woke.
 * Pure: unit-tested on the JVM.
 */
class LockKeeper(private val graceMs: Long = GRACE_MS) {
    /** When playback was last active (null: not since the player was switched on). */
    private var activeAt: Long? = null

    /**
     * Whether to hold the locks at [now] (a monotonic clock, ms), and in how many ms to
     * ask again (null: only when the inputs change). [enabled]: this phone is a player (or
     * has downloads loaded); [active]: playing, or about to.
     */
    fun decide(enabled: Boolean, active: Boolean, now: Long): Pair<Boolean, Long?> {
        if (!enabled) {
            activeAt = null
            return false to null
        }
        if (active) {
            activeAt = now
            return true to null
        }
        val since = activeAt ?: return false to null
        val left = graceMs - (now - since).coerceAtLeast(0)
        if (left <= 0) {
            activeAt = null
            return false to null
        }
        return true to left
    }

    companion object {
        /** How long the locks stay after playback stops (a slow queue load included). */
        const val GRACE_MS = 90_000L
    }
}

/** A partial wake lock and a Wi-Fi lock, held while this phone plays. */
class PlaybackLocks(context: Context) {
    private val wake = context.getSystemService(PowerManager::class.java)
        .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "LyrionAnywhere:playback")
        .apply { setReferenceCounted(false) }

    // High-performance mode keeps Wi-Fi awake on Android 10-13; on 14+ the platform keeps
    // a foreground media service's network up by itself (the mode is a no-op there).
    @Suppress("DEPRECATION")
    private val wifi = context.applicationContext.getSystemService(WifiManager::class.java)
        .createWifiLock(WifiManager.WIFI_MODE_FULL_HIGH_PERF, "LyrionAnywhere:playback")
        .apply { setReferenceCounted(false) }

    /** The locks are held (as last asked). */
    var held = false
        private set

    fun hold(on: Boolean) {
        held = on
        if (on) {
            // Renewed on every track change; the timeout is only a safety net.
            wake.acquire(WAKE_TIMEOUT_MS)
            if (!wifi.isHeld) wifi.acquire()
        } else {
            if (wake.isHeld) wake.release()
            if (wifi.isHeld) wifi.release()
        }
    }

    private companion object {
        const val WAKE_TIMEOUT_MS = 60 * 60 * 1000L
    }
}

/** The output devices, as plain values. */
fun outputDevices(audio: AudioManager): List<OutputDevice> =
    audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).map {
        OutputDevice(it.id, it.type, it.productName?.toString() ?: "")
    }

/**
 * What Android says about the output, for the signal path (best effort).
 *
 * Bluetooth shows the route and the device's name only, not the codec (LDAC, aptX, AAC,
 * SBC): `BluetoothA2dp.getCodecStatus` is a system API (not in the public SDK), and the
 * codec-change broadcast isn't public either, so an ordinary app can't read it.
 */
fun describeRoute(context: Context, audio: AudioManager): OutputRoute {
    val best = bestOutput(outputDevices(audio))
    val kind = best?.type?.let(::routeKind)
    val label = when (kind) {
        RouteKind.USB -> R.string.route_usb
        RouteKind.WIRED -> R.string.route_wired
        RouteKind.BLUETOOTH -> R.string.route_bluetooth
        else -> R.string.route_speaker
    }
    // The mixer rate Android reports is the primary output's: meaningful for the speaker
    // and wired headphones, not for USB or Bluetooth devices (unknown there).
    val primary = best == null || kind in setOf(RouteKind.SPEAKER, RouteKind.WIRED)
    val mixer = if (primary) {
        audio.getProperty(AudioManager.PROPERTY_OUTPUT_SAMPLE_RATE)?.toIntOrNull() ?: 0
    } else {
        0
    }
    return OutputRoute(routeLabel(context.getString(label), kind, best?.name), mixer.coerceAtLeast(0).toUInt())
}

/** The output media most likely plays on (see [routePriority]). */
fun bestOutput(devices: List<OutputDevice>): OutputDevice? = devices.maxByOrNull { routePriority(it.type) }

/**
 * The output as per-output EQ knows it (Wave 2 §5.3): the built-in speaker, wired
 * headphones, a USB DAC by its product name, or a Bluetooth device by its name. Anything
 * else counts as the speaker.
 */
fun audioOutputOf(device: OutputDevice?): AudioOutput = when (device?.type?.let(::routeKind)) {
    RouteKind.USB -> AudioOutput.of(OutputClass.USB, device.name)
    RouteKind.WIRED -> AudioOutput.of(OutputClass.WIRED, null)
    RouteKind.BLUETOOTH -> AudioOutput.of(OutputClass.BLUETOOTH, device.name)
    else -> AudioOutput.of(OutputClass.SPEAKER, null)
}

/** "USB audio · <DAC>" or "Bluetooth · <headphones>": the route, with the device's own name
 *  where it has a meaningful one. */
fun routeLabel(route: String, kind: RouteKind?, deviceName: String?): String {
    val name = deviceName?.filter { !it.isISOControl() }?.trim()?.take(48).orEmpty()
    val named = kind == RouteKind.USB || kind == RouteKind.BLUETOOTH
    return if (named && name.isNotEmpty()) "$route · $name" else route
}

enum class RouteKind { USB, WIRED, BLUETOOTH, SPEAKER, OTHER }

fun routeKind(type: Int): RouteKind = when (type) {
    AudioDeviceInfo.TYPE_USB_DEVICE, AudioDeviceInfo.TYPE_USB_HEADSET, AudioDeviceInfo.TYPE_USB_ACCESSORY -> RouteKind.USB
    AudioDeviceInfo.TYPE_WIRED_HEADPHONES, AudioDeviceInfo.TYPE_WIRED_HEADSET, AudioDeviceInfo.TYPE_LINE_ANALOG -> RouteKind.WIRED
    AudioDeviceInfo.TYPE_BLUETOOTH_A2DP, AudioDeviceInfo.TYPE_HEARING_AID, 26 /* BLE headset */, 27 /* BLE speaker */ ->
        RouteKind.BLUETOOTH
    AudioDeviceInfo.TYPE_BUILTIN_SPEAKER -> RouteKind.SPEAKER
    else -> RouteKind.OTHER
}

/** Android routes media to the most recently connected external device; approximate that. */
fun routePriority(type: Int): Int = when (routeKind(type)) {
    RouteKind.USB -> 4
    RouteKind.WIRED -> 3
    RouteKind.BLUETOOTH -> 2
    RouteKind.SPEAKER -> 1
    RouteKind.OTHER -> 0
}
