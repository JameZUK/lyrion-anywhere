package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.ui.player.Gapless
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEdit
import io.github.jamezuk.lyrionanywhere.ui.player.QueueSections
import io.github.jamezuk.lyrionanywhere.ui.player.UpNext
import io.github.jamezuk.lyrionanywhere.ui.player.UpNextTracker
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.random.Random

/**
 * Wave 2 §3.4–3.5: the Up next record (Play next from this device, best effort), the
 * queue's sections, the queue's play-next moves, and gapless markers.
 */
class QueueSectionsTest {
    private fun t(id: Long) = sampleTrack(id)

    private fun queue(vararg ids: Long) = ids.map(::t)

    @Test
    fun playNextGoesRightAfterTheCurrentTrackAheadOfEarlierOnes() {
        val a = UpNextTracker.inserted(null, 3, queue(10, 11))
        assertEquals(UpNext(3, listOf(10L, 11L)), a)
        assertEquals(4..5, a!!.range)
        // LMS puts each insert straight after the current track: the newest plays first.
        val b = UpNextTracker.inserted(a, 3, queue(12))
        assertEquals(UpNext(3, listOf(12L, 10L, 11L)), b)
        // Inserted while another track plays: the old block isn't next any more.
        assertEquals(UpNext(5, listOf(13L)), UpNextTracker.inserted(b, 5, queue(13)))
        // Nothing inserted changes nothing.
        assertEquals(b, UpNextTracker.inserted(b, 3, emptyList()))
    }

    @Test
    fun playingOnUsesUpTheBlockAndGoingBackDropsIt() {
        val s = UpNext(3, listOf(10L, 11L))
        assertEquals(s, UpNextTracker.advanced(s, 3))
        assertEquals(UpNext(4, listOf(11L)), UpNextTracker.advanced(s, 4))
        assertNull(UpNextTracker.advanced(s, 5)) // all played
        assertNull(UpNextTracker.advanced(s, 9)) // jumped past it
        assertNull(UpNextTracker.advanced(s, 2)) // back: not next any more
        assertNull(UpNextTracker.advanced(s, null))
        assertNull(UpNextTracker.advanced(null, 3))
    }

    @Test
    fun aQueueThatChangedElsewhereDropsTheSections() {
        val s = UpNext(1, listOf(10L, 11L))
        val q = queue(1, 2, 10, 11, 3)
        assertEquals(s, UpNextTracker.verified(s, 1, q))
        // Another controller inserted a track at the front of the block.
        assertNull(UpNextTracker.verified(s, 1, queue(1, 2, 99, 10, 11, 3)))
        // Only what was read is compared (the rest of the queue isn't loaded yet).
        assertEquals(s, UpNextTracker.verified(s, 1, queue(1, 2, 10)))
        // Played on to the first of them: the rest still match.
        assertEquals(UpNext(2, listOf(11L)), UpNextTracker.verified(s, 2, q))
    }

    @Test
    fun ownRemovalsShiftOrShrinkTheBlock() {
        val s = UpNext(2, listOf(10L, 11L, 12L))
        assertEquals(UpNext(1, listOf(10L, 11L, 12L)), UpNextTracker.removed(s, 0))
        assertNull(UpNextTracker.removed(s, 2)) // the current track
        assertEquals(UpNext(2, listOf(10L, 12L)), UpNextTracker.removed(s, 4))
        assertEquals(s, UpNextTracker.removed(s, 9)) // after it
        assertNull(UpNextTracker.removed(UpNext(2, listOf(10L)), 3)) // the last one
    }

    @Test
    fun undoingARemovalPutsAnUpNextTrackBack() {
        val s = UpNext(2, listOf(10L, 12L))
        assertEquals(UpNext(2, listOf(10L, 11L, 12L)), UpNextTracker.restored(s, 4, 11, wasUpNext = true))
        assertEquals(UpNext(3, listOf(10L, 12L)), UpNextTracker.restored(s, 0, 1, wasUpNext = false))
        // A track from "Then" put back inside the block: can't be accounted for.
        assertNull(UpNextTracker.restored(s, 4, 7, wasUpNext = false))
        assertEquals(s, UpNextTracker.restored(s, 9, 7, wasUpNext = false))
    }

    @Test
    fun movesJoinOrLeaveUpNext() {
        // A track moved to just after the current one joins (a queue swipe right).
        assertEquals(UpNext(1, listOf(50L)), UpNextTracker.moved(null, 6, 2, current = 1, movedId = 50))
        val s = UpNext(1, listOf(10L, 11L, 12L))
        assertEquals(UpNext(1, listOf(50L, 10L, 11L, 12L)), UpNextTracker.moved(s, 7, 2, current = 1, movedId = 50))
        // Between two Up next tracks: joins.
        assertEquals(UpNext(1, listOf(10L, 50L, 11L, 12L)), UpNextTracker.moved(s, 7, 3, current = 1, movedId = 50))
        // Reordered within: kept.
        assertEquals(UpNext(1, listOf(12L, 10L, 11L)), UpNextTracker.moved(s, 4, 2, current = 1, movedId = 12))
        // An Up next track moved away leaves.
        assertEquals(UpNext(1, listOf(10L, 12L)), UpNextTracker.moved(s, 3, 8, current = 1, movedId = 11))
        // A played track moved after the current one: the current shifts down, and it joins.
        assertEquals(UpNext(0, listOf(5L, 10L, 11L, 12L)), UpNextTracker.moved(s, 0, 1, current = 1, movedId = 5))
        // The current track moved away from its block: the block is no longer next.
        assertNull(UpNextTracker.moved(s, 1, 0, current = 1, movedId = 1))
        // Nothing moved.
        assertEquals(s, UpNextTracker.moved(s, 3, 3, current = 1, movedId = 11))
    }

    @Test
    fun theQueueSwipeRightTargetsJustAfterTheCurrentTrack() {
        assertNull(QueueEdit.playNextTarget(3, 3))
        assertNull(QueueEdit.playNextTarget(4, 3))
        assertEquals(4, QueueEdit.playNextTarget(9, 3))
        // A played track: after the move the current track sits one place earlier.
        assertEquals(3, QueueEdit.playNextTarget(0, 3))
        assertNull(QueueEdit.playNextTarget(-1, 3))
        // Where the current track ends up after a move.
        assertEquals(2, QueueEdit.indexAfterMove(3, 0, 5))
        assertEquals(4, QueueEdit.indexAfterMove(3, 7, 1))
        assertEquals(6, QueueEdit.indexAfterMove(3, 3, 6))
        assertEquals(3, QueueEdit.indexAfterMove(3, 7, 9))
        // A played track moved to play next: the current track and it line up.
        val from = 0
        val to = QueueEdit.playNextTarget(from, 3)!!
        assertEquals(to - 1, QueueEdit.indexAfterMove(3, from, to))
    }

    @Test
    fun sectionsOnlyWhenUpNextIsKnownAndLinesUp() {
        val s = QueueSections.of(10, 2, UpNext(2, listOf(10L, 11L)))!!
        assertEquals(2, s.current)
        assertEquals(3..4, s.upNext)
        assertEquals(5, s.thenFrom)
        assertNull(QueueSections.of(10, 2, null)) // flat
        assertNull(QueueSections.of(10, 3, UpNext(2, listOf(10L)))) // out of step
        assertNull(QueueSections.of(10, null, UpNext(2, listOf(10L))))
        assertNull(QueueSections.of(2, 5, UpNext(5, listOf(1L))))
        // Only what's been read is sectioned.
        assertEquals(3..3, QueueSections.of(4, 2, UpNext(2, listOf(10L, 11L, 12L)))!!.upNext)
        assertNull(QueueSections.of(3, 2, UpNext(2, listOf(10L))))
    }

    private fun track(album: Long?, number: Int?, disc: Int? = null) = Track(
        1, "T", null, null, album, null, null, number?.toUInt(), null, null, disc?.toUInt(), null, null, null, null,
    )

    @Test
    fun gaplessMarksOnlyNextTrackNumbersOfOneAlbum() {
        assertTrue(Gapless.follows(track(7, 1), track(7, 2)))
        assertFalse(Gapless.follows(track(7, 1), track(7, 3)))
        assertFalse(Gapless.follows(track(7, 2), track(7, 1)))
        assertFalse(Gapless.follows(track(7, 1), track(8, 2)))
        assertFalse(Gapless.follows(track(null, 1), track(null, 2)))
        assertFalse(Gapless.follows(track(7, null), track(7, 2)))
        // Discs, where known, must match.
        assertTrue(Gapless.follows(track(7, 9, 1), track(7, 10, 1)))
        assertFalse(Gapless.follows(track(7, 9, 1), track(7, 10, 2)))
        assertTrue(Gapless.follows(track(7, 9, null), track(7, 10, 2)))
        // No overflow at the top.
        assertFalse(Gapless.follows(Track(1, "T", null, null, 7, null, null, UInt.MAX_VALUE, null, null, null, null, null, null, null), track(7, 0)))
    }

    /** Whatever edits come, in whatever order, the tracker never throws and stays in bounds. */
    @Test
    fun theTrackerSurvivesArbitraryEdits() {
        val r = Random(11)
        var s: UpNext? = null
        repeat(20_000) {
            val cur = r.nextInt(-2, 40)
            s = when (r.nextInt(7)) {
                0 -> UpNextTracker.inserted(s, cur, queue(*LongArray(r.nextInt(0, 5)) { r.nextLong(1, 99) }))
                1 -> UpNextTracker.advanced(s, cur.takeIf { r.nextBoolean() })
                2 -> UpNextTracker.verified(s, cur, queue(*LongArray(r.nextInt(0, 50)) { r.nextLong(1, 99) }))
                3 -> UpNextTracker.removed(s, r.nextInt(-3, 60))
                4 -> UpNextTracker.moved(s, r.nextInt(-3, 60), r.nextInt(-3, 60), cur.takeIf { r.nextBoolean() }, r.nextLong(1, 99))
                5 -> UpNextTracker.restored(s, r.nextInt(-3, 60), r.nextLong(1, 99), r.nextBoolean())
                else -> if (r.nextInt(10) == 0) null else s
            }
            s?.let {
                assertTrue(it.current >= 0)
                assertTrue(it.ids.isNotEmpty() && it.ids.size <= UpNextTracker.MAX)
                QueueSections.of(r.nextInt(0, 80), it.current, it)
            }
        }
    }
}
