package io.github.jamezuk.lyrionanywhere.playback

import android.os.Looper
import androidx.annotation.OptIn
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.util.UnstableApi
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.OfflinePlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
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

fun dlTrack(n: Int, albumId: Long? = 7) = OfflineTrack(
    "%064x".format(n), 100L + n, "Song $n", "Band", "Record", albumId, n.toUInt(), 120.0,
    DownloadFormat.ORIGINAL, "flc", "flac", 1000uL, 0uL, DownloadOrigin.MANUAL,
)

fun dlState(
    index: Int? = 1,
    playback: LocalPlayback = LocalPlayback.PLAYING,
    order: List<Int> = listOf(0, 1, 2),
    shuffle: Boolean = false,
    repeat: LocalRepeat = LocalRepeat.OFF,
): OfflinePlayerState {
    val tracks = listOf(dlTrack(1), dlTrack(2), dlTrack(3))
    return OfflinePlayerState(
        status = OfflinePlayerStatus(
            playback, index?.toUInt(), tracks.map { it.key }, order.map { it.toUInt() }, 30_000uL, 120_000uL,
            shuffle, repeat, 400uL, 44_100u, 0uL, false, null,
        ),
        tracks = tracks.associateBy { it.key },
    )
}

class RecordingOffline(initial: OfflinePlayerState = dlState()) : OfflinePlayerRepository {
    val flow = MutableStateFlow(initial)
    override val state: StateFlow<OfflinePlayerState> = flow
    val calls = mutableListOf<String>()

    override fun play(tracks: List<OfflineTrack>, start: Int, shuffle: Boolean) {
        calls += "queue ${tracks.size} $start $shuffle"
    }

    override fun play() {
        calls += "play"
    }

    override fun pause() {
        calls += "pause"
    }

    override fun next() {
        calls += "next"
    }

    override fun previous() {
        calls += "previous"
    }

    override fun seekTo(ms: Long) {
        calls += "seek $ms"
    }

    override fun jumpTo(index: Int) {
        calls += "jump $index"
    }

    override fun setShuffle(on: Boolean) {
        calls += "shuffle $on"
    }

    override fun setRepeat(repeat: LocalRepeat) {
        calls += "repeat $repeat"
    }

    override fun stop() {
        calls += "stop"
    }
}

/** The pure mapping from the local queue to the session. */
class OfflineMappingTest {
    @Test
    fun noQueueIsIdle() {
        assertEquals(Player.STATE_IDLE, mapOffline(OfflinePlayerState()).playbackState)
        assertEquals(Player.STATE_IDLE, mapOffline(dlState(index = null)).playbackState)
    }

    @Test
    fun theQueueInPlayOrder() {
        val m = mapOffline(dlState(index = 0, order = listOf(2, 0, 1), shuffle = true, repeat = LocalRepeat.ONE))
        assertEquals(listOf(2, 0, 1), m.items.map { it.listIndex })
        assertEquals("list index 0 is second in play order", 1, m.currentIndex)
        assertTrue(m.shuffle)
        assertEquals(Player.REPEAT_MODE_ONE, m.repeatMode)
        assertEquals(120_000L, m.items[0].durationMs)
        assertEquals("a7", m.items[0].albumRef)
        assertEquals(30_000L, m.positionMs)
        assertTrue(m.playWhenReady && m.advancing)
    }

    @Test
    fun statesMap() {
        assertEquals(Player.STATE_BUFFERING, mapOffline(dlState(playback = LocalPlayback.BUFFERING)).playbackState)
        val paused = mapOffline(dlState(playback = LocalPlayback.PAUSED))
        assertEquals(Player.STATE_READY, paused.playbackState)
        assertFalse(paused.playWhenReady || paused.advancing)
        assertEquals(Player.STATE_ENDED, mapOffline(dlState(playback = LocalPlayback.STOPPED)).playbackState)
        assertEquals(LocalRepeat.ALL, repeatFromMedia3(Player.REPEAT_MODE_ALL))
        assertEquals(LocalRepeat.OFF, repeatFromMedia3(99))
        LocalRepeat.entries.forEach { assertEquals(it, repeatFromMedia3(it.toMedia3())) }
    }

    @Test
    fun aTrackWithoutAnAlbumHasItsOwnCover() {
        val t = dlTrack(5, albumId = null)
        val s = OfflinePlayerState(
            status = OfflinePlayerStatus(
                LocalPlayback.PLAYING, 0u, listOf(t.key), listOf(0u), 0uL, 0uL, false, LocalRepeat.OFF, 0uL, 0u, 0uL, false, null,
            ),
            tracks = mapOf(t.key to t),
        )
        assertEquals("t${t.key.take(16)}", mapOffline(s).items.single().albumRef)
    }
}

/** The session player while downloads play: state and buttons. */
@OptIn(UnstableApi::class)
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class OfflineSessionPlayerTest {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val offline = RecordingOffline()

    @After
    fun tearDown() = scope.cancel()

    private fun idle() = shadowOf(Looper.getMainLooper()).idle()

    private fun player(): OfflineSessionPlayer {
        val controller = OfflinePlaybackController(offline, { ref -> if (ref == "a7") byteArrayOf(4, 5, 6) else null }, scope) { 0L }
        controller.start()
        val p = OfflineSessionPlayer(Looper.getMainLooper(), controller.snapshot, offline, scope)
        idle()
        return p
    }

    @Test
    fun showsTheLocalQueueWithItsCover() {
        val p = player()
        assertEquals(Player.STATE_READY, p.playbackState)
        assertTrue(p.isPlaying)
        assertEquals(3, p.mediaItemCount)
        assertEquals(1, p.currentMediaItemIndex)
        val meta = p.mediaMetadata
        assertEquals("Song 2", meta.title.toString())
        assertEquals("Band", meta.artist.toString())
        assertArrayEquals(byteArrayOf(4, 5, 6), meta.artworkData)
        assertEquals(MediaMetadata.PICTURE_TYPE_FRONT_COVER, meta.artworkDataType)
        assertEquals(120_000L, p.duration)
        assertTrue(p.isCommandAvailable(Player.COMMAND_SET_SHUFFLE_MODE))
        assertTrue(p.isCommandAvailable(Player.COMMAND_SET_REPEAT_MODE))
    }

    @Test
    fun buttonsDriveTheLocalQueue() {
        val p = player()
        p.pause()
        p.play()
        p.seekToNext()
        p.seekToPrevious()
        p.seekTo(45_000)
        p.seekTo(2, 0)
        p.shuffleModeEnabled = true
        p.repeatMode = Player.REPEAT_MODE_ALL
        p.stop()
        idle()
        assertEquals(
            listOf("pause", "play", "next", "previous", "seek 45000", "jump 2", "shuffle true", "repeat ALL", "pause"),
            offline.calls,
        )
    }

    @Test
    fun followsTheQueue() {
        val p = player()
        offline.flow.value = dlState(index = 2, playback = LocalPlayback.PAUSED, shuffle = true, repeat = LocalRepeat.ALL)
        idle()
        assertEquals(2, p.currentMediaItemIndex)
        assertFalse(p.playWhenReady)
        assertTrue(p.shuffleModeEnabled)
        assertEquals(Player.REPEAT_MODE_ALL, p.repeatMode)
        offline.flow.value = OfflinePlayerState()
        idle()
        assertEquals(Player.STATE_IDLE, p.playbackState)
        assertNull(p.currentMediaItem)
    }
}
