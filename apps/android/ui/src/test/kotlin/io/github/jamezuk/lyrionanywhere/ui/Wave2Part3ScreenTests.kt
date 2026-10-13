package io.github.jamezuk.lyrionanywhere.ui

import android.content.Context
import android.os.PowerManager
import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.VisualFrame
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.Room
import io.github.jamezuk.lyrionanywhere.ui.player.RoomAlarm
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsActions
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsScreen
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsUiState
import io.github.jamezuk.lyrionanywhere.ui.player.VisualSource
import io.github.jamezuk.lyrionanywhere.ui.player.VisualiserSetting
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val PHONE_ID = "02:4c:41:00:00:01"
private const val KITCHEN_ID = "aa:00:00:00:00:01"
private const val LOUNGE_ID = "aa:00:00:00:00:02"

/** A visualiser source that records being switched on and off. */
internal class FakeVisualSource : VisualSource {
    val active = mutableListOf<Boolean>()

    override fun setActive(on: Boolean) {
        active += on
    }

    override fun frame(): VisualFrame? = VisualFrame.SILENT
}

/** Now playing with this phone as the player (the visualiser can show). */
internal fun phonePlayingState(prefs: NowPlayingPrefs = NowPlayingPrefs()) =
    auditPlayerState(selected = PHONE_ID).copy(nowPlaying = prefs)

internal fun sampleRooms(): RoomsUiState {
    fun room(id: String, name: String, mode: PlaybackMode, alarm: RoomAlarm? = null) = Room(
        Player(id, name, null, true, true, mode == PlaybackMode.PLAYING),
        sampleStatus(mode).copy(name = name, sleepSecs = if (alarm != null) 1_500.0 else null),
        alarm = alarm,
    )
    return RoomsUiState(
        rooms = listOf(
            room(KITCHEN_ID, "Kitchen", PlaybackMode.PLAYING, RoomAlarm(1, 7 * 3600)),
            room(LOUNGE_ID, "Lounge", PlaybackMode.PAUSED),
            room("aa:00:00:00:00:03", "Study", PlaybackMode.STOPPED),
        ),
        groups = listOf(SyncGroup(listOf(KITCHEN_ID, LOUNGE_ID))),
        loading = false,
    )
}

/** Now playing's layouts and visualiser, and the Rooms view (Wave 2 part 3). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class Wave2Part3ScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private fun nowPlaying(
        state: PlayerUiState,
        actions: NowPlayingActions = nowPlayingActions(),
        visual: VisualSource? = null,
        wide: Boolean = false,
    ) = show { NowPlayingScreen(state, clock = { 0L }, actions = actions, visual = visual, wide = wide) }

    @Test
    fun theLayoutButtonsSwitchAndAreRemembered() {
        val chosen = mutableListOf<NowPlayingLayout>()
        nowPlaying(auditPlayerState(), nowPlayingActions().copy(onLayout = { chosen += it }))
        compose.onNodeWithTag("artwork").assertExists()
        compose.onNodeWithTag("layout-LYRICS").performClick()
        compose.onNodeWithTag("layout-panel-LYRICS").assertExists()
        compose.onNodeWithTag("lyrics-none").assertExists()
        compose.onNodeWithTag("layout-INFO").performClick()
        compose.onNodeWithTag("info-panel").assertExists()
        compose.onNodeWithTag("up-next-card").assertExists()
        assertEquals(listOf(NowPlayingLayout.LYRICS, NowPlayingLayout.INFO), chosen)
    }

    @Test
    fun theRememberedLayoutShowsFirst() {
        val lyrics = TrackLyrics(false, true, listOf(LyricLine(0u, "First line"), LyricLine(5_000u, "Second line")))
        nowPlaying(auditPlayerState().copy(nowPlaying = NowPlayingPrefs(layout = NowPlayingLayout.LYRICS), lyrics = lyrics, lyricsTrackId = 1))
        compose.onNodeWithTag("layout-panel-LYRICS").assertExists()
        compose.onNodeWithText("Second line").assertExists()
    }

    @Test
    fun aSwipeAcrossTheArtMovesToTheNextLayout() {
        val chosen = mutableListOf<NowPlayingLayout>()
        nowPlaying(auditPlayerState(), nowPlayingActions().copy(onLayout = { chosen += it }))
        compose.onNodeWithTag("artwork").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(NowPlayingLayout.LYRICS), chosen)
        compose.onNodeWithTag("layout-panel-LYRICS").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(NowPlayingLayout.LYRICS, NowPlayingLayout.INFO), chosen)
    }

    @Test
    fun talkBackGetsTheOtherLayoutsAsActions() {
        val chosen = mutableListOf<NowPlayingLayout>()
        nowPlaying(auditPlayerState(), nowPlayingActions().copy(onLayout = { chosen += it }))
        val actions = compose.onNodeWithTag("artwork").fetchSemanticsNode().config[SemanticsActions.CustomActions]
        val labels = actions.map { it.label }
        assertTrue(labels.toString(), "Show the lyrics" in labels && "Show the track info" in labels)
        compose.runOnUiThread { actions.first { it.label == "Show the track info" }.action() }
        compose.waitForIdle()
        assertEquals(listOf(NowPlayingLayout.INFO), chosen)
        compose.onNodeWithTag("info-panel").assertExists()
    }

    @Test
    fun infoShowsCreditsAsLinksAndAsksForThem() {
        var asked = 0
        val followed = mutableListOf<Credit>()
        val credit = Credit(ArtistRole.COMPOSER, "Example Composer", 41)
        nowPlaying(
            auditPlayerState().copy(nowPlaying = NowPlayingPrefs(layout = NowPlayingLayout.INFO), credits = listOf(credit), creditsTrackId = 1),
            nowPlayingActions().copy(onCredits = { asked++ }, onCredit = { followed += it }),
        )
        compose.waitForIdle()
        assertTrue(asked >= 1)
        compose.onNodeWithTag("credit-41-COMPOSER").performClick()
        assertEquals(listOf(credit), followed)
    }

    @Test
    fun noVisualiserOnARemotePlayer() {
        val source = FakeVisualSource()
        nowPlaying(auditPlayerState().copy(nowPlaying = NowPlayingPrefs(visualiser = true)), visual = source)
        compose.onNodeWithTag("visualiser-toggle").assertDoesNotExist()
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        compose.onNodeWithTag("artwork").assertExists()
        assertTrue(source.active.isEmpty())
    }

    @Test
    fun offByDefaultThenSwitchedOnItRunsAndChangesStyle() {
        val source = FakeVisualSource()
        val on = mutableListOf<Boolean>()
        val styles = mutableListOf<VisualStyle>()
        nowPlaying(phonePlayingState(), nowPlayingActions().copy(onVisualiser = { on += it }, onVisualStyle = { styles += it }), source)
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        assertTrue("nothing analysed while off", source.active.isEmpty())
        compose.onNodeWithTag("visualiser-toggle").performClick()
        compose.onNodeWithTag("visualiser", useUnmergedTree = true).assertExists()
        assertEquals(listOf(true), on)
        assertEquals(listOf(true), source.active)
        compose.onNodeWithTag("visualiser-style").performClick()
        assertEquals(listOf(VisualStyle.SCOPE), styles)
        // Off again: the engine is told to stop.
        compose.onNodeWithTag("visualiser-toggle").performClick()
        compose.waitForIdle()
        assertEquals(listOf(true, false), source.active)
    }

    @Test
    fun theVisualiserIsHiddenFromScreenReaders() {
        nowPlaying(phonePlayingState(NowPlayingPrefs(visualiser = true)), visual = FakeVisualSource())
        val node = compose.onNodeWithTag("visualiser", useUnmergedTree = true).fetchSemanticsNode()
        assertTrue(node.children.isEmpty())
        assertEquals(null, node.config.getOrNull(SemanticsProperties.ContentDescription))
        // Its controls are labelled.
        compose.onNodeWithContentDescription("Visualiser").assertExists()
        compose.onNodeWithContentDescription("Visualiser style: Bars. Change style").assertExists()
    }

    @Test
    fun glowLightsTheBackgroundAndKeepsTheCover() {
        val source = FakeVisualSource()
        nowPlaying(phonePlayingState(NowPlayingPrefs(visualiser = true, visualStyle = VisualStyle.GLOW)), visual = source)
        compose.onNodeWithTag("artwork").assertExists()
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        assertEquals(listOf(true), source.active)
    }

    @Test
    fun batterySaverKeepsItOffAndSaysWhy() {
        val context = ApplicationProvider.getApplicationContext<Context>()
        shadowOf(context.getSystemService(PowerManager::class.java)).setIsPowerSaveMode(true)
        val source = FakeVisualSource()
        nowPlaying(phonePlayingState(NowPlayingPrefs(visualiser = true)), visual = source)
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        compose.onNodeWithTag("visualiser-toggle").assertIsNotEnabled()
        val state = compose.onNodeWithTag("visualiser-toggle").fetchSemanticsNode().config.getOrNull(SemanticsProperties.StateDescription)
        assertEquals("Off in battery saver", state)
        assertTrue(source.active.isEmpty())
    }

    @Test
    @Config(sdk = [35], qualifiers = "w1280dp-h900dp")
    fun wideWindowsPutLyricsOrInfoBesideTheArtAndTheQueueCanBeHidden() {
        val panel = mutableListOf<Boolean>()
        nowPlaying(auditPlayerState(), nowPlayingActions().copy(onQueuePanel = { panel += it }), wide = true)
        compose.onNodeWithTag("now-playing-wide").assertExists()
        compose.onNodeWithTag("artwork").assertExists()
        compose.onNodeWithTag("wide-pane-INFO").assertExists()
        compose.onNodeWithTag("queue-pane").assertExists()
        compose.onNodeWithTag("layout-LYRICS").performClick()
        compose.onNodeWithTag("wide-pane-LYRICS").assertExists()
        compose.onNodeWithTag("queue-panel-toggle").performClick()
        compose.onNodeWithTag("queue-pane").assertDoesNotExist()
        assertEquals(listOf(false), panel)
    }

    @Test
    fun thePlayerChipOpensAllRooms() {
        var rooms = 0
        nowPlaying(auditPlayerState(), nowPlayingActions().copy(onRooms = { rooms++ }))
        compose.onNodeWithTag("player-chip").performClick()
        assertEquals(1, rooms)
    }

    @Test
    fun playOnOffersAllRooms() {
        var rooms = 0
        show { PlayOnList(auditPlayerState(), PlayOnActions(onControl = {}, onBackToPhone = {}, onAllRooms = { rooms++ })) }
        compose.onNodeWithTag("all-rooms").performClick()
        assertEquals(1, rooms)
    }

    private val calls = mutableListOf<String>()

    private fun roomsActions() = RoomsActions(
        onControl = { calls += "control $it" },
        onTogglePlay = { calls += "play $it" },
        onVolume = { id, v -> calls += "volume $id $v" },
        onPower = { id, on -> calls += "power $id $on" },
        onSync = { a, b -> calls += "sync $a $b" },
        onUnsync = { calls += "unsync $it" },
        onTransfer = { calls += "transfer $it" },
        onAlarms = { id, _ -> calls += "alarms $id" },
    )

    @Test
    fun roomCardsAreLabelledWithTheRoomAndControlIt() {
        show { RoomsScreen(sampleRooms(), roomsActions(), selectedId = LOUNGE_ID) }
        compose.onNodeWithContentDescription("Kitchen volume").assertExists()
        compose.onNodeWithContentDescription("Pause Kitchen").performClick()
        compose.onNodeWithContentDescription("Play Lounge").performClick()
        compose.onNodeWithContentDescription("Power, Kitchen").performClick()
        compose.onNodeWithTag("room-head-$KITCHEN_ID").performClick()
        compose.onNodeWithTag("room-alarm-$KITCHEN_ID").assertExists()
        compose.onNodeWithTag("room-sleep-$KITCHEN_ID").assertExists()
        compose.onNodeWithText("With Lounge").assertExists()
        assertEquals(listOf("play $KITCHEN_ID", "play $LOUNGE_ID", "power $KITCHEN_ID false", "control $KITCHEN_ID"), calls)
    }

    @Test
    fun aLongPressMovesPlaybackThere() {
        show { RoomsScreen(sampleRooms(), roomsActions(), selectedId = LOUNGE_ID) }
        compose.onNodeWithTag("room-head-$KITCHEN_ID").performTouchInput { longClick() }
        assertEquals(listOf("transfer $KITCHEN_ID"), calls)
    }

    @Test
    fun syncWithFromTheMenuAndLeavingTheGroup() {
        show { RoomsScreen(sampleRooms(), roomsActions(), selectedId = LOUNGE_ID) }
        compose.onNodeWithTag("room-menu-aa:00:00:00:00:03").performClick()
        compose.onNodeWithTag("room-sync-aa:00:00:00:00:03").performClick()
        compose.onNodeWithTag("sync-with-$KITCHEN_ID").performClick()
        compose.onNodeWithTag("room-menu-$LOUNGE_ID").performClick()
        compose.onNodeWithTag("room-unsync-$LOUNGE_ID").performClick()
        assertEquals(listOf("sync aa:00:00:00:00:03 $KITCHEN_ID", "unsync $LOUNGE_ID"), calls)
    }

    @Test
    fun talkBackActionsStandInForDragging() {
        show { RoomsScreen(sampleRooms(), roomsActions(), selectedId = LOUNGE_ID) }
        val actions = compose.onNodeWithTag("room-head-aa:00:00:00:00:03").fetchSemanticsNode().config[SemanticsActions.CustomActions]
        val labels = actions.map { it.label }
        assertTrue(labels.toString(), "Sync Study with Kitchen" in labels && "Sync Study with Lounge" in labels)
        compose.runOnUiThread { actions.first { it.label == "Sync Study with Kitchen" }.action() }
        assertEquals(listOf("sync aa:00:00:00:00:03 $KITCHEN_ID"), calls)
    }

    @Test
    fun draggingACardOntoAnotherSyncsThem() {
        show { RoomsScreen(sampleRooms(), roomsActions(), selectedId = LOUNGE_ID) }
        val target = compose.onNodeWithTag("room-$KITCHEN_ID").fetchSemanticsNode().boundsInRoot
        val handle = compose.onNodeWithTag("room-drag-aa:00:00:00:00:03", useUnmergedTree = true)
        val from = handle.fetchSemanticsNode().boundsInRoot
        handle.performTouchInput {
            down(center)
            moveBy(androidx.compose.ui.geometry.Offset(0f, 20f))
            moveBy(androidx.compose.ui.geometry.Offset(target.center.x - from.center.x, target.center.y - from.center.y - 20f))
            up()
        }
        compose.waitForIdle()
        assertEquals(listOf("sync aa:00:00:00:00:03 $KITCHEN_ID"), calls)
    }

    @Test
    fun theVisualiserSettingIsOffByDefaultWithStylesOnceOn() {
        val on = mutableListOf<Boolean>()
        show { VisualiserSetting(false, VisualStyle.BARS, onChange = { on += it }, onStyle = {}) }
        compose.onNodeWithTag("visual-style-VU").assertDoesNotExist()
        compose.onNodeWithTag("visualiser-setting").performClick()
        assertEquals(listOf(true), on)
    }

    @Test
    fun roomsShowEmptyAndErrorStates() {
        show { RoomsScreen(RoomsUiState(loading = false), roomsActions()) }
        compose.onNodeWithText("No rooms to show").assertExists()
    }
}
