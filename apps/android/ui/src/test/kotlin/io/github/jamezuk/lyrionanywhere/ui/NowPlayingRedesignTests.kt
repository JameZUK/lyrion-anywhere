package io.github.jamezuk.lyrionanywhere.ui

import android.content.Context
import android.provider.Settings
import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.core.Quality
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PhoneVolume
import io.github.jamezuk.lyrionanywhere.ui.player.PhoneVolumeMath
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val ME = "02:4c:41:00:00:01"
private const val KITCHEN_PLAYER = "aa:00:00:00:00:01"

/** A stand-in for Android's media volume: 15 steps. */
internal class FakePhoneVolume(override val max: Int = 15, override val fixed: Boolean = false, var level: Int = 6) : PhoneVolume {
    val set = mutableListOf<Int>()
    private val listeners = mutableListOf<() -> Unit>()

    override fun index() = level

    override fun setIndex(index: Int) {
        level = index.coerceIn(0, max)
        set += level
        listeners.toList().forEach { it() }
    }

    override fun observe(onChange: () -> Unit): () -> Unit {
        listeners += onChange
        return { listeners -= onChange }
    }
}

private val HI_RES = Quality("FLAC", "FLAC 24/96", true, true, 96_000u, 24u, null)

/** Everything Now playing can show at once: a hi-res badge, a rating, "Up next", a sleep timer, lyrics. */
internal fun busyState(selected: String = KITCHEN_PLAYER, prefs: NowPlayingPrefs = NowPlayingPrefs()): PlayerUiState {
    val base = auditPlayerState(selected)
    return base.copy(
        status = sampleStatus().copy(
            current = sampleTrack(1, 0).copy(title = "A rather long title that will never fit on one line", quality = HI_RES),
            next = sampleTrack(2, 1).copy(title = "Next song"),
            sleepSecs = 1620.0,
        ),
        nowPlaying = prefs,
        lyrics = TrackLyrics(false, true, listOf(LyricLine(0u, "First line"), LyricLine(5_000u, "Second line"))),
        lyricsTrackId = 1,
        credits = listOf(Credit(ArtistRole.COMPOSER, "Ada Placeholder", 42L)),
        creditsTrackId = 1,
    )
}

/** Run the TalkBack action [label] of the node tagged [tag]. */
private fun androidx.compose.ui.test.junit4.ComposeContentTestRule.customAction(tag: String, label: String) {
    val action = onNodeWithTag(tag).fetchSemanticsNode().config[SemanticsActions.CustomActions].first { it.label == label }
    runOnUiThread { action.action() }
    waitForIdle()
}

/**
 * The transport is always fully on screen and nothing scrolls: on the smallest phone the
 * cover shrinks instead (the owner's first complaint).
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w360dp-h640dp")
class NowPlayingFitsSmallPhoneTest {
    @get:Rule
    val compose = createComposeRule()

    private fun assertFits(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.waitForIdle()
        assertTransportFits(compose)
    }

    @Test
    fun aRemotePlayerFits() = assertFits { NowPlayingScreen(busyState(), clock = { 0L }, actions = nowPlayingActions(onClose = {})) }

    @Test
    fun thisPhoneWithEveryActionFits() = assertFits {
        NowPlayingScreen(busyState(ME), clock = { 0L }, actions = nowPlayingActions(onClose = {}), phoneVolume = FakePhoneVolume())
    }

    @Test
    fun theLyricsPageScrollsOnlyInside() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(busyState(prefs = NowPlayingPrefs(layout = NowPlayingLayout.LYRICS)), clock = { 0L }, actions = nowPlayingActions())
            }
        }
        compose.waitForIdle()
        val root = compose.onRoot().fetchSemanticsNode().boundsInRoot
        val play = compose.onNodeWithTag("play-pause").fetchSemanticsNode().boundsInRoot
        assertTrue("play/pause at $play in $root", play.bottom <= root.bottom + 0.5f)
        // Only the lyrics list scrolls (inside its page).
        val scrolling = compose.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.VerticalScrollAxisRange)).fetchSemanticsNodes()
        val panel = compose.onNodeWithTag("page-panel-LYRICS").fetchSemanticsNode().boundsInRoot
        assertTrue(scrolling.isNotEmpty())
        scrolling.forEach { n -> assertTrue("${n.boundsInRoot} outside $panel", panel.contains(n.boundsInRoot.center)) }
    }

    @Test
    fun downloadsFitToo() {
        val tracks = listOf(dlTrack(1), dlTrack(2), dlTrack(3))
        val ui = OfflinePlayerUi(OfflinePlayerState(status = dlStatus(index = 0), tracks = tracks.associateBy { it.key }), at = 0)
        assertFits {
            OfflineNowPlayingScreen(ui, clock = { 0L }, actions = OfflineNowPlayingActions(), onClose = {}, visual = FakeVisualSource(), phoneVolume = FakePhoneVolume())
        }
    }
}

/** The same on a taller phone: the cover simply gets larger. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w360dp-h800dp")
class NowPlayingFitsTallerPhoneTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theTransportIsOnScreenAndTheCoverLarger() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(busyState(ME), clock = { 0L }, actions = nowPlayingActions(onClose = {}), phoneVolume = FakePhoneVolume())
            }
        }
        compose.waitForIdle()
        assertTransportFits(compose)
        val cover = compose.onNodeWithTag("artwork").fetchSemanticsNode()
        val sideDp = cover.size.height / cover.layoutInfo.density.density
        assertTrue("the cover is only $sideDp dp", sideDp >= 200f)
    }
}

/** Play/pause fully inside the window at full size, the actions too, and nothing that scrolls. */
private fun assertTransportFits(compose: androidx.compose.ui.test.junit4.ComposeContentTestRule) {
    val root = compose.onRoot().fetchSemanticsNode().boundsInRoot
    val playNode = compose.onNodeWithTag("play-pause").fetchSemanticsNode()
    val play = playNode.boundsInRoot
    val density = playNode.layoutInfo.density.density
    assertTrue("play/pause at $play outside $root", play.top >= root.top && play.bottom <= root.bottom + 0.5f && play.left >= root.left && play.right <= root.right + 0.5f)
    assertEquals("play/pause is cut", 72f, play.height / density, 0.6f)
    val actions = compose.onNodeWithTag("now-playing-actions").fetchSemanticsNode().boundsInRoot
    assertTrue("the actions at $actions outside $root", actions.bottom <= root.bottom + 0.5f)
    // The screen itself never scrolls.
    compose.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.VerticalScrollAxisRange)).assertCountEquals(0)
    // The cover shrank but is still there.
    val cover = compose.onNodeWithTag("artwork").fetchSemanticsNode()
    val parts = listOf("now-playing-top", "now-playing-pager", "page-dots", "now-playing-meta", "now-playing-title", "rating-toggle", "quality-badge", "lyrics-peek", "seek-elapsed", "transport", "up-next", "now-playing-actions", "lyrics-action")
        .joinToString { t -> "$t=" + compose.onAllNodesWithTag(t).fetchSemanticsNodes().firstOrNull()?.boundsInRoot?.let { "${it.top / density}-${it.bottom / density}" } }
    assertTrue("no room for the cover (${cover.size.height / density} dp; root ${root.height / density}; $parts)", cover.size.height / density >= 80f)
}

/** The hero's pager, the top row, ⋮, the visualiser's one button, the volume popover. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h900dp")
class NowPlayingRedesignScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private fun show(
        state: PlayerUiState,
        actions: NowPlayingActions = nowPlayingActions(),
        volume: PhoneVolume = FakePhoneVolume(),
        visual: io.github.jamezuk.lyrionanywhere.ui.player.VisualSource? = null,
    ) = compose.setContent {
        LyrionTheme(dynamicColour = false) { NowPlayingScreen(state, clock = { 0L }, actions = actions, visual = visual, phoneVolume = volume) }
    }

    @Test
    fun theDotsAreTabsNamedForTheirPages() {
        show(busyState())
        val names = mapOf("ART" to "Cover", "LYRICS" to "Lyrics", "INFO" to "Track info")
        names.forEach { (tag, name) ->
            compose.onNodeWithTag("page-$tag")
                .assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf(name)))
                .assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Tab))
        }
        compose.onNodeWithTag("page-ART").assertIsSelected()
        // No segmented row any more.
        compose.onAllNodesWithTag("layout-buttons").assertCountEquals(0)
    }

    @Test
    fun talkBackMovesBetweenPagesWithActions() {
        val chosen = mutableListOf<NowPlayingLayout>()
        show(busyState(), nowPlayingActions().copy(onLayout = { chosen += it }, onCredit = {}))
        compose.customAction("artwork", "Show the lyrics")
        compose.onNodeWithTag("page-panel-LYRICS").assertIsDisplayed()
        compose.customAction("page-panel-LYRICS", "Show the track info")
        compose.onNodeWithTag("info-panel").assertIsDisplayed()
        // The info page: details and credits, not the signal path.
        compose.onNodeWithText("FLAC 24/96").assertExists()
        compose.onNodeWithText("Ada Placeholder").assertExists()
        compose.customAction("page-panel-INFO", "Show the cover")
        compose.onNodeWithTag("artwork").assertIsDisplayed()
        assertEquals(listOf(NowPlayingLayout.LYRICS, NowPlayingLayout.INFO, NowPlayingLayout.ART), chosen)
    }

    @Test
    fun theTopRowClosesOpensPlayOnAndTheMenu() {
        var closed = 0
        var playOn = 0
        show(busyState(), nowPlayingActions(onClose = { closed++ }).copy(onOutputs = { playOn++ }))
        // Play on is the first of the bottom actions (Wave 3.5), named after the player.
        compose.onNodeWithTag("play-on-button").assertTextContains("Player $KITCHEN_PLAYER").performClick()
        compose.onNodeWithTag("now-playing-close").performClick()
        assertEquals(1, playOn)
        assertEquals(1, closed)
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("now-playing-more-sheet").assertExists()
    }

    @Test
    fun theMenuHoldsTheOccasionalThings() {
        val opened = mutableListOf<String>()
        show(
            busyState(ME),
            nowPlayingActions().copy(onCarMode = { opened += "car" }, onAlarms = { opened += "alarms" }, onRooms = { opened += "rooms" }, onMore = { opened += "track" }),
        )
        compose.onNodeWithTag("now-playing-more").performClick()
        for (tag in listOf("more-rate", "sleep-button", "volume-button", "visualiser-button", "more-visualiser-style", "more-car-mode", "more-track")) {
            compose.onNodeWithTag(tag).assertExists()
        }
        // A player's alarms and power are its row in Play on… (Wave 3.5 phase 3).
        compose.onNodeWithTag("more-alarms").assertDoesNotExist()
        compose.onNodeWithTag("more-power").assertDoesNotExist()
        // One way in (Wave 3.5): every room is in Play on…, the signal path is the badge's.
        compose.onNodeWithTag("more-rooms").assertDoesNotExist()
        compose.onNodeWithTag("more-signal-path").assertDoesNotExist()
        compose.onNodeWithTag("more-car-mode").performClick()
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("more-visualiser-style").performClick()
        compose.onNodeWithTag("visualiser-style-sheet").assertExists()
        assertEquals(listOf("car"), opened)
    }

    @Test
    fun aRemotePlayerHasNoVisualiserNorItsStyleInTheMenu() {
        show(busyState(), visual = FakeVisualSource())
        compose.onNodeWithTag("visualiser-button").assertDoesNotExist()
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("more-visualiser-style").assertDoesNotExist()
    }

    @Test
    fun theQualityBadgeOpensTheSignalPath() {
        show(busyState())
        compose.onNodeWithTag("signal-path-sheet").assertDoesNotExist()
        compose.onNodeWithTag("quality-badge").performClick()
        compose.onNodeWithTag("signal-path-sheet").assertExists()
    }

    @Test
    fun upNextIsOneLineThatCanStillBeRemoved() {
        var removed = 0
        show(busyState(), nowPlayingActions().copy(onRemoveUpNext = { removed++ }))
        // One line of text, no button: removing it is a swipe or the TalkBack action.
        compose.onNodeWithText("Next song — Artist", substring = true).assertIsDisplayed()
        compose.onNodeWithContentDescription("Remove Next song from the queue").assertDoesNotExist()
        val node = compose.onNodeWithContentDescription("Up next: Next song — Artist").fetchSemanticsNode()
        compose.runOnUiThread { node.config[SemanticsActions.CustomActions].single { it.label == "Remove from the queue" }.action() }
        assertEquals(1, removed)
    }

    @Test
    fun theSeekBarShowsElapsedAndRemaining() {
        show(busyState())
        compose.onNodeWithTag("seek-elapsed").assertTextEquals("0:10")
        compose.onNodeWithTag("seek-remaining").assertTextEquals("−3:10")
    }

    @Test
    fun theVisualiserIsOneButtonTapLongPressAndSwipe() {
        val on = mutableListOf<Boolean>()
        val styles = mutableListOf<VisualStyle>()
        val source = FakeVisualSource()
        show(busyState(ME), nowPlayingActions().copy(onVisualiser = { on += it }, onVisualStyle = { styles += it }), visual = source)
        // One button: no separate style button any more.
        compose.onAllNodesWithTag("visualiser-style").assertCountEquals(0)
        // In ⋮ since Wave 3.5.
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("visualiser-button").assert(SemanticsMatcher.expectValue(SemanticsProperties.Role, Role.Switch)).performClick()
        // It draws over the cover (which stays), and says which style.
        compose.onNodeWithTag("visualiser", useUnmergedTree = true).assertExists()
        compose.onNodeWithTag("artwork").assertExists()
        compose.onNodeWithTag("hero-toast").assertTextEquals("Bars")
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("visualiser-button").assertTextEquals("Bars")
        // A long press chooses a style.
        compose.onNodeWithTag("visualiser-button").performTouchInput { longClick() }
        compose.onNodeWithTag("visual-style-GLOW").performClick()
        compose.onNodeWithTag("glow-halo").assertExists()
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        // With Glow the pager takes sideways swipes again.
        compose.onNodeWithTag("artwork").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        compose.onNodeWithTag("page-panel-LYRICS").assertExists()
        assertEquals(listOf(true), on)
        assertEquals(listOf(VisualStyle.GLOW), styles)
        // Off from the sheet.
        compose.onNodeWithTag("page-ART").performClick()
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("visualiser-button").performTouchInput { longClick() }
        compose.onNodeWithTag("visualiser-switch-off").performClick()
        compose.onNodeWithTag("glow-halo").assertDoesNotExist()
        assertEquals(listOf(true, false), on)
    }

    @Test
    fun switchingTheVisualiserOnShowsTheCover() {
        val chosen = mutableListOf<NowPlayingLayout>()
        show(busyState(ME, NowPlayingPrefs(layout = NowPlayingLayout.INFO)), nowPlayingActions().copy(onLayout = { chosen += it }), visual = FakeVisualSource())
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("visualiser-button").performClick()
        compose.onNodeWithTag("artwork").assertIsDisplayed()
        assertEquals(listOf(NowPlayingLayout.ART), chosen)
    }

    @Test
    fun withAnimationsRemovedTheButtonIsOffAndSaysWhy() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        Settings.Global.putFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 0f)
        try {
            show(busyState(ME, NowPlayingPrefs(visualiser = true)), visual = FakeVisualSource())
            compose.onNodeWithTag("now-playing-more").performClick()
            compose.onNodeWithTag("visualiser-button").assertIsNotEnabled()
            val state = compose.onNodeWithTag("visualiser-button").fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription)
            assertEquals("Off while animations are removed", state)
            compose.onNodeWithTag("visualiser").assertDoesNotExist()
        } finally {
            Settings.Global.putFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f)
        }
    }

    @Test
    fun theVolumePopoverSetsARemotePlayerInLms() {
        val volumes = mutableListOf<Int>()
        val phone = FakePhoneVolume()
        show(busyState(), nowPlayingActions().copy(onVolume = { volumes += it }), volume = phone)
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("volume-button")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "40%"))
            .performClick()
        compose.onNodeWithTag("volume-popover").assertExists()
        compose.onNodeWithTag("volume-note").assertTextEquals("Player $KITCHEN_PLAYER")
        compose.onNodeWithTag("volume-slider")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Volume on Player $KITCHEN_PLAYER")))
            .performSemanticsAction(SemanticsActions.SetProgress) { it(55f) }
        compose.waitForIdle()
        assertEquals(55, volumes.last())
        assertTrue("the phone's own volume is left alone", phone.set.isEmpty())
    }

    @Test
    fun onThisPhoneItIsThePhonesMediaVolume() {
        val volumes = mutableListOf<Int>()
        val phone = FakePhoneVolume(max = 15, level = 6)
        show(busyState(ME), nowPlayingActions().copy(onVolume = { volumes += it }), volume = phone)
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("volume-button")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "40%"))
            .performClick()
        compose.onNodeWithTag("volume-note").assertTextEquals("Phone media volume")
        compose.onNodeWithTag("volume-slider")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Volume on this phone")))
            .performSemanticsAction(SemanticsActions.SetProgress) { it(100f) }
        compose.waitForIdle()
        assertEquals(15, phone.level)
        assertTrue("LMS's player level is not touched", volumes.isEmpty())
        // A change from elsewhere (the hardware keys) shows at once.
        compose.runOnUiThread { phone.setIndex(3) }
        compose.onNodeWithTag("volume-value").assertTextEquals("20")
    }

    @Test
    fun aFixedVolumeCantBeChanged() {
        show(busyState(ME), volume = FakePhoneVolume(fixed = true))
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("volume-button").performClick()
        compose.onNodeWithTag("volume-note").assertTextEquals("This output's volume is fixed")
        compose.onNodeWithTag("volume-slider").assertIsNotEnabled()
    }

    @Test
    fun theSleepActionShowsTheTimeLeft() {
        show(busyState())
        // The moon and the time left sit in the top row while it runs (Wave 3.5).
        compose.onNodeWithTag("sleep-pill").assertTextEquals("27 min")
        compose.onNodeWithTag("sleep-pill").performClick()
        compose.onNodeWithTag("sleep-sheet").assertExists()
    }
}

/** Every state of the new Now playing, audited as TalkBack sees it. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h900dp")
class NowPlayingRedesignAccessibilityTest {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(state: PlayerUiState, visual: Boolean = false, then: () -> Unit = {}) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(
                    state,
                    clock = { 0L },
                    actions = nowPlayingActions(onClose = {}).copy(onCarMode = {}, onAlarms = {}, onRooms = {}, onMore = {}, onCredit = {}),
                    visual = if (visual) FakeVisualSource() else null,
                    phoneVolume = FakePhoneVolume(),
                )
            }
        }
        then()
        compose.assertAccessible()
    }

    @Test
    fun theCover() = audit(busyState())

    @Test
    fun theCoverOnThisPhone() = audit(busyState(ME), visual = true)

    @Test
    fun theLyricsPage() = audit(busyState(prefs = NowPlayingPrefs(layout = NowPlayingLayout.LYRICS)))

    @Test
    fun theTrackInfoPage() = audit(busyState(prefs = NowPlayingPrefs(layout = NowPlayingLayout.INFO)))

    @Test
    fun theVisualiserOn() = audit(busyState(ME, NowPlayingPrefs(visualiser = true)), visual = true)

    @Test
    fun glowOn() = audit(busyState(ME, NowPlayingPrefs(visualiser = true, visualStyle = VisualStyle.GLOW)), visual = true)

    @Test
    fun theVolumePopover() = audit(busyState()) {
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("volume-button").performClick()
    }

    @Test
    fun theVolumePopoverOnThisPhone() = audit(busyState(ME)) {
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("volume-button").performClick()
    }

    @Test
    fun theMenu() = audit(busyState(ME), visual = true) { compose.onNodeWithTag("now-playing-more").performClick() }

    @Test
    fun theStyleSheet() = audit(busyState(ME, NowPlayingPrefs(visualiser = true)), visual = true) {
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("visualiser-button").performTouchInput { longClick() }
    }

    @Test
    fun theSignalPathSheet() = audit(busyState(ME)) { compose.onNodeWithTag("quality-badge").performClick() }

    @Test
    fun theSleepSheet() = audit(busyState()) {
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("sleep-button").performClick()
    }

    @Test
    fun theQueueSheet() = audit(busyState()) { compose.onNodeWithTag("queue-button").performClick() }

    @Test
    fun downloads() {
        val tracks = listOf(dlTrack(1), dlTrack(2))
        val ui = OfflinePlayerUi(OfflinePlayerState(status = dlStatus(index = 0), tracks = tracks.associateBy { it.key }), at = 0)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                OfflineNowPlayingScreen(ui, clock = { 0L }, actions = OfflineNowPlayingActions(), onClose = {}, visual = FakeVisualSource(), phoneVolume = FakePhoneVolume())
            }
        }
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("volume-button").performClick()
        compose.assertAccessible()
    }

    @Test
    fun thePlayerLevelSetting() {
        compose.setContent { LyrionTheme(dynamicColour = false) { SoundScreen(SoundUiState(playerLevel = 100), object : SoundActions {}) } }
        compose.assertAccessible()
    }
}

/** The player level in Settings › Sound, and the volume arithmetic. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h3000dp")
class PlayerLevelSettingTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theSliderSetsTheLevelAndExplainsIt() {
        val set = mutableListOf<Int>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SoundScreen(
                    SoundUiState(playerLevel = 100),
                    object : SoundActions {
                        override fun playerLevel(level: Int) {
                            set += level
                        }
                    },
                )
            }
        }
        compose.onNodeWithText("Player level: 100%").assertExists()
        compose.onNode(hasText("normally left at 100%", substring = true, ignoreCase = true)).assertExists()
        compose.onNodeWithTag("player-level").performSemanticsAction(SemanticsActions.SetProgress) { it(80f) }
        compose.waitForIdle()
        assertEquals(listOf(80), set)
    }

    @Test
    fun withoutAPlayerItSaysHowToGetOne() {
        compose.setContent { LyrionTheme(dynamicColour = false) { SoundScreen(SoundUiState(), object : SoundActions {}) } }
        compose.onNodeWithTag("player-level").assertDoesNotExist()
        compose.onNodeWithText("Turn on Play on this phone to set its level.").assertExists()
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class PlayerLevelViewModelTest : MainDispatcherTest() {
    @Test
    fun theLevelIsReadFromLmsAndSetThere() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val players = FakePlayers().apply { statusResult = ApiResult.Ok(sampleStatus().copy(volume = 80u)) }
        val vm = SoundViewModel(local, sdk = 34, players = players)
        // No player in LMS yet: nothing to set.
        assertNull(vm.state.value.playerLevel)
        vm.setPlayerLevel(50)
        assertTrue(players.commands.isEmpty())
        local.state.update { it.copy(enabled = true, status = localStatus(ME)) }
        assertEquals(80, vm.state.value.playerLevel)
        assertEquals(listOf(ME), players.statusCalls)
        vm.setPlayerLevel(150)
        assertEquals(100, vm.state.value.playerLevel)
        assertEquals(listOf("volume 100 $ME"), players.commands)
    }

    @Test
    fun phoneVolumeStepsAndPercentages() {
        assertEquals(40, PhoneVolumeMath.percent(6, 15))
        assertEquals(100, PhoneVolumeMath.percent(99, 15))
        assertEquals(0, PhoneVolumeMath.percent(-3, 15))
        assertEquals(0, PhoneVolumeMath.percent(3, 0))
        assertEquals(15, PhoneVolumeMath.index(100f, 15))
        assertEquals(8, PhoneVolumeMath.index(50f, 15))
        assertEquals(0, PhoneVolumeMath.index(-20f, 15))
        assertEquals(15, PhoneVolumeMath.index(1e9f, 15))
        assertEquals(0, PhoneVolumeMath.index(Float.NaN, 15))
        assertEquals(0, PhoneVolumeMath.index(50f, 0))
    }
}
