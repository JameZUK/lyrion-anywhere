package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.remember
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.isDialog
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueDrag
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEntry
import io.github.jamezuk.lyrionanywhere.ui.player.QueueHeader
import io.github.jamezuk.lyrionanywhere.ui.player.QueueSections
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.ShuffleButton
import io.github.jamezuk.lyrionanywhere.ui.player.UpNext
import io.github.jamezuk.lyrionanywhere.ui.player.queueItems
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Wave 2 §3.4–3.5 in the player view model: Up next tracked for this device's Play next,
 * dropped when the queue changes elsewhere; play next from the queue; Clear upcoming (from
 * the end, each entry checked, stopping if the queue changes); the shuffle modes.
 */
class Wave2QueueViewModelTest : MainDispatcherTest() {
    private val me = "aa:00:00:00:00:02"
    private val vms = mutableListOf<PlayerViewModel>()

    private fun t(id: Long, at: Int) = sampleTrack(id, at)

    /** A queue of [size] tracks (ids 1…), playing [current]. */
    private fun setup(size: Int = 5, current: Int = 1): Pair<PlayerViewModel, FakePlayers> {
        val players = FakePlayers()
        players.queueTracks = (0 until size).map { t(it + 1L, it) }
        players.statusResult = ApiResult.Ok(sampleStatus().copy(queueIndex = current.toUInt(), queueLength = size.toUInt(), current = t(current + 1L, current)))
        val settings = FakeSettings()
        settings.state.update { it.copy(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = me) }
        val vm = PlayerViewModel(players, FakeEvents(), settings) { 0L }
        vms += vm
        vm.start()
        vm.loadQueue()
        return vm to players
    }

    private fun vmTest(body: suspend TestScope.() -> Unit) = runTest(dispatcher) {
        try {
            body()
        } finally {
            vms.forEach { it.close() }
        }
    }

    /** LMS's insert: [ids] go in right after the current track. */
    private fun FakePlayers.insertsAfterCurrent(vararg ids: Long) {
        onEnqueue = { _, placement ->
            if (placement == Placement.PLAY_NEXT) {
                val s = (statusResult as ApiResult.Ok).value
                val cur = s.queueIndex!!.toInt()
                val q = queueTracks.toMutableList()
                q.addAll(cur + 1, ids.map { sampleTrack(it) })
                queueTracks = q
                statusResult = ApiResult.Ok(s.copy(queueLength = q.size.toUInt()))
            }
        }
    }

    @Test
    fun playNextFromThisDeviceBecomesUpNext() = vmTest {
        val (vm, players) = setup()
        players.insertsAfterCurrent(50, 51)
        vm.enqueue(Selection.Album(7), Placement.PLAY_NEXT)
        assertEquals(UpNext(1, listOf(50L, 51L)), vm.state.value.upNext)
        vm.loadQueue()
        val sections = vm.state.value.sections
        assertEquals(QueueSections(1, 2..3), sections)
        // A later Play next goes in front of them, as LMS's insert does.
        players.insertsAfterCurrent(52)
        vm.enqueue(Selection.Track(52), Placement.PLAY_NEXT)
        assertEquals(UpNext(1, listOf(52L, 50L, 51L)), vm.state.value.upNext)
        // Play now starts a new queue: nothing is up next any more.
        vm.enqueue(Selection.Album(8), Placement.PLAY_NOW)
        assertNull(vm.state.value.upNext)
    }

    @Test
    fun aQueueChangedElsewhereShowsFlat() = vmTest {
        val (vm, players) = setup()
        players.insertsAfterCurrent(50, 51)
        vm.enqueue(Selection.Album(7), Placement.PLAY_NEXT)
        vm.loadQueue()
        assertNotNull(vm.state.value.sections)
        // Another controller puts a track in front of them.
        players.queueTracks = players.queueTracks.toMutableList().apply { add(2, sampleTrack(99)) }
        vm.loadQueue()
        assertNull(vm.state.value.upNext)
        assertNull(vm.state.value.sections)
    }

    @Test
    fun playingOnUsesUpUpNext() = vmTest {
        val (vm, players) = setup()
        players.insertsAfterCurrent(50, 51)
        vm.enqueue(Selection.Album(7), Placement.PLAY_NEXT)
        val s = (players.statusResult as ApiResult.Ok).value
        players.statusResult = ApiResult.Ok(s.copy(queueIndex = 2u))
        vm.refresh()
        assertEquals(UpNext(2, listOf(51L)), vm.state.value.upNext)
    }

    @Test
    fun aRemovalInsideUpNextShrinksIt() = vmTest {
        val (vm, players) = setup()
        players.liveQueue = true
        players.insertsAfterCurrent(50, 51)
        vm.enqueue(Selection.Album(7), Placement.PLAY_NEXT)
        vm.loadQueue()
        vm.removeFromQueue(2)
        assertTrue(players.commands.contains("remove 2 $me"))
        assertEquals(UpNext(1, listOf(51L)), vm.state.value.upNext)
    }

    @Test
    fun playNextFromTheQueueMovesItAfterTheCurrentTrack() = vmTest {
        val (vm, players) = setup()
        players.liveQueue = true
        vm.playNextInQueue(4)
        assertEquals("move 4 2 $me", players.commands.last { it.startsWith("move") })
        // Shown at once: the track is next, and up next.
        assertEquals(5L, vm.state.value.queue[2].track.id)
        assertEquals(UpNext(1, listOf(5L)), vm.state.value.upNext)
        // A played track: after the current one, which moves up a place.
        vm.playNextInQueue(0)
        assertEquals("move 0 1 $me", players.commands.last { it.startsWith("move") })
        // The current track, or the one already next: nothing sent.
        val sent = players.commands.size
        val cur = vm.state.value.status!!.queueIndex!!.toInt()
        vm.playNextInQueue(cur)
        vm.playNextInQueue(cur + 1)
        assertEquals(sent, players.commands.size)
    }

    @Test
    fun clearUpcomingDeletesFromTheEndCheckingEachEntry() = vmTest {
        val (vm, players) = setup(size = 5, current = 1)
        players.liveQueue = true
        vm.clearUpcoming()
        assertEquals(listOf("remove 4 $me", "remove 3 $me", "remove 2 $me"), players.commands.filter { it.startsWith("remove") })
        // Each one was read just before it was taken out.
        assertTrue(players.queueCalls.containsAll(listOf(4 to 1, 3 to 1, 2 to 1)))
        assertEquals(2, players.queueTracks.size)
    }

    @Test
    fun clearUpcomingStopsWhenTheQueueChangesPartWay() = vmTest {
        val (vm, players) = setup(size = 6, current = 1)
        val messages = mutableListOf<PlayerMessage>()
        val c = backgroundScope.launch { vm.messages.collect { messages += it } }
        players.onRemove = { index ->
            // After the first removal, someone else swaps the track before it.
            if (index == 5) players.queueTracks = players.queueTracks.toMutableList().apply { this[4] = sampleTrack(77) }
        }
        vm.clearUpcoming()
        assertEquals(listOf("remove 5 $me"), players.commands.filter { it.startsWith("remove") })
        assertTrue(messages.contains(PlayerMessage.ClearStopped))
        c.cancel()
    }

    @Test
    fun clearUpcomingRefusesALongQueueAndAPlayerThatMovedOn() = vmTest {
        val (vm, players) = setup(size = PlayerViewModel.CLEAR_UPCOMING_MAX + 3, current = 1)
        vm.clearUpcoming()
        assertTrue(players.commands.none { it.startsWith("remove") })

        val (vm2, players2) = setup(size = 5, current = 1)
        // The player has moved on since the queue was shown.
        val s = (players2.statusResult as ApiResult.Ok).value
        players2.statusResult = ApiResult.Ok(s.copy(queueIndex = 2u))
        vm2.clearUpcoming()
        assertTrue(players2.commands.none { it.startsWith("remove") })
    }

    @Test
    fun shuffleCyclesOffTracksAlbumsAndCanBeChosen() = vmTest {
        val (vm, players) = setup()
        assertEquals(ShuffleMode.SONGS, PlayerViewModel.nextShuffle(ShuffleMode.OFF))
        assertEquals(ShuffleMode.ALBUMS, PlayerViewModel.nextShuffle(ShuffleMode.SONGS))
        assertEquals(ShuffleMode.OFF, PlayerViewModel.nextShuffle(ShuffleMode.ALBUMS))
        vm.cycleShuffle()
        assertEquals("shuffle SONGS $me", players.commands.last { it.startsWith("shuffle") })
        vm.setShuffle(ShuffleMode.ALBUMS)
        assertEquals("shuffle ALBUMS $me", players.commands.last { it.startsWith("shuffle") })
    }
}

/** The queue's sections, swipes, menus and markers, and the shuffle button, on screen. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class Wave2QueueScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val playNext = mutableListOf<Int>()
    private val removed = mutableListOf<Int>()

    /** Tracks 1 and 2 are tracks 1 and 2 of album 7: they play straight on. */
    private fun entries(): List<QueueEntry> = listOf(
        QueueEntry(1, Track(1, "Track 1", "Artist", "Album", 7, 3, 61.0, 1u, null, 0u, null, null, null, null, null)),
        QueueEntry(2, Track(2, "Track 2", "Artist", "Album", 7, 3, 61.0, 2u, null, 1u, null, null, null, null, null)),
        QueueEntry(3, sampleTrack(30, 2)),
        QueueEntry(4, sampleTrack(40, 3)),
    )

    private fun queue(sections: QueueSections? = null) = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            val drag = remember { QueueDrag() }
            LazyColumn {
                queueItems(
                    entries(),
                    currentIndex = 0,
                    drag = drag,
                    actions = QueueActions(onJump = {}, onRemove = { removed += it }, onPlayNext = { playNext += it }),
                    sections = sections,
                )
            }
        }
    }

    @Test
    fun sectionsShowNowPlayingUpNextAndThen() {
        queue(QueueSections(0, 1..2))
        compose.onNodeWithTag("section-now").assertIsDisplayed()
        compose.onNodeWithTag("section-up-next").assertIsDisplayed()
        compose.onNodeWithTag("section-then").assertIsDisplayed()
    }

    @Test
    fun aFlatQueueHasNoSections() {
        queue(null)
        compose.onNodeWithTag("section-now").assertDoesNotExist()
        compose.onNodeWithTag("section-up-next").assertDoesNotExist()
    }

    @Test
    fun gaplessTracksAreMarkedWithALabel() {
        queue()
        compose.onNodeWithTag("gapless-1")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Plays straight on from the track before")))
        compose.onNodeWithTag("gapless-2").assertDoesNotExist()
    }

    @Test
    fun swipingRightPlaysNextAndLeftRemoves() {
        queue()
        compose.onNodeWithTag("queue-row-3").performTouchInput { swipeRight() }
        compose.waitForIdle()
        assertEquals(listOf(3), playNext)
        // The track already next can't be moved there.
        compose.onNodeWithTag("queue-row-1").performTouchInput { swipeRight() }
        compose.waitForIdle()
        assertEquals(listOf(3), playNext)
        compose.onNodeWithTag("queue-row-2").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(2), removed)
    }

    @Test
    fun theRowMenuAndTalkBackOfferPlayNext() {
        queue()
        compose.onNodeWithContentDescription("More options for Track 30").performClick()
        compose.onNodeWithText("Play next").performClick()
        assertEquals(listOf(2), playNext)
        val actions = compose.onNodeWithTag("queue-track-3").fetchSemanticsNode().config[SemanticsActions.CustomActions]
        assertEquals(listOf("Play next", "Move up", "Remove from queue"), actions.map { it.label })
        actions.first { it.label == "Play next" }.action()
        assertEquals(listOf(2, 3), playNext)
    }

    @Test
    fun clearUpcomingAsksAndOffersClearAllForALongQueue() {
        var upcoming = 0
        var all = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) { QueueHeader(5, "Kitchen", onClear = { all++ }, upcoming = 3, onClearUpcoming = { upcoming++ }) }
        }
        compose.onNodeWithText("Clear upcoming").performClick()
        compose.onNodeWithText("This removes the 3 tracks after the current one from the queue of Kitchen.").assertIsDisplayed()
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(0, upcoming)
        compose.onNodeWithText("Clear upcoming").performClick()
        compose.onNodeWithTag("clear-upcoming").assertIsDisplayed()
        compose.onNodeWithText("Clear upcoming tracks?").assertIsDisplayed()
        // The dialog's own button (the header's has the same words).
        compose.onNode(hasText("Clear upcoming") and hasAnyAncestor(isDialog())).performClick()
        assertEquals(1, upcoming)
        assertEquals(0, all)
    }

    @Test
    fun aLongQueueClearsAllInstead() {
        var upcoming = 0
        var all = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) { QueueHeader(300, "Kitchen", onClear = { all++ }, upcoming = 250, onClearUpcoming = { upcoming++ }) }
        }
        compose.onNodeWithText("Clear upcoming").performClick()
        compose.onNodeWithText("Clear all").performClick()
        assertEquals(0, upcoming)
        assertEquals(1, all)
    }

    @Test
    fun theShuffleButtonCyclesSaysWhichAndOffersAllThree() {
        var cycled = 0
        val chosen = mutableListOf<ShuffleMode>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) { ShuffleButton(ShuffleMode.ALBUMS, onCycle = { cycled++ }, onChoose = { chosen += it }) }
        }
        compose.onNodeWithTag("shuffle-button")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Shuffle")))
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Shuffling albums"))
        compose.onNodeWithTag("shuffle-albums-mark", useUnmergedTree = true).assertExists()
        compose.onNodeWithTag("shuffle-button").performClick()
        assertEquals(1, cycled)
        // Long-press (TalkBack's long-press action too) offers the three.
        val longClick = compose.onNodeWithTag("shuffle-button").fetchSemanticsNode().config[SemanticsActions.OnLongClick]
        assertEquals("Choose how to shuffle", longClick.label)
        compose.onNodeWithTag("shuffle-button").performTouchInput { longClick() }
        compose.onNodeWithText("Don't shuffle").assertIsDisplayed()
        compose.onNodeWithText("Shuffle tracks").performClick()
        assertEquals(listOf(ShuffleMode.SONGS), chosen)
    }

    @Test
    fun theQueuePeekOpensFromItsButtonAndTheArtworksAction() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(auditPlayerState().copy(upNext = UpNext(0, listOf(2L))), clock = { 0L }, actions = nowPlayingActions())
            }
        }
        compose.onNodeWithTag("queue-peek").assertDoesNotExist()
        compose.onNodeWithTag("queue-peek-button").performClick()
        compose.onNodeWithTag("queue-peek").assertExists()
    }

    @Test
    fun theArtworkOffersTheQueuePeekToTalkBack() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) { NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions()) }
        }
        val actions = compose.onNodeWithTag("artwork").fetchSemanticsNode().config[SemanticsActions.CustomActions]
        val peek = actions.first { it.label == "Show the queue" }
        compose.runOnUiThread { peek.action() }
        compose.waitForIdle()
        compose.onNodeWithTag("queue-peek").assertExists()
    }

    @Test
    fun swipingUpOnTheArtworkOpensTheQueuePeek() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) { NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions()) }
        }
        compose.onNodeWithTag("artwork").performTouchInput { swipeUp(startY = bottom - 10f, endY = top + 10f) }
        compose.waitForIdle()
        compose.onNodeWithTag("queue-peek").assertExists()
    }
}
