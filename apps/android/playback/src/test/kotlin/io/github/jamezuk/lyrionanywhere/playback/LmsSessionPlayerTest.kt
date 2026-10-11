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
    fun noSeekingWhenLmsSaysNo() {
        snapshots.value = snapshots.value.copy(status = lmsStatus(canSeek = false))
        val p = player()
        assertFalse(p.isCommandAvailable(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM))
    }
}
