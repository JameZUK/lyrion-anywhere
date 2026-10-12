package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.SleepEnd
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.sanitiseSleepFade
import io.github.jamezuk.lyrionanywhere.ui.player.DragAxis
import io.github.jamezuk.lyrionanywhere.ui.player.MiniGestures
import io.github.jamezuk.lyrionanywhere.ui.player.SleepArm
import io.github.jamezuk.lyrionanywhere.ui.player.SleepChoice
import io.github.jamezuk.lyrionanywhere.ui.player.SleepTimer
import io.github.jamezuk.lyrionanywhere.ui.player.SwipeAction
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The sleep timer's rules (Wave 1 §1), with the vectors the web app (`sleep.test.ts`) and
 * the Rust core (`mobile/src/sleep.rs`) use.
 */
class SleepTimerTest {
    private fun t(album: Long, duration: Double?) = Track(
        album * 100, "T", null, null, album, null, duration, null, null, null, null, null, null, null, null,
    )

    private fun status(elapsed: Double = 60.0, duration: Double? = 200.0, index: Int = 2, length: Int = 6) =
        sampleStatus(elapsed = elapsed).copy(
            durationSecs = duration,
            queueIndex = index.toUInt(),
            queueLength = length.toUInt(),
            current = t(7, 200.0),
        )

    @Test
    fun endOfTrack() {
        assertEquals(140, SleepTimer.remaining(SleepEnd.TRACK, status()))
        assertEquals(141, SleepTimer.remaining(SleepEnd.TRACK, status(elapsed = 59.2)))
        assertEquals(0, SleepTimer.remaining(SleepEnd.TRACK, status(elapsed = 250.0)))
        // The track's own duration when the status has none.
        assertEquals(140, SleepTimer.remaining(SleepEnd.TRACK, status(duration = null)))
    }

    @Test
    fun endOfAlbumIsTheRunOfTheSameAlbum() {
        val up = listOf(t(7, 180.0), t(7, 240.0), t(8, 300.0), t(7, 100.0))
        assertEquals(140 + 180 + 240, SleepTimer.remaining(SleepEnd.ALBUM, status(), up))
        assertEquals(140, SleepTimer.remaining(SleepEnd.ALBUM, status(), listOf(t(9, 100.0))))
        assertNull(SleepTimer.remaining(SleepEnd.ALBUM, status(), listOf(t(7, 180.0), t(7, null))))
        val noAlbum = status().let { it.copy(current = it.current!!.copy(albumId = null)) }
        assertEquals(140, SleepTimer.remaining(SleepEnd.ALBUM, noAlbum, up))
    }

    @Test
    fun endOfQueueNeedsTheWholeQueue() {
        val up = listOf(t(7, 180.0), t(8, 240.0), t(9, 300.0))
        assertEquals(140 + 720, SleepTimer.remaining(SleepEnd.QUEUE, status(), up))
        assertEquals(140, SleepTimer.remaining(SleepEnd.QUEUE, status(index = 5), emptyList()))
        assertNull(SleepTimer.remaining(SleepEnd.QUEUE, status(length = 900), up))
        assertNull(SleepTimer.remaining(SleepEnd.QUEUE, status(), listOf(t(7, 180.0), t(8, null), t(9, 300.0))))
    }

    @Test
    fun anEndMoreThanADayAwayIsNotOffered() {
        val long = List(30) { t(8, 3600.0) }
        assertNull(SleepTimer.remaining(SleepEnd.QUEUE, status(index = 0, length = 31), long))
        assertEquals(140 + 23 * 3600, SleepTimer.remaining(SleepEnd.QUEUE, status(index = 0, length = 24), long.take(23)))
        assertNull(SleepTimer.remaining(SleepEnd.TRACK, status(duration = 1e9)))
    }

    @Test
    fun nothingPlayingOrNoLengthIsUnknowable() {
        assertNull(SleepTimer.remaining(SleepEnd.TRACK, null))
        assertNull(SleepTimer.remaining(SleepEnd.TRACK, status().copy(current = null)))
        assertNull(SleepTimer.remaining(SleepEnd.TRACK, status(duration = null).copy(current = t(7, null))))
    }

    @Test
    fun hostileNumbersNeverMakeANegativeOrEndlessTimer() {
        for (bad in listOf(Double.NaN, Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, -5.0, 1e308, Double.MAX_VALUE)) {
            for (end in SleepEnd.entries) {
                val s = status(elapsed = bad, duration = bad).copy(current = t(7, bad), queueIndex = UInt.MAX_VALUE, queueLength = UInt.MAX_VALUE)
                val r = SleepTimer.remaining(end, s, listOf(t(7, bad)))
                assertTrue("$bad $end $r", r == null || r in 0..86_400)
            }
        }
    }

    @Test
    fun fixedAndCustomLengths() {
        assertEquals(1800, SleepTimer.seconds(SleepChoice.Minutes(30), null))
        assertNull(SleepTimer.seconds(SleepChoice.Minutes(0), null))
        assertNull(SleepTimer.seconds(SleepChoice.Minutes(1441), null))
        assertEquals(140, SleepTimer.seconds(SleepChoice.End(SleepEnd.TRACK), status()))
        assertEquals(45, SleepTimer.customMinutes("45"))
        assertEquals(90, SleepTimer.customMinutes(" 90 "))
        for (bad in listOf("", "0", "-5", "1.5", "1e3", "1441", "abc", "45;sleep 0", "99999", "٤٥")) {
            assertNull(bad, SleepTimer.customMinutes(bad))
        }
    }

    @Test
    fun rearmOnlyWhenTheEndMoves() {
        val set = 1_000_000L
        val arm = SleepArm(SleepEnd.TRACK, listOf("aa:bb:cc:dd:ee:01"), set + 140_000, set)
        assertNull(SleepTimer.rearm(arm, 140, set))
        assertNull(SleepTimer.rearm(arm, 136, set + 2_000))
        assertEquals(40, SleepTimer.rearm(arm, 40, set))
        assertEquals(140, SleepTimer.rearm(arm, 140, set + 30_000))
        assertEquals(1, SleepTimer.rearm(arm, 0, set + 120_000))
        assertNull(SleepTimer.rearm(arm, null, set))
    }

    @Test
    fun armedIsOverOnceLmsStopsReportingIt() {
        val set = 1_000_000L
        val arm = SleepArm(SleepEnd.TRACK, listOf("aa:bb:cc:dd:ee:01"), set + 140_000, set)
        assertFalse(SleepTimer.over(arm, 120.0, set + 20_000))
        assertFalse(SleepTimer.over(arm, null, set + 3_000))
        assertTrue(SleepTimer.over(arm, null, set + 11_000))
        assertTrue(SleepTimer.over(arm, 0.0, set + 11_000))
        assertTrue(SleepTimer.over(arm, Double.NaN, set + 11_000))
    }

    @Test
    fun whatLmsReports() {
        assertEquals(1800, SleepTimer.left(sampleStatus().copy(sleepSecs = 1799.4)))
        for (none in listOf(null, 0.0, -3.0, Double.NaN, Double.POSITIVE_INFINITY)) {
            assertNull("$none", SleepTimer.left(sampleStatus().copy(sleepSecs = none)))
        }
        assertNull(SleepTimer.left(null))
    }

    @Test
    fun targetsAreWellFormedAndDistinct() {
        val me = "AA:BB:CC:DD:EE:01"
        assertEquals(listOf("aa:bb:cc:dd:ee:01"), SleepTimer.targets(me))
        assertEquals(
            listOf("aa:bb:cc:dd:ee:01", "aa:bb:cc:dd:ee:02", "aa:bb:cc:dd:ee:03"),
            SleepTimer.targets(
                me,
                listOf("aa:bb:cc:dd:ee:02", "aa:bb:cc:dd:ee:01"),
                listOf("aa:bb:cc:dd:ee:03", "../x", "", "all", "aa:bb:cc:dd:ee:04;rescan"),
            ),
        )
        assertTrue(SleepTimer.targets("bad").isEmpty())
    }

    @Test
    fun theFadeSettingIsChecked() {
        assertEquals(30, sanitiseSleepFade(null))
        assertEquals(90, sanitiseSleepFade(90))
        assertEquals(0, sanitiseSleepFade(0))
        for (bad in listOf(-1, 121, 9_999, Int.MAX_VALUE, Int.MIN_VALUE)) assertEquals(30, sanitiseSleepFade(bad))
        assertEquals(120, SleepTimer.fade(500))
        assertEquals(0, SleepTimer.fade(-4))
    }
}

/** The mini player's swipes (Wave 1 §4), as the web app's `gesture.ts`. */
class MiniGesturesTest {
    @Test
    fun theAxisIsClearlyAlongOneWay() {
        assertEquals(DragAxis.HORIZONTAL, MiniGestures.axis(-100f, 5f))
        assertEquals(DragAxis.VERTICAL, MiniGestures.axis(3f, -60f))
        assertNull(MiniGestures.axis(80f, -80f)) // a diagonal is not ours
        assertNull(MiniGestures.axis(Float.NaN, 0f))
    }

    @Test
    fun swipes() {
        fun c(axis: DragAxis?, dx: Float, dy: Float, w: Float = 360f) = MiniGestures.classify(axis, dx, dy, w, 60f, 40f)
        assertEquals(SwipeAction.NEXT, c(DragAxis.HORIZONTAL, -100f, 0f))
        assertEquals(SwipeAction.PREVIOUS, c(DragAxis.HORIZONTAL, 100f, 0f))
        assertEquals(SwipeAction.NONE, c(DragAxis.HORIZONTAL, -40f, 0f))
        assertEquals(SwipeAction.OPEN, c(DragAxis.VERTICAL, 0f, -40f))
        assertEquals(SwipeAction.NONE, c(DragAxis.VERTICAL, 0f, -39f))
        assertEquals(SwipeAction.NONE, c(DragAxis.VERTICAL, 0f, 80f)) // down does nothing here
        // A wide bar needs a longer swipe (a share of its width).
        assertEquals(SwipeAction.NONE, c(DragAxis.HORIZONTAL, -150f, 0f, 1000f))
        assertEquals(SwipeAction.NEXT, c(DragAxis.HORIZONTAL, -220f, 0f, 1000f))
        // Nonsense never skips.
        for (bad in listOf(Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY)) {
            assertEquals(SwipeAction.NONE, c(DragAxis.HORIZONTAL, bad, 0f))
            assertEquals(SwipeAction.NONE, c(DragAxis.VERTICAL, 0f, bad))
        }
        assertEquals(SwipeAction.NONE, c(null, -500f, 0f))
        assertEquals(SwipeAction.NEXT, c(DragAxis.HORIZONTAL, -80f, 0f, Float.NaN))
    }

    @Test
    fun arbitraryDragsNeverThrow() {
        val r = java.util.Random(7)
        val odd = listOf(Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY, Float.MAX_VALUE, -0f)
        fun pick() = if (r.nextInt(10) == 0) odd[r.nextInt(odd.size)] else (r.nextFloat() - 0.5f) * 4000f
        repeat(5_000) {
            val dx = pick()
            val dy = pick()
            val a = MiniGestures.axis(dx, dy)
            MiniGestures.classify(a, dx, dy, pick(), 60f, 40f)
        }
    }
}
