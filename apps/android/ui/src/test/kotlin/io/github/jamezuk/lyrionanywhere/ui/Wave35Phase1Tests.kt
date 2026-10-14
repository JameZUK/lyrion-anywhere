package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.ComposeContentTestRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.offline.ContinueOnState
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumActions
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumDetail
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FavouritesScreen
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeEditActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeed
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.PlayButtons
import io.github.jamezuk.lyrionanywhere.ui.library.TrackRow
import io.github.jamezuk.lyrionanywhere.ui.offline.ContinueChoice
import io.github.jamezuk.lyrionanywhere.ui.offline.ContinueOnPrompt
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueHeader
import io.github.jamezuk.lyrionanywhere.ui.player.QueueMark
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreen
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreenActions
import io.github.jamezuk.lyrionanywhere.ui.player.shownMark
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.FavouritePage
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.player.ControlGate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"

/**
 * The controls a row carries besides its own tap: its clickable descendants narrower than
 * half the row (the row's body, which plays or opens it, is wider). Wave 3.5 rule 3 allows
 * one, besides a drag handle (which has no click of its own).
 */
internal fun SemanticsNode.trailingControls(): Int {
    val rowWidth = size.width
    fun walk(n: SemanticsNode): Int = n.children.sumOf { c ->
        val control = c.config.getOrNull(SemanticsActions.OnClick) != null && c.size.width < rowWidth / 2
        (if (control) 1 else 0) + walk(c)
    }
    return walk(this)
}

/** The row tagged [tag] carries at most one control of its own. */
internal fun ComposeContentTestRule.assertOneTrailingControl(tag: String) {
    val node = onNodeWithTag(tag, useUnmergedTree = true).fetchSemanticsNode()
    val n = node.trailingControls()
    assertTrue("$tag has $n controls of its own", n <= 1)
}

/** Wave 3.5 phase 1: the quick wins, and that every moved action is still reachable. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h914dp")
class Wave35Phase1Tests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    // Item 1: the "Playing on" strip is gone.

    @Test
    fun theMiniPlayerNamesTheOtherPlayerWithNoStrip() {
        show { MiniPlayer(auditPlayerState(KITCHEN), onOpen = {}, onTogglePlay = {}, onNext = {}) }
        compose.onNodeWithText("on Player $KITCHEN", substring = true).assertIsDisplayed()
        compose.onNodeWithTag("playing-on").assertDoesNotExist()
        compose.onNodeWithText("Back to this phone").assertDoesNotExist()
        compose.onNodeWithText("Bring to this phone").assertDoesNotExist()
    }

    @Test
    fun movingPlaybackHereAndBackToThePhoneAreInPlayOn() {
        var brought = 0
        var back = 0
        show { PlayOnList(auditPlayerState(KITCHEN), PlayOnActions(onControl = {}, onBackToPhone = { back++ }, onBringToPhone = { brought++ })) }
        compose.onNodeWithTag("play-on-phone-more").performClick()
        compose.onNodeWithTag("play-on-phone-move").assertTextContains("Move playback to this phone").performClick()
        compose.onNode(hasText("This phone") and hasClickAction()).performClick()
        assertEquals(listOf(1, 1), listOf(brought, back))
    }

    // Item 2: page headers.

    @Test
    fun aHeaderIsPlayShuffleDownloadAndMore() {
        var more = 0
        show {
            Box(Modifier.width(379.dp)) {
                PlayButtons(onPlay = {}, onShuffle = {}, onMore = { more++ }, onDownload = {})
            }
        }
        compose.onNodeWithTag("play-buttons-play").assertIsDisplayed()
        compose.onNodeWithTag("play-buttons-shuffle").assertIsDisplayed()
        compose.onNodeWithTag("play-buttons-download").assertIsDisplayed()
        compose.onNodeWithContentDescription("Add to queue").assertDoesNotExist()
        compose.onNodeWithContentDescription("More options").performClick()
        assertEquals(1, more)
    }

    @Test
    @Config(sdk = [35], qualifiers = "w411dp-h914dp", fontScale = 1.3f)
    fun aHeaderNeverWrapsWithLargeText() {
        show {
            Box(Modifier.width(379.dp)) {
                PlayButtons(onPlay = {}, onShuffle = {}, onMore = {}, onDownload = {})
            }
        }
        val y = { tag: String ->
            val n = compose.onNodeWithTag(tag).fetchSemanticsNode()
            n.positionInRoot.y + n.size.height / 2f
        }
        val more = compose.onNodeWithContentDescription("More options").fetchSemanticsNode()
        val moreY = more.positionInRoot.y + more.size.height / 2f
        assertEquals(y("play-buttons-play"), y("play-buttons-shuffle"), 2f)
        assertEquals(y("play-buttons-play"), moreY, 2f)
    }

    // Item 3: the queue.

    @Test
    fun theQueueHeaderIsOneLineAndItsActionsAreInItsMenu() {
        show { Box(Modifier.width(411.dp)) { QueueHeader(5, "Kitchen", onClear = {}, upcoming = 3, onSaveAsPlaylist = {}) } }
        compose.onNodeWithText("5 tracks on Kitchen").assertIsDisplayed()
        listOf("save-queue", "clear-upcoming", "clear-queue").forEach { compose.onNodeWithTag(it).assertDoesNotExist() }
        compose.onNodeWithTag("queue-actions").performClick()
        listOf("save-queue", "clear-upcoming", "clear-queue").forEach { compose.onNodeWithTag(it).assertIsDisplayed() }
    }

    @Test
    fun aQueueRowHasOneControlBesidesItsHandleAndNoGaplessMarks() {
        val state = auditPlayerState(KITCHEN).copy(downloaded = setOf(2L))
        show { QueueScreen(state, QueueScreenActions(QueueActions(onJump = {}))) }
        listOf(0, 1, 2).forEach { compose.assertOneTrailingControl("queue-row-$it") }
        compose.onNodeWithTag("gapless-1").assertDoesNotExist()
        compose.onNodeWithTag("queue-mark-1", useUnmergedTree = true).assertExists()
    }

    // Item 4: Now playing.

    @Test
    fun nowPlayingHasOneRatingControlAndThreeActions() {
        show { NowPlayingScreen(busyState(), clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("rating-toggle").assertIsDisplayed()
        compose.onNodeWithTag("rating-stars").assertDoesNotExist()
        listOf("play-on-button", "lyrics-action", "queue-button").forEach { compose.onNodeWithTag(it).assertIsDisplayed() }
        val row = compose.onNodeWithTag("now-playing-actions").fetchSemanticsNode()
        assertEquals(3, row.children.size)
        compose.onNodeWithTag("up-next-remove").assertDoesNotExist()
        compose.onNodeWithTag("playing-on-chip").assertDoesNotExist()
    }

    @Test
    fun theFullRatingIsInTheMenuAndOnTheStarsLongPress() {
        val rated = mutableListOf<Int>()
        val state = busyState().copy(rating = TrackRating(1, 40u, 40u, 40u), ratingStyle = RatingStyle.STARS)
        show { NowPlayingScreen(state, clock = { 0L }, actions = nowPlayingActions().copy(onRate = { rated += it })) }
        compose.onNodeWithTag("rating-toggle").performClick()
        compose.onNodeWithTag("rate-sheet").assertExists()
        compose.onNodeWithTag("rating-star-4").performClick()
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("more-rate").assertExists()
        compose.onNodeWithTag("rating-star-5").performClick()
        assertEquals(listOf(80, 100), rated)
    }

    @Test
    fun theHeartTogglesAndALongPressRatesInFull() {
        val rated = mutableListOf<Int>()
        val state = busyState().copy(rating = TrackRating(1, null, null, null), ratingStyle = RatingStyle.HEART)
        show { NowPlayingScreen(state, clock = { 0L }, actions = nowPlayingActions().copy(onRate = { rated += it })) }
        compose.onNodeWithTag("rating-heart").performClick()
        compose.onNodeWithTag("rating-heart").performTouchInput { longClick() }
        compose.onNodeWithTag("rate-sheet").assertExists()
        compose.onNodeWithTag("rating-star-3").performClick()
        assertEquals(listOf(100, 60), rated)
    }

    @Test
    fun everyActionTakenOffNowPlayingIsInItsMenu() {
        val opened = mutableListOf<String>()
        show {
            NowPlayingScreen(
                busyState(PHONE),
                clock = { 0L },
                actions = nowPlayingActions().copy(onCarMode = { opened += "car" }, onAlarms = { opened += "alarms" }, onMore = { opened += "track" }),
                visual = FakeVisualSource(),
                phoneVolume = FakePhoneVolume(),
            )
        }
        compose.onNodeWithTag("now-playing-more").performClick()
        // Sleep, volume and the visualiser (were buttons), the full rating (were five stars).
        for (tag in listOf("sleep-button", "volume-button", "visualiser-button", "more-visualiser-style", "more-rate", "more-track", "more-car-mode", "more-alarms", "more-power")) {
            compose.onNodeWithTag(tag).performScrollTo().assertExists()
        }
        compose.onNodeWithTag("sleep-button").performClick()
        compose.onNodeWithTag("sleep-sheet").assertExists()
    }

    @Test
    fun upNextCanStillBeRemovedAndTheQueueOpened() {
        var removed = 0
        show { NowPlayingScreen(busyState(), clock = { 0L }, actions = nowPlayingActions().copy(onRemoveUpNext = { removed++ })) }
        val node = compose.onNodeWithContentDescription("Up next: Next song — Artist").fetchSemanticsNode()
        compose.runOnUiThread { node.config[SemanticsActions.CustomActions].single { it.label == "Remove from the queue" }.action() }
        assertEquals(1, removed)
        compose.onNodeWithContentDescription("Up next: Next song — Artist").performClick()
        compose.onNodeWithTag("queue-peek").assertExists()
    }

    @Test
    fun gaplessIsInTrackInfo() {
        show { NowPlayingScreen(busyState(prefs = NowPlayingPrefs(layout = io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout.INFO)), clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithText("Gapless").assertExists()
        compose.onNodeWithText("Plays straight on into the next track").assertExists()
    }

    @Test
    fun allRoomsIsInPlayOnNotInNowPlayingsMenu() {
        show { NowPlayingScreen(busyState(), clock = { 0L }, actions = nowPlayingActions().copy(onRooms = {})) }
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("more-rooms").assertDoesNotExist()
    }

    // Item 5: Home.

    @Test
    fun stationsHaveNoBinAndAreDeletedFromTheirMenu() {
        val deleted = mutableListOf<String>()
        val feed = HomeFeed(stations = Loadable.Ready(FakeMe().stations), layout = HomeLayouts.DEFAULT, layoutLoaded = true)
        show { HomeScreen(feed, HomeActions({}, {}, {}, {}, {}, {}, {}).copy(onDeleteStation = { deleted += it.name }), edit = homeEdit()) }
        val first = FakeMe().stations.first().name
        compose.onNodeWithContentDescription("Delete station", substring = true).assertDoesNotExist()
        compose.onNodeWithContentDescription("More options for $first").performClick()
        compose.onNodeWithTag("station-delete").performClick()
        compose.onNodeWithText("Delete").performClick()
        assertEquals(listOf(first), deleted)
        // TalkBack has it on the card too.
        val card = compose.onNode(hasText(first) and hasClickAction()).fetchSemanticsNode()
        assertTrue(card.config[SemanticsActions.CustomActions].any { it.label == "Delete station $first" })
        // Edit Home is a quiet text button.
        compose.onNodeWithTag("home-edit").performScrollTo().assertIsDisplayed()
    }

    private fun homeEdit() = HomeEditActions({ _, _ -> }, { _, _ -> }, {}, {}, {}, {}, {})

    // Item 6: Continue on…

    @Test
    fun continueOnIsOneLine() {
        val choices = listOf(ContinueChoice(null, "This phone"), ContinueChoice(KITCHEN, "Kitchen"))
        show {
            Column {
                ContinueOnPrompt(ContinueOnState(tracks = 12), choices, choices[1], {}, {}, {})
            }
        }
        val node = compose.onNodeWithTag("continue-on").fetchSemanticsNode()
        val heightDp = node.size.height / node.layoutInfo.density.density
        assertTrue("the banner is $heightDp dp tall", heightDp <= 72f)
        compose.onNodeWithTag("continue-on-choose").assertIsDisplayed()
        compose.onNodeWithContentDescription("Not now").assertIsDisplayed()
    }

    // Item 7: marks.

    @Test
    fun aDownloadedTrackHasTheOneOfflineMark() {
        show {
            Column {
                TrackRow(sampleTrack(1), onClick = {}, onMore = {}, downloaded = true, modifier = Modifier.testTag("row-1"))
                TrackRow(sampleTrack(2), onClick = {}, onMore = {}, modifier = Modifier.testTag("row-2"))
            }
        }
        compose.onNodeWithTag("offline-mark", useUnmergedTree = true).assertExists()
        compose.onNodeWithContentDescription("Downloaded", useUnmergedTree = true).assertExists()
        compose.assertOneTrailingControl("row-1")
    }

    @Test
    fun albumTracksShowTheOfflineMark() {
        show {
            AlbumScreen(
                Loadable.Ready(AlbumDetail(sampleAlbum(7), listOf(sampleTrack(1), sampleTrack(2)))),
                AlbumActions({}, { _, _ -> }, {}, {}, { _, _ -> }, {}, downloaded = setOf(1L)),
            )
        }
        compose.onNodeWithTag("offline-mark", useUnmergedTree = true).performScrollTo().assertExists()
    }

    @Test
    fun theCacheMarkShowsOnlyWhileOutOfReach() {
        assertEquals(QueueMark.NONE, shownMark(QueueMark.ON_DEVICE, ControlGate.ONLINE))
        assertEquals(QueueMark.DOWNLOADED, shownMark(QueueMark.DOWNLOADED, ControlGate.ONLINE))
        assertEquals(QueueMark.ON_DEVICE, shownMark(QueueMark.ON_DEVICE, ControlGate.NOTHING))
    }

    @Test
    fun favouritesRowsHaveOneControlWithPlayNextAndAdd() {
        val placed = mutableListOf<Placement>()
        val page = FavouritePage(null, 1u, listOf(Favourite("f1", "Radio One", null, false)))
        show { FavouritesScreen(Loadable.Ready(page), onRetry = {}, onFolder = {}, onPlay = { _, p -> placed += p }) }
        compose.onNodeWithContentDescription("More options for Radio One").performClick()
        compose.onNodeWithTag("placement-next").performClick()
        compose.onNodeWithContentDescription("More options for Radio One").performClick()
        compose.onNodeWithTag("placement-add").performClick()
        assertEquals(listOf(Placement.PLAY_NEXT, Placement.ADD_TO_QUEUE), placed)
    }
}

/** B3: on a small phone (360×640) the cover stays a usable size. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w360dp-h640dp")
class Wave35SmallPhoneTests {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theCoverStaysUsable() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                // Without the tabs, Now playing keeps clear of the gesture bar itself.
                Box(Modifier.padding(bottom = 48.dp)) {
                    NowPlayingScreen(busyState(), clock = { 0L }, actions = nowPlayingActions(onClose = {}))
                }
            }
        }
        val cover = compose.onNodeWithTag("artwork").fetchSemanticsNode()
        val sideDp = cover.size.height / cover.layoutInfo.density.density
        assertTrue("the cover is only $sideDp dp", sideDp >= 160f)
        // The transport and actions are all still there.
        listOf("play-pause", "play-on-button", "lyrics-action", "queue-button").forEach { compose.onNodeWithTag(it).assertIsDisplayed() }
    }
}
