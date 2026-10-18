package io.github.jamezuk.lyrionanywhere.ui.player

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import androidx.core.content.ContextCompat
import android.database.ContentObserver
import android.media.AudioManager
import android.os.Handler
import android.os.Looper
import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import kotlin.math.roundToInt

/**
 * This phone's media volume (Android's music stream), for Now playing's volume control
 * while this phone is the player. Hardware keys change the same volume.
 */
interface PhoneVolume {
    /** The highest step (at least 1). */
    val max: Int

    /** The output has a fixed volume (the phone can't change it). */
    val fixed: Boolean

    /** The step now, 0–[max]. */
    fun index(): Int

    /** Set the step (held to the range). */
    fun setIndex(index: Int)

    /** Call [onChange] when the volume changes (by keys, another app, here); returns a stop. */
    fun observe(onChange: () -> Unit): () -> Unit
}

/** Between a step and a percentage, for the popover's 0–100 % readout. */
object PhoneVolumeMath {
    fun percent(index: Int, max: Int): Int =
        if (max <= 0) 0 else (100.0 * index.coerceIn(0, max) / max).roundToInt()

    /** The step nearest [percent] (held to 0–100). */
    fun index(percent: Float, max: Int): Int =
        if (max <= 0 || percent.isNaN()) 0 else (percent.coerceIn(0f, 100f) / 100f * max).roundToInt().coerceIn(0, max)
}

/**
 * The real thing: `AudioManager`'s music stream. Changes made here don't raise the system's
 * own volume panel (the popover shows the level); the hardware keys still do.
 */
class SystemPhoneVolume(private val context: Context) : PhoneVolume {
    private val audio: AudioManager? = context.getSystemService(AudioManager::class.java)

    override val max: Int get() = (audio?.getStreamMaxVolume(AudioManager.STREAM_MUSIC) ?: 0).coerceAtLeast(1)

    override val fixed: Boolean get() = audio?.isVolumeFixed ?: true

    override fun index(): Int = audio?.getStreamVolume(AudioManager.STREAM_MUSIC)?.coerceIn(0, max) ?: 0

    override fun setIndex(index: Int) {
        val a = audio ?: return
        if (fixed) return
        try {
            a.setStreamVolume(AudioManager.STREAM_MUSIC, index.coerceIn(0, max), 0)
        } catch (_: SecurityException) {
            // Do Not Disturb can refuse a change; the slider shows the real level again.
        }
    }

    /**
     * Follows the music stream's volume. Android writes the level to its settings (which the
     * observer sees) about half a second after a change, and not on every device, so the
     * system's own volume broadcast is followed too: it comes at once, for the hardware keys
     * as for any other change. Only its arrival is used, never its contents.
     */
    override fun observe(onChange: () -> Unit): () -> Unit {
        val app = context.applicationContext
        val observer = object : ContentObserver(Handler(Looper.getMainLooper())) {
            override fun onChange(selfChange: Boolean) = onChange()
        }
        val resolver = app.contentResolver
        val observing = runCatching { resolver.registerContentObserver(Settings.System.CONTENT_URI, true, observer) }.isSuccess
        val receiver = object : BroadcastReceiver() {
            override fun onReceive(context: Context?, intent: Intent?) = onChange()
        }
        // Not exported: only the system (and this app) can send it here.
        val receiving = runCatching {
            ContextCompat.registerReceiver(app, receiver, IntentFilter(VOLUME_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
        }.isSuccess
        return {
            if (observing) runCatching { resolver.unregisterContentObserver(observer) }
            if (receiving) runCatching { app.unregisterReceiver(receiver) }
        }
    }

    companion object {
        /** AudioManager's (hidden) VOLUME_CHANGED_ACTION, a protected system broadcast. */
        const val VOLUME_CHANGED = "android.media.VOLUME_CHANGED_ACTION"
    }
}

/** The phone's volume for this screen ([given] in tests), and its step, kept up to date. */
@Composable
fun rememberPhoneVolume(given: PhoneVolume? = null): PhoneVolume {
    val context = LocalContext.current
    return given ?: remember(context) { SystemPhoneVolume(context) }
}

/** [volume]'s step now, following changes while shown. */
@Composable
fun phoneVolumeIndex(volume: PhoneVolume): Int {
    var index by remember(volume) { mutableIntStateOf(volume.index()) }
    DisposableEffect(volume) {
        index = volume.index()
        val stop = volume.observe { index = volume.index() }
        onDispose { stop() }
    }
    return index
}
