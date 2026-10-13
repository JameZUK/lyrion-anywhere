package io.github.jamezuk.lyrionanywhere.playback

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.database.ContentObserver
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.core.content.ContextCompat
import io.github.jamezuk.lyrionanywhere.data.MAX_LOUDNESS_REFERENCE_DB

/**
 * Loudness-compensated volume while this phone is the player follows the phone's media
 * volume (SPEC.md, Sound): the attenuation the system applies at the current index on the
 * current output, in dB. Plain Kotlin, so the rules are tested on the JVM.
 *
 * The phone's index says nothing about what is heard when the output's volume is fixed, or
 * when a Bluetooth device uses absolute volume (the headset turns itself down; the phone's
 * curve for it is flat at 0 dB). Then the answer is null, and loudness follows the player's
 * own level (LMS's) alone.
 */
object LoudnessReference {
    /**
     * Android's default media volume curve (AOSP `DEFAULT_MEDIA_VOLUME_CURVE`): index as a
     * percentage of the range, and the gain there in dB. Used only when the system can't say.
     */
    private val CURVE = listOf(1.0 to -58.0, 20.0 to -40.0, 60.0 to -17.0, 100.0 to 0.0)

    /** Below this (dB) at the lowest step, a Bluetooth curve isn't flat: the phone attenuates. */
    const val FLAT_DB = -0.5

    /** Bluetooth outputs (A2DP, LE audio headset, speaker and broadcast, hearing aids). */
    fun isBluetooth(type: Int): Boolean = type in BLUETOOTH

    private val BLUETOOTH = setOf(
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
        AudioDeviceInfo.TYPE_HEARING_AID,
        26, // TYPE_BLE_HEADSET
        27, // TYPE_BLE_SPEAKER
        30, // TYPE_BLE_BROADCAST
    )

    /** The default curve's attenuation at [index] of [max] (dB, 0 = full); muted is the most. */
    fun fallbackDb(index: Int, max: Int): Double {
        if (max <= 0) return 0.0
        val i = index.coerceIn(0, max)
        if (i == 0) return MAX_LOUDNESS_REFERENCE_DB
        val pct = 100.0 * i / max
        if (pct <= CURVE.first().first) return -CURVE.first().second
        for (k in 1 until CURVE.size) {
            val (x1, y1) = CURVE[k]
            if (pct <= x1) {
                val (x0, y0) = CURVE[k - 1]
                return -(y0 + (y1 - y0) * (pct - x0) / (x1 - x0))
            }
        }
        return 0.0
    }

    /**
     * The attenuation loudness should follow (dB, 0 = full; muted is the most), or null when
     * the phone's volume doesn't set the level heard.
     *
     * @param index the media stream's index now, of [max].
     * @param deviceType the output media plays on (null: not known, taken as the speaker).
     * @param fixed `AudioManager.isVolumeFixed`: the phone has no volume of its own.
     * @param streamDb the system's gain (dB) at an index on that device
     *   (`getStreamVolumeDb`), or null where it can't say.
     */
    fun attenuationDb(index: Int, max: Int, deviceType: Int?, fixed: Boolean, streamDb: (Int) -> Float?): Double? {
        if (fixed || max <= 0) return null
        val i = index.coerceIn(0, max)
        val bluetooth = deviceType != null && isBluetooth(deviceType)
        val lowest = streamDb(1)?.toDouble()?.takeUnless { it.isNaN() }
        if (bluetooth) {
            // Absolute volume: the phone's own curve is flat (0 dB at every step), the headset
            // turns itself down. Not knowing the curve, assume the same.
            if (lowest == null || lowest >= FLAT_DB) return null
        }
        if (i == 0) return MAX_LOUDNESS_REFERENCE_DB
        val db = streamDb(i)?.toDouble()?.takeUnless { it.isNaN() } ?: return fallbackDb(i, max)
        return (-db).coerceIn(0.0, MAX_LOUDNESS_REFERENCE_DB)
    }
}

/**
 * Watches the phone's media volume and output while this phone plays, and hands the
 * attenuation loudness compensation should follow to [onChange] (once per change). Listens
 * to the settings (where the system keeps each stream's volume) and the volume broadcast,
 * neither exported; [refresh] is called by the owner when the output changes too.
 */
class PhoneVolumeFollower(
    private val context: Context,
    private val audio: AudioManager,
    private val onChange: (Double?) -> Unit,
) {
    private val handler = Handler(Looper.getMainLooper())
    private var active = false
    private var sent = false
    private var last: Double? = null

    private val observer = object : ContentObserver(handler) {
        override fun onChange(selfChange: Boolean) = refresh()
    }

    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(c: Context, intent: Intent) = refresh()
    }

    /** Follow (true) or stop following the volume. */
    fun setActive(on: Boolean) {
        if (on == active) return
        active = on
        if (on) {
            runCatching { context.contentResolver.registerContentObserver(Settings.System.CONTENT_URI, true, observer) }
            runCatching {
                ContextCompat.registerReceiver(context, receiver, IntentFilter(VOLUME_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
            }
            refresh()
        } else {
            runCatching { context.contentResolver.unregisterContentObserver(observer) }
            runCatching { context.unregisterReceiver(receiver) }
        }
    }

    /** Read the volume and output now, and pass on a change. */
    fun refresh() {
        if (!active) return
        val db = runCatching { read() }.getOrNull()
        if (sent && db == last) return
        sent = true
        last = db
        onChange(db)
    }

    private fun read(): Double? {
        val stream = AudioManager.STREAM_MUSIC
        val type = mediaDevice()
        return LoudnessReference.attenuationDb(
            index = audio.getStreamVolume(stream),
            max = audio.getStreamMaxVolume(stream),
            deviceType = type,
            fixed = audio.isVolumeFixed,
        ) { i -> runCatching { audio.getStreamVolumeDb(stream, i, type ?: AudioDeviceInfo.TYPE_BUILTIN_SPEAKER) }.getOrNull() }
    }

    /** The output media plays on: the system's own answer (Android 13+), else the best guess. */
    private fun mediaDevice(): Int? {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            val attrs = AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).build()
            runCatching { audio.getAudioDevicesForAttributes(attrs).firstOrNull()?.type }.getOrNull()?.let { return it }
        }
        return bestOutput(outputDevices(audio))?.type
    }

    private companion object {
        /** The system's volume-changed broadcast (sent by the system only). */
        const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
    }
}
