package io.github.jamezuk.lyrionanywhere.ui

import androidx.activity.ComponentActivity
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeEditActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeed
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuBrowser
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuNavigator
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairUiState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.UnreachableScreen
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.RatingsSetting
import io.github.jamezuk.lyrionanywhere.ui.player.msToNextLine
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val KITCHEN = "aa:00:00:00:00:01"

/**
 * Review fixes on the phone UI: "Play on…" is always reachable, system back follows the
 * in-screen levels, Home can't be edited from a layout that didn't load, sign-out from the
 * unreachable screen asks first, the camera dead end, clearing ratings, and lyric timing.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class ReachAndBackTests {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private fun back() = compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }

    private fun content(body: @androidx.compose.runtime.Composable () -> Unit) =
        compose.setContent { LyrionTheme(dynamicColour = false) { body() } }

    // --- "Play on…" with nothing playing ---------------------------------------------------

    @Test
    fun theMiniPlayerOffersAPlayerWhenNoneIsChosen() {
        var outputs = 0
        content {
            MiniPlayer(PlayerUiState(players = listOf(samplePlayer(KITCHEN)), loading = false), {}, {}, {}, onOutputs = { outputs++ })
        }
        compose.onNodeWithText("No player chosen").assertIsDisplayed()
        compose.onNodeWithText("Choose a player").performClick()
        assertEquals(1, outputs)
    }

    @Test
    fun theMiniPlayerOffersPlayOnWhenNothingIsQueued() {
        var outputs = 0
        val state = PlayerUiState(
            players = listOf(samplePlayer(KITCHEN)),
            selectedId = KITCHEN,
            status = sampleStatus().copy(current = null, queueLength = 0u),
            loading = false,
        )
        content { MiniPlayer(state, {}, {}, {}, onOutputs = { outputs++ }) }
        compose.onNodeWithText("Nothing playing on Player $KITCHEN").assertIsDisplayed()
        compose.onNodeWithTag("choose-player").performClick()
        assertEquals(1, outputs)
    }

    @Test
    fun nothingIsOfferedWhileThePlayersAreStillLoading() {
        content { MiniPlayer(PlayerUiState(), {}, {}, {}) }
        compose.onNodeWithTag("mini-choose-player").assertDoesNotExist()
    }

    @Test
    fun nowPlayingWithNoPlayerHasAButtonToChooseOne() {
        var outputs = 0
        content {
            NowPlayingScreen(
                PlayerUiState(players = listOf(samplePlayer(KITCHEN)), loading = false),
                clock = { 0L },
                actions = nowPlayingActions().copy(onOutputs = { outputs++ }),
            )
        }
        compose.onNodeWithTag("now-playing-choose-player").performClick()
        assertEquals(1, outputs)
    }

    // --- system back -----------------------------------------------------------------------

    @Test
    fun backGoesUpAPluginMenuLevelFirst() {
        val menus = FakeMenus().apply {
            pages["someapp"] = listOf(menuItem("Genres", MenuItemKind.FOLDER, go = action("genres", "items")))
            pages["genres"] = listOf(menuItem("Jazz", MenuItemKind.TEXT))
        }
        val navigator = MenuNavigator(CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate), menus) { KITCHEN }
        navigator.start("Some App", action("someapp", "items"))
        content { MenuBrowser(navigator) }
        compose.onNodeWithText("Genres").performClick()
        compose.waitForIdle()
        assertEquals(2, navigator.stack.value.size)
        back()
        compose.waitForIdle()
        assertEquals(1, navigator.stack.value.size)
        // At the first level, back is the screen's own (it isn't kept here).
        assertFalse(compose.activity.onBackPressedDispatcher.hasEnabledCallbacks())
    }

    private val noHome = HomeActions({}, {}, {}, {}, {}, {}, {})

    private fun editActions(onCancel: () -> Unit) = HomeEditActions({ _, _ -> }, { _, _ -> }, {}, {}, {}, {}, onCancel)

    private fun loadedFeed(draft: io.github.jamezuk.lyrionanywhere.core.HomeLayout?) = HomeFeed(
        recent = Loadable.Ready(listOf(sampleAlbum(1))),
        layout = HomeLayouts.DEFAULT,
        layoutLoaded = true,
        draft = draft,
    )

    @Test
    fun backLeavesAnUnchangedHomeEditAtOnce() {
        var cancels = 0
        content { HomeScreen(loadedFeed(HomeLayouts.DEFAULT), noHome, edit = editActions { cancels++ }) }
        back()
        compose.waitForIdle()
        assertEquals(1, cancels)
        compose.onNodeWithText("Discard your changes?").assertDoesNotExist()
    }

    @Test
    fun backAsksBeforeDiscardingHomeChanges() {
        var cancels = 0
        val changed = HomeLayouts.move(HomeLayouts.DEFAULT, 0, 1)
        content { HomeScreen(loadedFeed(changed), noHome, edit = editActions { cancels++ }) }
        back()
        compose.onNodeWithText("Discard your changes?").assertIsDisplayed()
        compose.onNodeWithText("Keep editing").performClick()
        assertEquals(0, cancels)
        // Cancel asks too.
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithTag("home-discard").performClick()
        assertEquals(1, cancels)
    }

    // --- Home whose layout didn't load ----------------------------------------------------

    @Test
    fun aHomeLayoutThatDidntLoadOffersRetryNotEdit() {
        var retries = 0
        val feed = HomeFeed(
            layout = HomeLayouts.DEFAULT,
            layoutLoaded = false,
            layoutError = AppError.Offline("down"),
            history = mapOf(ShelfKind.RECENT to Loadable.Ready(listOf(sampleAlbum(7)))),
        )
        content { HomeScreen(feed, noHome.copy(onRetry = { retries++ }), edit = editActions {}) }
        compose.onNodeWithTag("home-edit").assertDoesNotExist()
        compose.onNodeWithTag("home-layout-failed").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Try again").performClick()
        assertEquals(1, retries)
    }

    // --- signing out when the server can't be reached --------------------------------------

    @Test
    fun signingOutFromUnreachableAsksFirst() {
        val outs = mutableListOf<Boolean>()
        content { UnreachableScreen("music.example.com", "", 0, 0, onRetry = {}, onSignOut = { outs += it }) }
        compose.onNodeWithTag("unreachable-sign-out").performClick()
        assertTrue(outs.isEmpty())
        compose.onNodeWithText("Cancel").performClick()
        assertTrue(outs.isEmpty())
        compose.onNodeWithTag("unreachable-sign-out").performClick()
        compose.onNode(
            androidx.compose.ui.test.hasText("Sign out") and androidx.compose.ui.test.hasAnyAncestor(androidx.compose.ui.test.isDialog()),
        ).performClick()
        assertEquals(listOf(false), outs)
    }

    @Test
    fun signingOutFromUnreachableCanRemoveDownloads() {
        val outs = mutableListOf<Boolean>()
        content { UnreachableScreen("music.example.com", "", 12, 3_000_000, onRetry = {}, onSignOut = { outs += it }) }
        compose.onNodeWithTag("unreachable-sign-out").performClick()
        compose.onNodeWithText("Sign out and remove downloads").performClick()
        assertEquals(listOf(true), outs)
    }

    // --- the camera refused for good ------------------------------------------------------

    @Test
    fun aBlockedCameraExplainsAndKeepsTheCodeField() {
        var settings = 0
        var asked = 0
        content {
            PairScreen(
                state = PairUiState(),
                cameraPermitted = false,
                onRequestCamera = { asked++ },
                onScanned = {},
                onCodeChange = {},
                onSubmit = {},
                onBack = {},
                cameraBlocked = true,
                onOpenSettings = { settings++ },
            )
        }
        compose.onNodeWithTag("camera-denied").assertIsDisplayed()
        compose.onNodeWithText("Scan QR code").assertDoesNotExist()
        compose.onNodeWithText("Pairing code").assertIsDisplayed()
        compose.onNodeWithTag("camera-settings").performClick()
        assertEquals(1, settings)
        assertEquals(0, asked)
    }

    // --- clearing ratings -----------------------------------------------------------------

    @Test
    fun clearingRatingsAsksFirstAndCountsInWords() {
        var clears = 0
        var result: Long? = 1
        content {
            RatingsSetting(RatingStyle.STARS, {}) {
                clears++
                result
            }
        }
        compose.onNodeWithTag("clear-ratings").performClick()
        compose.onNodeWithText("Clear your ratings?").assertIsDisplayed()
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(0, clears)
        compose.onNodeWithTag("clear-ratings").performClick()
        compose.onNodeWithTag("clear-ratings-confirm").performClick()
        compose.waitForIdle()
        assertEquals(1, clears)
        compose.onNodeWithText("1 rating cleared").assertIsDisplayed()
        result = 7
        compose.onNodeWithTag("clear-ratings").performClick()
        compose.onNodeWithTag("clear-ratings-confirm").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("7 ratings cleared").assertIsDisplayed()
        result = null
        compose.onNodeWithTag("clear-ratings").performClick()
        compose.onNodeWithTag("clear-ratings-confirm").performClick()
        compose.waitForIdle()
        compose.onNodeWithText("Couldn't clear your ratings.").assertExists()
    }

    // --- the lyric line follows the singing -------------------------------------------------

    private val tight = TrackLyrics(
        false,
        true,
        listOf(LyricLine(0u, "Line A"), LyricLine(10_300u, "Line B"), LyricLine(10_600u, "Line C"), LyricLine(60_000u, "Line D")),
    )

    @Test
    fun theLyricHighlightMovesAsEachLineStarts() {
        compose.mainClock.autoAdvance = false
        val start = compose.mainClock.currentTime
        // Track 1 at 10 s, playing; the clock follows the test's.
        val state = PlayerUiState(
            players = listOf(samplePlayer(KITCHEN)),
            selectedId = KITCHEN,
            status = sampleStatus(elapsed = 10.0),
            loading = false,
            local = LocalPlayerState(),
            lyrics = tight,
            lyricsTrackId = 1,
            lyricsPeek = true,
        )
        content {
            NowPlayingScreen(state, clock = { compose.mainClock.currentTime - start }, actions = nowPlayingActions())
        }
        compose.mainClock.advanceTimeByFrame()
        compose.onNodeWithTag("lyrics-peek").assertTextEquals("Line A")
        // 400 ms on: well inside a second, the next line is already shown.
        compose.mainClock.advanceTimeBy(400)
        compose.onNodeWithTag("lyrics-peek").assertTextEquals("Line B")
        compose.mainClock.advanceTimeBy(300)
        compose.onNodeWithTag("lyrics-peek").assertTextEquals("Line C")
    }

    @Test
    fun theWaitIsUntilTheNextLine() {
        assertEquals(300L, msToNextLine(tight, 10_000))
        assertEquals(1L, msToNextLine(tight, 10_299))
        assertEquals(300L, msToNextLine(tight, 10_300))
        assertNull(msToNextLine(tight, 60_000))
        assertNull(msToNextLine(TrackLyrics(true, false, tight.lines), 0))
        assertNull(msToNextLine(null, 0))
    }
}
