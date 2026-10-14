package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsPlayer
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi
import io.github.jamezuk.lyrionanywhere.ui.player.ControlGate
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreen
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreenActions
import io.github.jamezuk.lyrionanywhere.ui.player.Room
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsActions
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsScreen
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val KITCHEN = "aa:00:00:00:00:01"
private const val LOUNGE = "aa:00:00:00:00:02"

private fun downloads(index: Int = 0, shuffle: Boolean = false, repeat: LocalRepeat = LocalRepeat.OFF, playback: LocalPlayback = LocalPlayback.PLAYING) =
    OfflinePlayerUi(
        OfflinePlayerState(
            status = dlStatus(index = index, keys = listOf(dlTrack(1).key, dlTrack(2).key, dlTrack(3).key), shuffle = shuffle, repeat = repeat, playback = playback),
            tracks = listOf(dlTrack(1), dlTrack(2), dlTrack(3)).associateBy { it.key },
        ),
        at = 0,
    )

/** Wave 3.5 phase 3: Rooms folded into Play on…, and downloads with the main player screens. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h1800dp")
class Wave35Phase3Tests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private val calls = mutableListOf<String>()

    private fun roomActions() = RoomsActions(
        onControl = { calls += "control $it" },
        onTogglePlay = { calls += "play $it" },
        onVolume = { id, v -> calls += "volume $id $v" },
        onPower = { id, on -> calls += "power $id $on" },
        onSync = { a, b -> calls += "sync $a $b" },
        onUnsync = { calls += "unsync $it" },
        onAlarms = { id, _ -> calls += "alarms $id" },
    )

    private fun rooms() = listOf(
        Room(samplePlayer(KITCHEN, playing = true), sampleStatus()),
        Room(samplePlayer(LOUNGE), sampleStatus(mode = PlaybackMode.PAUSED)),
    )

    // Item 13: Rooms folds into Play on…

    @Test
    fun playOnRowsDoWhatRoomsCardsDid() {
        // Lounge controlled; Kitchen playing; both with a volume, each row's ⋮ with Rooms' actions.
        show {
            PlayOnList(
                auditPlayerState(LOUNGE),
                PlayOnActions(onControl = { calls += "select $it" }, onBackToPhone = {}, onMoveHere = { calls += "move $it" }),
                rooms = rooms(),
                roomActions = roomActions(),
            )
        }
        compose.onNodeWithTag("play-on-volume-$KITCHEN").assertIsDisplayed()
        compose.onNodeWithTag("play-on-volume-$LOUNGE").assertIsDisplayed()
        compose.onNodeWithTag("play-on-$KITCHEN-more").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-play").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-more").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-alarms").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-more").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-power").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-more").performClick()
        compose.onNodeWithTag("play-on-$KITCHEN-move").performClick()
        compose.onNode(hasText("Player $KITCHEN") and hasClickAction()).performClick()
        assertEquals(listOf("play $KITCHEN", "alarms $KITCHEN", "power $KITCHEN false", "move $KITCHEN", "select $KITCHEN"), calls)
        // The expanded view is the caller's to offer (large windows only).
        compose.onNodeWithTag("all-rooms").assertDoesNotExist()
    }

    @Test
    fun anIdlePlayerHasNoVolumeSlider() {
        show {
            PlayOnList(
                auditPlayerState(KITCHEN),
                PlayOnActions(onControl = {}, onBackToPhone = {}),
                rooms = rooms(),
                roomActions = roomActions(),
            )
        }
        compose.onNodeWithTag("play-on-volume-$KITCHEN").assertIsDisplayed()
        compose.onNodeWithTag("play-on-volume-$LOUNGE").assertDoesNotExist()
    }

    @Test
    fun aRoomsCardHasThreeControls() {
        show { RoomsScreen(RoomsUiState(rooms = rooms(), loading = false), roomActions(), selectedId = LOUNGE) }
        val card = compose.onNodeWithTag("room-$KITCHEN").fetchSemanticsNode()
        val width = card.size.width
        fun walk(n: androidx.compose.ui.semantics.SemanticsNode): List<androidx.compose.ui.semantics.SemanticsNode> = n.children.flatMap { listOf(it) + walk(it) }
        val controls = walk(card).filter {
            (it.config.getOrNull(SemanticsActions.OnClick) != null && it.size.width < width / 2) ||
                it.config.getOrNull(SemanticsActions.SetProgress) != null
        }
        // Play/pause, the volume and ⋮; power is in ⋮, and there is no drag handle button.
        assertEquals(3, controls.size)
        compose.onNodeWithTag("room-power-$KITCHEN").assertDoesNotExist()
        compose.onNodeWithTag("room-menu-$KITCHEN").performClick()
        compose.onNodeWithTag("room-power-$KITCHEN").performClick()
        assertEquals(listOf("power $KITCHEN false"), calls)
    }

    // Item 14: downloads through the main screens.

    @Test
    fun theDownloadsQueueIsAPlayerOfItsOwn() {
        val base = auditPlayerState(KITCHEN).copy(connection = ConnectionUi(unreachable = true))
        val s = DownloadsPlayer.state(downloads(index = 1, shuffle = true, repeat = LocalRepeat.ONE), "Downloads", base)!!
        assertEquals(DownloadsPlayer.ID, s.selectedId)
        assertEquals("Downloads", s.selected?.name)
        assertTrue(s.downloads)
        assertTrue(s.localSelected)
        assertTrue(!s.controllingOther)
        // Every control works whatever the server: the engine is on the phone.
        assertEquals(ControlGate.ONLINE, s.gate)
        val status = s.status!!
        assertEquals("Song 2", status.current?.title)
        assertEquals("Song 3", status.next?.title)
        assertEquals(1u, status.queueIndex)
        assertEquals(3u, status.queueLength)
        assertEquals(ShuffleMode.SONGS, status.shuffle)
        assertEquals(RepeatMode.ONE, status.repeat)
        assertEquals(PlaybackMode.PLAYING, status.mode)
        assertEquals(listOf("Song 1", "Song 2", "Song 3"), s.queue.map { it.track.title })
        assertTrue(s.queue.all { it.track.artwork?.isNotEmpty() == true })
        // The other players stay, for Play on…
        assertTrue(s.players.any { it.id == KITCHEN })
    }

    @Test
    fun nothingLoadedIsNoPlayer() {
        assertNull(DownloadsPlayer.state(OfflinePlayerUi(OfflinePlayerState()), "Downloads"))
        assertNull(DownloadsPlayer.state(downloads().copy(state = downloads().state.copy(status = dlStatus(index = null))), "Downloads"))
    }

    @Test
    fun downloadsUseTheMainNowPlayingWithStopAndNoLmsControls() {
        var stopped = 0
        var toggled = 0
        val s = DownloadsPlayer.state(downloads(), "Downloads")!!
        show {
            NowPlayingScreen(
                s,
                clock = { 0L },
                actions = nowPlayingActions().copy(onTogglePlay = { toggled++ }, onStop = { stopped++ }),
                phoneVolume = FakePhoneVolume(),
            )
        }
        compose.onNodeWithTag("now-playing").assertExists()
        compose.onNodeWithTag("play-on-button").assertIsDisplayed()
        compose.onNodeWithTag("rating-toggle").assertDoesNotExist()
        compose.onNodeWithTag("play-pause").performClick()
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("sleep-button").assertDoesNotExist()
        compose.onNodeWithTag("more-rate").assertDoesNotExist()
        compose.onNodeWithTag("volume-button").assertExists()
        compose.onNodeWithTag("offline-stop").performClick()
        assertEquals(listOf(1, 1), listOf(toggled, stopped))
    }

    @Test
    fun theMiniPlayerSaysDownloads() {
        show { MiniPlayer(DownloadsPlayer.state(downloads(), "Downloads")!!, onOpen = {}, onTogglePlay = {}, onNext = {}) }
        compose.onNodeWithText("Band · Downloads").assertIsDisplayed()
        compose.onNodeWithTag("mini-player-on", useUnmergedTree = true).assertDoesNotExist()
    }

    @Test
    fun downloadsAreInTheQueueTabAndARowPlaysFromIt() {
        val jumped = mutableListOf<Int>()
        val s = DownloadsPlayer.state(downloads(), "Downloads")!!
        show { QueueScreen(s, QueueScreenActions(DownloadsPlayer.queueActions({ jumped += it }, s.queue))) }
        compose.onNodeWithText("Song 3").performClick()
        assertEquals(listOf(2), jumped)
        // The downloads queue can't be edited: no row menus, no drag handles, the header's actions off.
        compose.onNodeWithTag("queue-drag-0").assertDoesNotExist()
        compose.onNodeWithText("More options for Song 1").assertDoesNotExist()
    }
}
