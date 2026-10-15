package io.github.jamezuk.lyrionanywhere.playback

import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioManager

/** Audio focus changes, as Android reports them. */
enum class FocusChange { GAIN, LOSS, LOSS_TRANSIENT, LOSS_TRANSIENT_CAN_DUCK }

/** What to do about a focus change. */
sealed interface FocusAction {
    /** Silence at once and pause LMS. */
    data object Pause : FocusAction

    /** Ask LMS to play again (after a transient loss). */
    data object Resume : FocusAction

    data class Duck(val on: Boolean) : FocusAction
}

/**
 * The focus rules, kept apart from Android so they are unit-tested:
 * - permanent loss (another music app): pause, and don't resume by ourselves;
 * - transient loss (a call, navigation prompt): pause, and resume when focus returns if
 *   we were playing;
 * - "may duck": lower the volume (music keeps going), restore on gain.
 */
class AudioFocusPolicy {
    var resumeOnGain = false
        private set
    private var ducked = false

    fun onChange(change: FocusChange, playing: Boolean): List<FocusAction> = buildList {
        when (change) {
            FocusChange.GAIN -> {
                if (ducked) add(FocusAction.Duck(false))
                if (resumeOnGain) add(FocusAction.Resume)
                ducked = false
                resumeOnGain = false
            }
            FocusChange.LOSS -> {
                if (ducked) add(FocusAction.Duck(false))
                ducked = false
                resumeOnGain = false
                if (playing) add(FocusAction.Pause)
            }
            FocusChange.LOSS_TRANSIENT -> {
                if (playing) {
                    resumeOnGain = true
                    add(FocusAction.Pause)
                }
            }
            FocusChange.LOSS_TRANSIENT_CAN_DUCK -> {
                if (!ducked) {
                    ducked = true
                    add(FocusAction.Duck(true))
                }
            }
        }
    }

    /** The user paused (or playback stopped): never resume by ourselves. */
    fun userPaused() {
        resumeOnGain = false
    }
}

/** Why audio focus was refused, as far as the app can tell. */
enum class FocusRefusal {
    /** A real conflict (a call, or another app holding focus): don't play over it. */
    CONFLICT,

    /** Android 15+'s rule: no focus for an app neither on screen nor in a foreground service. */
    BACKGROUND,
}

/** How to read a refusal of audio focus. Pure: unit-tested on the JVM. */
object FocusRefusals {
    /**
     * A refusal is explained by the background rule only while the app is wholly in the
     * background: not on screen, the playback service not in the foreground, and no call.
     * Anything else is a real conflict.
     */
    fun classify(onScreen: Boolean, serviceForeground: Boolean, inCall: Boolean): FocusRefusal =
        if (onScreen || serviceForeground || inCall) FocusRefusal.CONFLICT else FocusRefusal.BACKGROUND

    /** A refusal no retry can change: the app is on screen (the rule doesn't apply) or a call is on. */
    fun conclusive(onScreen: Boolean, inCall: Boolean): Boolean = onScreen || inCall
}

/** Requests and abandons audio focus for music; reports changes to [onChange]. */
class AudioFocusHandler(
    private val audio: AudioManager,
    private val onChange: (FocusChange) -> Unit,
) {
    private var held = false

    private val request = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_MEDIA)
                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                .build(),
        )
        // We duck ourselves (in the engine), so Android doesn't have to.
        .setWillPauseWhenDucked(false)
        .setOnAudioFocusChangeListener { change ->
            when (change) {
                AudioManager.AUDIOFOCUS_GAIN -> onChange(FocusChange.GAIN)
                AudioManager.AUDIOFOCUS_LOSS -> {
                    held = false
                    onChange(FocusChange.LOSS)
                }
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT -> onChange(FocusChange.LOSS_TRANSIENT)
                AudioManager.AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK -> onChange(FocusChange.LOSS_TRANSIENT_CAN_DUCK)
            }
        }
        .build()

    /** Returns false if focus was refused (e.g. during a call). */
    fun request(): Boolean {
        if (held) return true
        held = audio.requestAudioFocus(request) == AudioManager.AUDIOFOCUS_REQUEST_GRANTED
        return held
    }

    fun abandon() {
        if (!held) return
        held = false
        audio.abandonAudioFocusRequest(request)
    }
}
