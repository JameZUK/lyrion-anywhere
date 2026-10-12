package io.github.jamezuk.lyrionanywhere.ui.player

import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.SleepEnd
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.MAX_SLEEP_SECONDS
import io.github.jamezuk.lyrionanywhere.data.SLEEP_FADE_MAX
import kotlin.math.abs
import kotlin.math.ceil

/** What the sleep timer is set to: a length, or the end of the track, album or queue. */
sealed interface SleepChoice {
    data class Minutes(val minutes: Int) : SleepChoice

    data class End(val end: SleepEnd) : SleepChoice
}

/** Which players a timer is for. */
enum class SleepScope { PLAYER, GROUP, ALL }

/** A timer set to an end of the queue, kept in step with it. */
data class SleepArm(
    val end: SleepEnd,
    /** The players it was set on (the selected one first). */
    val players: List<String>,
    /** When LMS should stop them, and when it was last set ([PlayerViewModel]'s clock, ms). */
    val endsAtMs: Long,
    val setAtMs: Long,
)

/**
 * The sleep timer's rules (Wave 1 §1), as the web app (`sleep.ts`) and the Rust core
 * (`mobile/src/sleep.rs`, which iOS uses and the engine's fade is built on) have them,
 * kept in Kotlin like [QueueEdit] so the view model's tests run on the JVM. The tests use the
 * same vectors on every platform.
 */
object SleepTimer {
    /** The fixed lengths offered, in minutes. */
    val MINUTES = listOf(15, 30, 45, 60, 90)
    const val MAX_CUSTOM_MINUTES = 24 * 60

    /** Fade-out lengths offered in Settings (seconds; 0 is off). */
    val FADE_CHOICES = listOf(0, 10, 30, 60, 90, 120)

    /** Re-arm when the end has moved by more than this. */
    const val REARM_TOLERANCE_MS = 5_000L

    /** LMS has had this long to report a timer just set before its absence means "cancelled". */
    const val CONFIRM_MS = 10_000L

    private fun positive(v: Double?): Double? = v?.takeIf { it.isFinite() && it > 0 }

    /**
     * Seconds until the end of the current track, the current album (the run of tracks from
     * the same album that follows it) or the queue, rounded up. [upcoming] is the queue after
     * the current track, in playing order. Null when it can't be known: nothing playing, a
     * stream with no length, a track in range without one, (for the queue) a queue longer
     * than what was read, or an end more than a day away.
     */
    fun remaining(end: SleepEnd, status: PlayerStatus?, upcoming: List<Track> = emptyList()): Int? {
        val cur = status?.current ?: return null
        val length = positive(status.durationSecs) ?: positive(cur.durationSecs) ?: return null
        val elapsed = status.elapsedSecs.takeIf { it.isFinite() }?.coerceAtLeast(0.0) ?: 0.0
        var left = (length - elapsed).coerceAtLeast(0.0)
        val album = cur.albumId?.takeIf { it > 0 }
        when (end) {
            SleepEnd.TRACK -> return withinADay(left)
            SleepEnd.ALBUM -> if (album == null) return withinADay(left)
            SleepEnd.QUEUE -> {
                val index = status.queueIndex?.toLong() ?: 0L
                val after = (status.queueLength.toLong() - index - 1).coerceAtLeast(0)
                if (upcoming.size < after) return null
            }
        }
        for (t in upcoming) {
            if (end == SleepEnd.ALBUM && t.albumId?.takeIf { it > 0 } != album) break
            left += positive(t.durationSecs) ?: return null
        }
        return withinADay(left)
    }

    private fun withinADay(s: Double): Int? {
        if (!s.isFinite()) return null
        val v = ceil(s.coerceAtLeast(0.0))
        return if (v <= MAX_SLEEP_SECONDS) v.toInt() else null
    }

    /** Seconds for a choice (null when "end of …" can't be worked out, or a bad length). */
    fun seconds(choice: SleepChoice, status: PlayerStatus?, upcoming: List<Track> = emptyList()): Int? = when (choice) {
        is SleepChoice.Minutes -> customMinutes(choice.minutes)?.let { it * 60 }
        is SleepChoice.End -> remaining(choice.end, status, upcoming)
    }

    /** A custom length in whole minutes, 1 to a day, or null. */
    fun customMinutes(v: Int?): Int? = v?.takeIf { it in 1..MAX_CUSTOM_MINUTES }

    /** What was typed as a custom length, checked. */
    fun customMinutes(text: String): Int? {
        val t = text.trim()
        if (t.isEmpty() || t.length > 4 || !t.all { it in '0'..'9' }) return null
        return customMinutes(t.toInt())
    }

    /**
     * The seconds to set an armed timer to again, if its end has moved (a seek, skip, pause
     * or queue edit) by more than the tolerance; otherwise null. Never 0 (that would cancel).
     */
    fun rearm(arm: SleepArm, remaining: Int?, nowMs: Long): Int? {
        val r = remaining ?: return null
        val wanted = nowMs + r * 1000L
        return if (abs(wanted - arm.endsAtMs) > REARM_TOLERANCE_MS) r.coerceAtLeast(1) else null
    }

    /** Whether an armed timer is over: LMS no longer reports it once it has had time to. */
    fun over(arm: SleepArm, sleepSecs: Double?, nowMs: Long): Boolean {
        if (positive(sleepSecs) != null) return false
        return nowMs - arm.setAtMs > CONFIRM_MS || nowMs > arm.endsAtMs + CONFIRM_MS
    }

    /** Seconds left on the player's timer as LMS reports it (rounded up), or null. */
    fun left(status: PlayerStatus?): Int? = positive(status?.sleepSecs)?.let { ceil(it).toInt() }

    /**
     * The players a timer is set on: the selected one, plus [group] (its sync group) or
     * [all] (every connected player listed). Only well-formed ids, each once.
     */
    fun targets(selected: String, group: List<String> = emptyList(), all: List<String> = emptyList()): List<String> {
        val out = mutableListOf<String>()
        for (raw in listOf(selected) + group + all) {
            val id = raw.trim().lowercase()
            if (PLAYER_ID.matches(id) && id !in out) out += id
        }
        return out
    }

    private val PLAYER_ID = Regex("^[0-9a-f]{2}(:[0-9a-f]{2}){5}$")

    /** The fade-out setting, checked (0–120 s). */
    fun fade(v: Int): Int = v.coerceIn(0, SLEEP_FADE_MAX)
}
