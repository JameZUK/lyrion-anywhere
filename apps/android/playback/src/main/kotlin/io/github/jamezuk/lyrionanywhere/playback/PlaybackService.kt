package io.github.jamezuk.lyrionanywhere.playback

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
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
import io.github.jamezuk.lyrionanywhere.playback.auto.AutoCatalogue
import io.github.jamezuk.lyrionanywhere.playback.auto.CallerAccess
import io.github.jamezuk.lyrionanywhere.playback.auto.CallerFactsReader
import io.github.jamezuk.lyrionanywhere.playback.auto.CallerPolicy
import io.github.jamezuk.lyrionanywhere.playback.auto.LibraryCallback
import io.github.jamezuk.lyrionanywhere.playback.auto.ResourceLabels
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
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
                CallerPolicy.decide(callers.read(controller.packageName, controller.uid, controller.isTrusted))
            }
        }
    }
    private lateinit var controller: LocalPlaybackController
    private lateinit var host: PlaybackHost
    private lateinit var focus: AudioFocusHandler
    private lateinit var locks: PlaybackLocks
    private lateinit var audio: AudioManager
    private val policy = AudioFocusPolicy()
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
        catalogue = host.autoSource?.let { AutoCatalogue(it, ResourceLabels(this)) }
        val requests = catalogue?.let { c -> PlayRequests { id, query -> c.play(id, query) } }
        audio = getSystemService(AudioManager::class.java)
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
                    FocusAction.Pause -> if (offline != null && offlineActive) {
                        offline.setHold(true)
                        offline.pause()
                    } else {
                        host.localPlayer.setHold(true)
                        scope.launch { controller.pause() }
                    }
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
        val callback = LibraryCallback(::access, { catalogue }, scope)
        session = MediaLibrarySession.Builder(this, if (offlineActive) offlineSessionPlayer ?: player else player, callback)
            .apply { launch?.let { setSessionActivity(it) } }
            .build()
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
            // Keep the wake lock's timeout fresh while playing (each track change).
            controller.snapshot.map { it.status?.current?.id }.distinctUntilChanged().collect {
                if (playing) locks.hold(true)
            }
        }
    }

    private fun onPlayingChanged(wantsToPlay: Boolean, enabled: Boolean) {
        if (!enabled) {
            // "Play on this phone" was switched off and no downloads are loaded.
            playing = false
            locks.hold(false)
            setNoisyReceiver(false)
            focus.abandon()
            stopSelf()
            return
        }
        playing = wantsToPlay
        if (wantsToPlay) {
            if (!focus.request()) {
                // Refused (a call is in progress): don't play over it.
                host.localPlayer.setHold(true)
                scope.launch { controller.pause() }
                return
            }
            setNoisyReceiver(true)
            locks.hold(true)
        } else {
            setNoisyReceiver(false)
            locks.hold(false)
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
    }

    private fun currentUsbDac(): OutputDevice? = pickUsbDac(outputDevices(audio))

    /** A device came or went: update the route, and renegotiate if the USB DAC changed. */
    private fun devicesChanged() {
        updateRoute()
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
        // The engine clears any mixer attributes through the negotiator that set them.
        host.localPlayer.setOutputNegotiator(null)
        host.offlinePlayer?.setOutputNegotiator(null)
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

    fun hold(on: Boolean) {
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
    val best = outputDevices(audio).maxByOrNull { routePriority(it.type) }
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
