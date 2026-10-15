package io.github.jamezuk.lyrionanywhere.playback

import android.os.Looper
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/** The Media3 player mirrors LMS and turns button presses into LMS commands. */
@OptIn(UnstableApi::class)
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class LmsSessionPlayerTest {
    private val scope = CoroutineScope(SupervisorJob())
    private val actions = RecordingActions()
    private val snapshots = MutableStateFlow(
        SessionSnapshot(
            local = running(),
            status = lmsStatus(),
            statusAt = 0,
            queue = QUEUE,
            artwork = Artwork("music/11/cover", byteArrayOf(7, 8, 9)),
        ),
    )

    @After
    fun tearDown() = scope.cancel()

    private fun idle() = shadowOf(Looper.getMainLooper()).idle()

    private fun player(): LmsSessionPlayer {
        val p = LmsSessionPlayer(Looper.getMainLooper(), snapshots, actions, scope)
        idle()
        return p
    }

    @Test
    fun withTheServerAwayOnlyWorkingButtonsAreAdvertised() {
        val p = player()
        for (c in listOf(Player.COMMAND_PLAY_PAUSE, Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_MEDIA_ITEM)) {
            assertTrue("online: $c", p.isCommandAvailable(c))
        }
        snapshots.value = snapshots.value.copy(local = away(offline(next = true, previous = false, seek = false)))
        idle()
        assertTrue(p.isCommandAvailable(Player.COMMAND_PLAY_PAUSE))
        assertTrue(p.isCommandAvailable(Player.COMMAND_SEEK_TO_NEXT))
        assertFalse(p.isCommandAvailable(Player.COMMAND_SEEK_TO_PREVIOUS))
        assertFalse(p.isCommandAvailable(Player.COMMAND_SEEK_TO_MEDIA_ITEM))
        assertFalse(p.isCommandAvailable(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM))
        // Waiting for the server with nothing to play: not even play.
        snapshots.value = snapshots.value.copy(local = away(offline(playPause = false)))
        idle()
        assertFalse(p.isCommandAvailable(Player.COMMAND_PLAY_PAUSE))
        assertFalse(p.isCommandAvailable(Player.COMMAND_SEEK_TO_NEXT))
    }

    @Test
    fun mirrorsLmsState() {
        val p = player()
        assertEquals(Player.STATE_READY, p.playbackState)
        assertTrue(p.playWhenReady)
        assertTrue(p.isPlaying)
        assertEquals(3, p.mediaItemCount)
        assertEquals(1, p.currentMediaItemIndex)
        val meta = p.mediaMetadata
        assertEquals("Track 11", meta.title.toString())
        assertEquals("Artist", meta.artist.toString())
        assertArrayEquals(byteArrayOf(7, 8, 9), meta.artworkData)
        assertEquals(MediaMetadata.PICTURE_TYPE_FRONT_COVER, meta.artworkDataType)
        assertEquals(200_000L, p.duration)
        assertTrue(p.currentPosition >= 12_500)
        assertTrue(p.isCommandAvailable(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM))
        assertTrue(p.isCommandAvailable(Player.COMMAND_SEEK_TO_NEXT))
    }

    @Test
    fun followsChangesFromLms() {
        val p = player()
        snapshots.value = snapshots.value.copy(status = lmsStatus(mode = PlaybackMode.PAUSED, index = 2, current = track(12, 2)))
        idle()
        assertFalse(p.playWhenReady)
        assertEquals(2, p.currentMediaItemIndex)
        assertEquals("Track 12", p.mediaMetadata.title.toString())
        assertNull("art belongs to another track", p.mediaMetadata.artworkData)

        snapshots.value = snapshots.value.copy(local = running(LocalPlayback.BUFFERING), status = lmsStatus())
        idle()
        assertEquals(Player.STATE_BUFFERING, p.playbackState)

        snapshots.value = SessionSnapshot(local = LocalPlayerState())
        idle()
        assertEquals(Player.STATE_IDLE, p.playbackState)
        assertEquals(0, p.mediaItemCount)
    }

    @Test
    fun buttonsBecomeLmsCommands() {
        val p = player()
        p.pause()
        idle()
        p.play()
        idle()
        p.seekToNext()
        idle()
        p.seekToPrevious()
        idle()
        p.seekTo(30_000)
        idle()
        p.seekTo(0, 0)
        idle()
        p.stop()
        idle()
        assertEquals(listOf("pause", "play", "next", "previous", "seek 30.0", "jump 0", "pause"), actions.calls)
    }

    @Test
    fun theCarsPlayRequestsGoToThePhoneWithoutAnExtraPlay() {
        val asked = mutableListOf<Pair<String, String?>>()
        val p = LmsSessionPlayer(
            Looper.getMainLooper(),
            snapshots,
            actions,
            scope,
            PlayRequests { id, q ->
                asked += id to q
                true
            },
        )
        idle()
        assertTrue(p.isCommandAvailable(Player.COMMAND_SET_MEDIA_ITEM))
        assertFalse(p.isCommandAvailable(Player.COMMAND_CHANGE_MEDIA_ITEMS))
        // As Media3 does for "play from media id": set, prepare, play.
        p.setMediaItem(androidx.media3.common.MediaItem.Builder().setMediaId("n7").build())
        p.prepare()
        p.play()
        idle()
        assertEquals(listOf("n7" to null), asked)
        assertFalse("the request plays by itself", "play" in actions.calls)
        // Afterwards the state is LMS's again, and play is a play.
        p.play()
        idle()
        assertTrue("play" in actions.calls)
    }

    @Test
    fun withoutPlayRequestsNothingCanBeSetById() {
        val p = player()
        assertFalse(p.isCommandAvailable(Player.COMMAND_SET_MEDIA_ITEM))
    }

    @Test
    fun noSeekingWhenLmsSaysNo() {
        snapshots.value = snapshots.value.copy(status = lmsStatus(canSeek = false))
        val p = player()
        assertFalse(p.isCommandAvailable(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM))
    }

    /**
     * Regression: a queue replaced from another device (LMS stops, clears the playlist,
     * then loads the new one) used to read for a moment as no playlist at all. The session
     * then went idle with an empty timeline, Media3 took the notification away and dropped
     * the foreground service at once, and Android doesn't let a backgrounded app go back
     * to the foreground: with the screen off the next track waited for the phone to wake.
     */
    @Test
    fun aQueueReplacedFromElsewhereKeepsTheSessionThroughTheGap() {
        val p = player()
        assertTrue(p.isPlaying)
        // Mid-change: LMS stopped, nothing in the playlist yet; the engine flushed.
        snapshots.value = snapshots.value.copy(
            local = running(LocalPlayback.STOPPED),
            status = lmsStatus(mode = PlaybackMode.STOPPED, current = null),
            queue = emptyList(),
        )
        idle()
        assertFalse("the timeline stays, so the notification and service stay", p.currentTimeline.isEmpty)
        assertTrue(p.playbackState != Player.STATE_IDLE)
        assertEquals("Track 11", p.mediaMetadata.title.toString())
        // The new queue arrives and plays.
        snapshots.value = snapshots.value.copy(
            local = running(LocalPlayback.BUFFERING),
            status = lmsStatus(index = 0, current = track(20, 0)),
            queue = listOf(track(20, 0), track(21, 1)),
        )
        idle()
        assertEquals(2, p.mediaItemCount)
        assertEquals("Track 20", p.mediaMetadata.title.toString())
        assertTrue(p.playWhenReady)
    }

    @Test
    fun aKeptPlaylistGoesByItselfOnceItsTimeIsUp() {
        var now = 0L
        val p = LmsSessionPlayer(Looper.getMainLooper(), snapshots, actions, scope, clock = { now })
        idle()
        snapshots.value = SessionSnapshot(local = running(LocalPlayback.STOPPED), status = lmsStatus(mode = PlaybackMode.STOPPED, current = null))
        idle()
        assertEquals(3, p.mediaItemCount)
        // A jump from the kept list still names LMS's entry.
        p.seekTo(2, 0)
        idle()
        assertEquals("jump 2", actions.calls.last())
        now = SessionHoldover.HOLD_MS
        shadowOf(Looper.getMainLooper()).idleFor(java.time.Duration.ofMillis(SessionHoldover.HOLD_MS))
        assertEquals(Player.STATE_IDLE, p.playbackState)
        assertTrue(p.currentTimeline.isEmpty)
    }
}
