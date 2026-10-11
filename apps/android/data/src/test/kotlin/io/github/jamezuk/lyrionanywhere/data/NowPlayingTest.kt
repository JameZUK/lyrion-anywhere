package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"

class NowPlayingTest {
    private fun np(id: String, playing: Boolean, phone: Boolean = id == PHONE) =
        NowPlaying(id, "Player", "Track", "Artist", "music/1/cover", playing, phone)

    private fun status(mode: PlaybackMode, track: Track?) = PlayerStatus(
        "Kitchen", true, true, mode, 1.0, 100.0, 40u, false, ShuffleMode.OFF, RepeatMode.OFF, 0u, 1u, true, track,
    )

    private val track = Track(
        1, "One", "Artist", "Album", 7, 3, 61.0, 1u, "music/1/cover", 0u, null, null, null, null, null,
    )

    @Test
    fun fromAStatus() {
        val n = NowPlaying.from(KITCHEN, "", status(PlaybackMode.PLAYING, track), isPhone = false)!!
        assertEquals("Kitchen", n.playerName)
        assertEquals("One", n.title)
        assertEquals("music/1/cover", n.artwork)
        assertTrue(n.playing)
        val idle = NowPlaying.from(KITCHEN, "Kitchen", status(PlaybackMode.STOPPED, null), isPhone = false)!!
        assertEquals("", idle.title)
        assertEquals(false, idle.playing)
        assertNull(NowPlaying.from(KITCHEN, "Kitchen", null, isPhone = false))
    }

    @Test
    fun theWidgetShowsThePhoneWhilePlayingOtherwiseTheChosenPlayer() {
        val phonePlaying = np(PHONE, true)
        val phonePaused = np(PHONE, false)
        val kitchen = np(KITCHEN, false)
        assertEquals(phonePlaying, NowPlayingHub.pick(phonePlaying, kitchen))
        assertEquals(kitchen, NowPlayingHub.pick(phonePaused, kitchen))
        assertEquals(phonePaused, NowPlayingHub.pick(phonePaused, null))
        assertNull(NowPlayingHub.pick(null, null))
    }

    @Test
    fun aButtonPressShowsAtOnceAndSignOutForgets() = runTest {
        val hub = NowPlayingHub()
        hub.publishPhone(np(PHONE, false))
        hub.publishSelected(np(KITCHEN, false))
        hub.setPlaying(KITCHEN, true)
        assertEquals(true, hub.selected.value?.playing)
        assertEquals(false, hub.phone.value?.playing)
        assertEquals(KITCHEN, hub.shown.first()?.playerId)
        hub.setPlaying(PHONE, true)
        assertEquals(PHONE, hub.shown.first()?.playerId)
        hub.clear()
        assertNull(hub.shown.first())
    }
}
