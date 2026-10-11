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
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * The media session for "Play on this phone": notification, lock screen, Bluetooth and
 * headset buttons, audio focus, "becoming noisy", and wake and Wi-Fi locks while playing.
 *
 * Media3 makes this a foreground service (type `mediaPlayback`) while playback is on.
 * Not exported: only this app binds to it; the system's media controls use the platform
 * session token Media3 publishes.
 */
@OptIn(UnstableApi::class)
class PlaybackService : MediaSessionService() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var session: MediaSession? = null
    private lateinit var controller: LocalPlaybackController
    private lateinit var host: PlaybackHost
    private lateinit var focus: AudioFocusHandler
    private lateinit var locks: PlaybackLocks
    private lateinit var audio: AudioManager
    private val policy = AudioFocusPolicy()
    private var noisyRegistered = false
    private var playing = false

    private val noisy = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            if (intent.action == AudioManager.ACTION_AUDIO_BECOMING_NOISY) {
                policy.userPaused()
                scope.launch { controller.becomingNoisy() }
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
        focus = AudioFocusHandler(audio) { change ->
            for (action in policy.onChange(change, playing)) {
                when (action) {
                    FocusAction.Pause -> {
                        host.localPlayer.setHold(true)
                        scope.launch { controller.pause() }
                    }
                    FocusAction.Resume -> scope.launch { controller.play() }
                    is FocusAction.Duck -> host.localPlayer.setDuck(action.on)
                }
            }
        }
        val player = LmsSessionPlayer(Looper.getMainLooper(), controller.snapshot, controller, scope)
        val launch = packageManager.getLaunchIntentForPackage(packageName)?.let {
            PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
        session = MediaSession.Builder(this, player)
            .apply { launch?.let { setSessionActivity(it) } }
            .build()
        // Bit-perfect USB output: the engine asks this before opening its output.
        host.localPlayer.setOutputNegotiator(
            UsbDacNegotiator(audio) { host.localPlayer.state.value.sound.bitPerfectUsb },
        )
        usbDac = currentUsbDac()
        audio.registerAudioDeviceCallback(devices, Handler(Looper.getMainLooper()))
        updateRoute()

        scope.launch {
            controller.snapshot
                .map { mapSnapshot(it).playWhenReady to it.local.enabled }
                .distinctUntilChanged()
                .collect { (wantsToPlay, enabled) -> onPlayingChanged(wantsToPlay, enabled) }
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
            // "Play on this phone" was switched off: nothing left to do here.
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
        host.localPlayer.setOutputRoute(describeRoute(this, audio))
    }

    private fun currentUsbDac(): OutputDevice? = pickUsbDac(outputDevices(audio))

    /** A device came or went: update the route, and renegotiate if the USB DAC changed. */
    private fun devicesChanged() {
        updateRoute()
        val now = currentUsbDac()
        if (now?.id != usbDac?.id) {
            usbDac = now
            host.localPlayer.renegotiateOutput()
        }
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? = session

    override fun onTaskRemoved(rootIntent: Intent?) {
        // Keep playing in the background; otherwise there is no reason to stay.
        if (!playing) stopSelf()
    }

    override fun onDestroy() {
        audio.unregisterAudioDeviceCallback(devices)
        // The engine clears any mixer attributes through the negotiator that set them.
        host.localPlayer.setOutputNegotiator(null)
        setNoisyReceiver(false)
        focus.abandon()
        locks.hold(false)
        session?.run {
            player.release()
            release()
        }
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
