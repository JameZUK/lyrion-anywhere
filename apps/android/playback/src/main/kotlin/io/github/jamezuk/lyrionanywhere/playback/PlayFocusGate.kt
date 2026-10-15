package io.github.jamezuk.lyrionanywhere.playback

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** What became of audio focus for one start of playback. */
enum class FocusDecision {
    /** Focus is held: play. */
    GRANTED,

    /** A real conflict (a call): silence and pause LMS. */
    PAUSE,

    /** Refused only because the app is in the background: keep playing, and note it. */
    PLAY_WITHOUT_FOCUS,
}

/**
 * Audio focus when this phone starts playing because LMS says so (often a resume pressed on
 * another controller, with the phone's screen off).
 *
 * Android 15+ refuses focus outright to an app that is neither on screen nor running a
 * foreground service. Answering that refusal with a pause paused the whole sync group a
 * moment after another device resumed it. So, once per start of playback:
 * 1. the playback service is brought to the foreground first ([bringToForeground]), then
 *    focus is asked for;
 * 2. refused while the app is on screen or a call is on: a real conflict, pause at once;
 * 3. refused otherwise: asked again over about 2 s ([retryDelaysMs]) while the service
 *    reaches the foreground; still refused, a refusal the background rule explains (see
 *    [FocusRefusals.classify]) keeps playing, and any other pauses.
 *
 * A start of playback lasts from [onPlay] to [onStop]: repeated reports in between make no
 * second decision, and a pause made for a conflict isn't repeated within
 * [PAUSE_COOLDOWN_MS] (LMS's status can bounce back to playing for a moment). Not
 * thread-safe: used from the main thread.
 */
class PlayFocusGate(
    private val scope: CoroutineScope,
    private val bringToForeground: () -> Unit,
    private val request: () -> Boolean,
    private val onScreen: () -> Boolean,
    private val serviceForeground: () -> Boolean,
    private val inCall: () -> Boolean,
    private val clock: () -> Long,
    private val retryDelaysMs: List<Long> = RETRY_DELAYS_MS,
    private val onDecision: (FocusDecision) -> Unit,
) {
    /** This start of playback has been (or is being) decided. */
    private var started = false
    private var retrying: Job? = null
    private var lastPauseAt: Long? = null

    /** LMS (or the downloads) says play. */
    fun onPlay() {
        if (started) return
        started = true
        if (attempt()) return
        retrying = scope.launch {
            for (wait in retryDelaysMs) {
                delay(wait)
                if (attempt()) return@launch
            }
            val refusal = FocusRefusals.classify(onScreen(), serviceForeground(), inCall())
            decide(if (refusal == FocusRefusal.BACKGROUND) FocusDecision.PLAY_WITHOUT_FOCUS else FocusDecision.PAUSE)
        }
    }

    /** Playback stopped or paused: the next play is a new start. */
    fun onStop() {
        started = false
        retrying?.cancel()
        retrying = null
    }

    /** One try: true when it settled the decision. */
    private fun attempt(): Boolean {
        if (!serviceForeground()) bringToForeground()
        if (request()) {
            decide(FocusDecision.GRANTED)
            return true
        }
        if (FocusRefusals.conclusive(onScreen(), inCall())) {
            decide(FocusDecision.PAUSE)
            return true
        }
        return false
    }

    private fun decide(decision: FocusDecision) {
        retrying = null
        if (decision == FocusDecision.PAUSE) {
            val now = clock()
            val last = lastPauseAt
            // Already paused for this a moment ago: one pause, not one per status report.
            if (last != null && now - last < PAUSE_COOLDOWN_MS) return
            lastPauseAt = now
        }
        onDecision(decision)
    }

    companion object {
        /** The waits between tries after a refusal: about 2 s in all. */
        val RETRY_DELAYS_MS = listOf(250L, 500L, 500L, 750L)

        /** A pause for a refusal isn't sent again within this long. */
        const val PAUSE_COOLDOWN_MS = 3_000L
    }
}
