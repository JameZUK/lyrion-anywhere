package io.github.jamezuk.lyrionanywhere.playback

import android.media.AudioDeviceInfo
import androidx.media3.common.Player
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** The LMS → Media3 state mapping (pure; runs on the JVM). */
class SessionStateTest {
    private fun map(
        local: LocalPlayerState = running(),
        status: io.github.jamezuk.lyrionanywhere.core.PlayerStatus? = lmsStatus(),
        queue: List<io.github.jamezuk.lyrionanywhere.core.Track> = QUEUE,
    ) = mapSnapshot(SessionSnapshot(local = local, status = status, queue = queue))

    @Test
    fun withTheServerAwayOnlyWhatThePhoneCanDoIsOffered() {
        val online = map()
        assertTrue(online.canPlayPause && online.canNext && online.canPrevious && online.canJump)
        val s = map(local = away(offline(index = 2, playing = false, next = true, previous = false, seek = true)))
        assertTrue(s.canPlayPause && s.canNext && s.canSeek)
        assertFalse("previous isn't on the phone", s.canPrevious)
        assertFalse("jumping from the list needs the server", s.canJump)
        // What the phone does: paused at its position, on the entry it plays.
        assertFalse(s.playWhenReady || s.advancing)
        assertEquals(30_000, s.positionMs)
        assertEquals(2, s.items[s.currentIndex].queueIndex)
        // Nothing playable: nothing offered.
        val none = map(local = away(offline(playPause = false)))
        assertFalse(none.canPlayPause || none.canNext || none.canPrevious || none.canSeek)
    }

    @Test
    fun idleUnlessThePhoneIsAPlayerAndLmsHasSpoken() {
        assertEquals(Player.STATE_IDLE, map(local = LocalPlayerState()).playbackState)
        assertEquals(Player.STATE_IDLE, map(status = null).playbackState)
        val stopped = LocalPlayerState(enabled = true, status = localStatus(connection = PlayerConnection.Stopped("x")))
        val s = map(local = stopped)
        assertEquals(Player.STATE_IDLE, s.playbackState)
        assertTrue(s.items.isEmpty())
        assertFalse(s.playWhenReady)
    }

    @Test
    fun playingPausedAndBuffering() {
        val playing = map()
        assertEquals(Player.STATE_READY, playing.playbackState)
        assertTrue(playing.playWhenReady && playing.advancing)
        assertEquals(12_500, playing.positionMs)

        val paused = map(status = lmsStatus(mode = PlaybackMode.PAUSED))
        assertEquals(Player.STATE_READY, paused.playbackState)
        assertFalse(paused.playWhenReady || paused.advancing)

        // LMS says play, but the engine is still filling its buffer.
        val buffering = map(local = running(LocalPlayback.BUFFERING))
        assertEquals(Player.STATE_BUFFERING, buffering.playbackState)
        assertTrue(buffering.playWhenReady)
        assertFalse(buffering.advancing)

        val stopped = map(status = lmsStatus(mode = PlaybackMode.STOPPED))
        assertEquals(Player.STATE_READY, stopped.playbackState)
        assertFalse(stopped.playWhenReady)
    }

    @Test
    fun theQueueIsLmsPlaylistWithTheCurrentTrackSelected() {
        val s = map()
        assertEquals(listOf("0-10", "1-11", "2-12"), s.items.map { it.uid })
        assertEquals(1, s.currentIndex)
        assertEquals("Track 11", s.current?.title)
        assertEquals(200_000L, s.current?.durationMs, "the status's duration wins for the current track")
        assertEquals(180_000L, s.items[0].durationMs)
    }

    @Test
    fun aCurrentTrackOutsideTheLoadedQueueStillShows() {
        val s = map(status = lmsStatus(index = 250, current = track(99, 250)), queue = QUEUE)
        assertEquals(1, s.items.size)
        assertEquals("Track 99", s.current?.title)
        assertEquals(250, s.current?.queueIndex)
        val empty = map(status = lmsStatus(current = null), queue = emptyList())
        assertEquals(Player.STATE_IDLE, empty.playbackState)
    }

    @Test
    fun seekingFollowsLms() {
        assertTrue(map().canSeek)
        assertFalse(map(status = lmsStatus(canSeek = false)).canSeek)
    }

    @Test
    fun negativePositionsAreClamped() {
        assertEquals(0, map(status = lmsStatus(elapsed = -5.0)).positionMs)
    }

    private fun assertEquals(expected: Long?, actual: Long?, message: String) =
        org.junit.Assert.assertEquals(message, expected, actual)
}

class AudioFocusPolicyTest {
    @Test
    fun transientLossPausesAndResumes() {
        val p = AudioFocusPolicy()
        assertEquals(listOf(FocusAction.Pause), p.onChange(FocusChange.LOSS_TRANSIENT, playing = true))
        assertTrue(p.resumeOnGain)
        assertEquals(listOf(FocusAction.Resume), p.onChange(FocusChange.GAIN, playing = false))
        assertFalse(p.resumeOnGain)
    }

    @Test
    fun permanentLossNeverResumes() {
        val p = AudioFocusPolicy()
        assertEquals(listOf(FocusAction.Pause), p.onChange(FocusChange.LOSS, playing = true))
        assertEquals(emptyList<FocusAction>(), p.onChange(FocusChange.GAIN, playing = false))
    }

    @Test
    fun aTransientLossWhilePausedDoesNotStartPlayback() {
        val p = AudioFocusPolicy()
        assertEquals(emptyList<FocusAction>(), p.onChange(FocusChange.LOSS_TRANSIENT, playing = false))
        assertEquals(emptyList<FocusAction>(), p.onChange(FocusChange.GAIN, playing = false))
    }

    @Test
    fun duckingLowersAndRestores() {
        val p = AudioFocusPolicy()
        assertEquals(listOf(FocusAction.Duck(true)), p.onChange(FocusChange.LOSS_TRANSIENT_CAN_DUCK, true))
        assertEquals(emptyList<FocusAction>(), p.onChange(FocusChange.LOSS_TRANSIENT_CAN_DUCK, true))
        assertEquals(listOf(FocusAction.Duck(false)), p.onChange(FocusChange.GAIN, true))
        // Ducked, then lost for good: restore the volume and pause.
        p.onChange(FocusChange.LOSS_TRANSIENT_CAN_DUCK, true)
        assertEquals(listOf(FocusAction.Duck(false), FocusAction.Pause), p.onChange(FocusChange.LOSS, true))
    }

    @Test
    fun aUserPauseCancelsTheResume() {
        val p = AudioFocusPolicy()
        p.onChange(FocusChange.LOSS_TRANSIENT, playing = true)
        p.userPaused()
        assertEquals(emptyList<FocusAction>(), p.onChange(FocusChange.GAIN, playing = false))
    }
}

class RouteTest {
    @Test
    fun externalDevicesWinOverTheSpeaker() {
        assertEquals(RouteKind.USB, routeKind(AudioDeviceInfo.TYPE_USB_HEADSET))
        assertEquals(RouteKind.WIRED, routeKind(AudioDeviceInfo.TYPE_WIRED_HEADPHONES))
        assertEquals(RouteKind.BLUETOOTH, routeKind(AudioDeviceInfo.TYPE_BLUETOOTH_A2DP))
        assertEquals(RouteKind.SPEAKER, routeKind(AudioDeviceInfo.TYPE_BUILTIN_SPEAKER))
        assertEquals(RouteKind.OTHER, routeKind(AudioDeviceInfo.TYPE_TELEPHONY))
        val order = listOf(
            AudioDeviceInfo.TYPE_BUILTIN_SPEAKER,
            AudioDeviceInfo.TYPE_BLUETOOTH_A2DP,
            AudioDeviceInfo.TYPE_WIRED_HEADSET,
            AudioDeviceInfo.TYPE_USB_DEVICE,
        )
        assertEquals(order, order.shuffled(java.util.Random(1)).sortedBy(::routePriority))
    }
}
