package io.github.jamezuk.lyrionanywhere.playback

import androidx.media3.common.Player
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.random.Random

/**
 * The gap between one track and the next while the screen is off: the session keeps its
 * playlist and the locks stay held, so the next track starts without the phone being
 * woken (pure; runs on the JVM).
 */
class PlaybackGapTest {
    private val playing = SessionSnapshot(local = running(), status = lmsStatus(), queue = QUEUE)

    /** Mid-change: LMS stopped with nothing in its playlist yet, the engine flushed. */
    private val between = SessionSnapshot(
        local = running(LocalPlayback.STOPPED),
        status = lmsStatus(mode = PlaybackMode.STOPPED, current = null),
        queue = emptyList(),
    )

    private fun SessionHoldover.at(s: SessionSnapshot, now: Long) = apply(mapSnapshot(s), s, now)

    @Test
    fun theLastPlaylistShowsThroughAGapThenGoes() {
        val h = SessionHoldover(holdMs = 1_000)
        assertNull(h.at(playing, 0).second)
        val (kept, left) = h.at(between, 100)
        assertEquals(3, kept.items.size)
        assertEquals("Track 11", kept.current?.title)
        assertEquals("LMS stopped: shown paused", Player.STATE_READY, kept.playbackState)
        assertFalse(kept.playWhenReady || kept.advancing)
        assertEquals(1_000L, left)
        // Still the same gap: the time counts from its start.
        assertEquals(400L, h.at(between, 700).second)
        // Time up: idle, as LMS says.
        val (gone, none) = h.at(between, 1_100)
        assertEquals(Player.STATE_IDLE, gone.playbackState)
        assertTrue(gone.items.isEmpty())
        assertNull(none)
        // And it isn't brought back by a later gap.
        assertTrue(h.at(between, 1_200).first.items.isEmpty())
    }

    @Test
    fun theEngineStartingTheNextTrackShowsAsBuffering() {
        val h = SessionHoldover()
        h.at(playing, 0)
        val starting = between.copy(local = running(LocalPlayback.BUFFERING))
        val (s, _) = h.at(starting, 10)
        assertEquals(Player.STATE_BUFFERING, s.playbackState)
        assertTrue(s.playWhenReady)
        // LMS playing with its playlist not read yet: the same.
        val lms = between.copy(status = lmsStatus(current = null))
        assertTrue(h.at(lms, 20).first.playWhenReady)
    }

    @Test
    fun theNewPlaylistReplacesTheKeptOneAndRestartsTheClock() {
        val h = SessionHoldover(holdMs = 1_000)
        h.at(playing, 0)
        h.at(between, 100)
        val next = SessionSnapshot(local = running(), status = lmsStatus(index = 0, current = track(20, 0)), queue = listOf(track(20, 0)))
        val (s, left) = h.at(next, 900)
        assertEquals("Track 20", s.current?.title)
        assertNull(left)
        // A second gap gets its own full time.
        assertEquals(1_000L, h.at(between, 1_500).second)
        assertEquals("Track 20", h.at(between, 2_400).first.current?.title)
    }

    @Test
    fun nothingIsKeptOnceThePhoneIsNoLongerAPlayer() {
        for (gone in listOf(
            LocalPlayerState(),
            LocalPlayerState(enabled = false, status = localStatus()),
            LocalPlayerState(enabled = true, status = localStatus(connection = PlayerConnection.Stopped("signed out"))),
        )) {
            val h = SessionHoldover()
            h.at(playing, 0)
            val (s, left) = h.at(SessionSnapshot(local = gone), 10)
            assertEquals(Player.STATE_IDLE, s.playbackState)
            assertTrue(s.items.isEmpty())
            assertNull(left)
            // Nor afterwards, when it is a player again with nothing yet.
            assertTrue(h.at(between, 20).first.items.isEmpty())
        }
    }

    @Test
    fun nothingIsMadeUpBeforeAnyPlaylist() {
        val h = SessionHoldover()
        val (s, left) = h.at(between, 0)
        assertTrue(s.items.isEmpty())
        assertNull(left)
    }

    @Test
    fun reconnectingStillCountsAsAPlayer() {
        val h = SessionHoldover()
        h.at(playing, 0)
        val reconnecting = between.copy(
            local = LocalPlayerState(enabled = true, status = localStatus(LocalPlayback.STOPPED, PlayerConnection.Reconnecting(500uL))),
        )
        assertEquals(3, h.at(reconnecting, 10).first.items.size)
    }

    /** Arbitrary snapshots and times: never a playlist for a phone that isn't a player, never kept too long. */
    @Test
    fun robustOnArbitrarySequences() {
        val rnd = Random(20261015)
        val locals = listOf(
            running(), running(LocalPlayback.STOPPED), running(LocalPlayback.BUFFERING), running(LocalPlayback.PAUSED),
            LocalPlayerState(), LocalPlayerState(enabled = true, status = localStatus(connection = PlayerConnection.Stopped("x"))),
        )
        val statuses = listOf(null, lmsStatus(), lmsStatus(mode = PlaybackMode.STOPPED, current = null), lmsStatus(mode = PlaybackMode.PAUSED))
        val queues = listOf(QUEUE, emptyList())
        repeat(200) {
            val hold = rnd.nextLong(1, 5_000)
            val h = SessionHoldover(hold)
            var now = rnd.nextLong(-1_000_000, 1_000_000)
            var emptySince: Long? = null
            repeat(100) {
                now += rnd.nextLong(0, 2_000)
                val snap = SessionSnapshot(local = locals.random(rnd), status = statuses.random(rnd), queue = queues.random(rnd))
                val mapped = mapSnapshot(snap)
                val (s, left) = h.apply(mapped, snap, now)
                if (mapped.items.isNotEmpty()) {
                    assertEquals(mapped, s)
                    emptySince = null
                } else if (s.items.isNotEmpty()) {
                    assertTrue(snap.local.running)
                    val since = emptySince ?: now.also { emptySince = it }
                    assertTrue(now - since < hold)
                    assertTrue(left != null && left in 1..hold)
                } else {
                    emptySince = null
                }
            }
        }
    }

    @Test
    fun theLocksStayThroughAGapAndGoAfterTheGrace() {
        val k = LockKeeper(graceMs = 1_000)
        assertEquals(true to null, k.decide(enabled = true, active = true, now = 0))
        // LMS's flush: not playing, but the locks stay for the next track.
        assertEquals(true to 1_000L, k.decide(enabled = true, active = false, now = 0))
        assertEquals(true to 500L, k.decide(enabled = true, active = false, now = 500))
        // The next track starts in time: held, and the grace starts again afterwards.
        assertEquals(true to null, k.decide(enabled = true, active = true, now = 800))
        assertEquals(true to 1_000L, k.decide(enabled = true, active = false, now = 800))
        // A real stop: let go once the grace is over.
        assertEquals(false to null, k.decide(enabled = true, active = false, now = 1_800))
        assertEquals(false to null, k.decide(enabled = true, active = false, now = 5_000))
    }

    @Test
    fun theLocksAreNeverTakenWithoutPlaybackAndGoAtOnceWhenSwitchedOff() {
        val k = LockKeeper(graceMs = 1_000)
        assertEquals("never played", false to null, k.decide(enabled = true, active = false, now = 0))
        k.decide(enabled = true, active = true, now = 10)
        assertEquals("switched off", false to null, k.decide(enabled = false, active = false, now = 20))
        // Switched on again: no grace left over from before.
        assertEquals(false to null, k.decide(enabled = true, active = false, now = 30))
        // Disabled wins even while "active".
        assertEquals(false to null, k.decide(enabled = false, active = true, now = 40))
    }

    @Test
    fun aClockGoingBackwardsNeverExtendsTheGrace() {
        val k = LockKeeper(graceMs = 1_000)
        k.decide(enabled = true, active = true, now = 5_000)
        val (hold, left) = k.decide(enabled = true, active = false, now = 4_000)
        assertTrue(hold)
        assertEquals(1_000L, left)
    }

    @Test
    fun locksRobustOnArbitraryInputs() {
        val rnd = Random(1015)
        repeat(200) {
            val grace = rnd.nextLong(1, 10_000)
            val k = LockKeeper(grace)
            var now = rnd.nextLong(-1_000_000, 1_000_000)
            var lastActive: Long? = null
            repeat(200) {
                now += rnd.nextLong(0, 3_000)
                val enabled = rnd.nextInt(5) != 0
                val active = rnd.nextBoolean()
                val (hold, left) = k.decide(enabled, active, now)
                when {
                    !enabled -> {
                        assertFalse(hold)
                        lastActive = null
                    }
                    active -> {
                        assertTrue(hold)
                        assertNull(left)
                        lastActive = now
                    }
                    else -> {
                        val expected = lastActive?.let { now - it < grace } ?: false
                        assertEquals(expected, hold)
                        if (hold) assertTrue(left != null && left in 1..grace) else lastActive = null
                    }
                }
            }
        }
    }
}
