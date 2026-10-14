package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.TransferRefusal
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.ui.components.ListDetail
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueDrag
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEntry
import io.github.jamezuk.lyrionanywhere.ui.player.QueueHeader
import io.github.jamezuk.lyrionanywhere.ui.player.SwipeableArtwork
import io.github.jamezuk.lyrionanywhere.ui.player.TransferState
import io.github.jamezuk.lyrionanywhere.ui.player.errorDetail
import io.github.jamezuk.lyrionanywhere.ui.player.errorWithDetail
import io.github.jamezuk.lyrionanywhere.ui.player.queueItems
import io.github.jamezuk.lyrionanywhere.ui.player.queueKey
import io.github.jamezuk.lyrionanywhere.ui.player.transferText
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"
private const val LOUNGE = "aa:00:00:00:00:02"

private fun state(
    selected: String = PHONE,
    transfer: TransferState = TransferState.Idle,
    groups: List<SyncGroup> = emptyList(),
) = PlayerUiState(
    players = listOf(samplePlayer(PHONE), samplePlayer(KITCHEN), samplePlayer(LOUNGE)),
    selectedId = selected,
    status = sampleStatus(),
    loading = false,
    local = LocalPlayerState(enabled = true, status = localStatus(PHONE)),
    syncGroups = groups,
    transfer = transfer,
    queue = listOf(QueueEntry(1, sampleTrack(1, 0)), QueueEntry(2, sampleTrack(2, 1)), QueueEntry(3, sampleTrack(3, 2))),
)

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class TransferScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val res get() = ApplicationProvider.getApplicationContext<android.content.Context>().resources

    private fun sheet(s: PlayerUiState, actions: PlayOnActions) = compose.setContent {
        LyrionTheme(dynamicColour = false) { PlayOnList(s, actions) }
    }

    @Test
    fun eachOtherPlayerOffersMovePlaybackHere() {
        val moved = mutableListOf<String>()
        sheet(state(), PlayOnActions(onControl = {}, onBackToPhone = {}, onMoveHere = { moved += it }))
        // A secondary action in each row's ⋮ (Wave 3.5; the paragraph about it went).
        compose.onNodeWithText("Moving playback replaces the other player's queue.").assertDoesNotExist()
        compose.onNodeWithTag("play-on-$KITCHEN-more").performClick()
        compose.onNodeWithText("Move playback to Player $KITCHEN").performClick()
        assertEquals(listOf(KITCHEN), moved)
        // Not for the player already being controlled.
        compose.onNodeWithTag("play-on-$PHONE-more").assertDoesNotExist()
    }

    @Test
    fun whileControllingAnotherPlayerThePhoneRowOffersBringToThisPhone() {
        var brought = 0
        sheet(state(selected = KITCHEN), PlayOnActions(onControl = {}, onBackToPhone = {}, onBringToPhone = { brought++ }))
        compose.onNodeWithTag("play-on-phone-more").performClick()
        compose.onNodeWithText("Move playback to this phone").performClick()
        assertEquals(1, brought)
    }

    @Test
    fun progressIsShownAndMovesCannotOverlap() {
        val moved = mutableListOf<String>()
        sheet(
            state(transfer = TransferState.Running(KITCHEN, "Kitchen", TransferStage.COPYING)),
            PlayOnActions(onControl = {}, onBackToPhone = {}, onMoveHere = { moved += it }),
        )
        compose.onNodeWithText("Moving playback to Kitchen: copying the queue…").assertIsDisplayed()
        compose.onNodeWithTag("play-on-$LOUNGE-more").performClick()
        compose.onNodeWithTag("play-on-$LOUNGE-move").assertIsNotEnabled().performClick()
        assertTrue(moved.isEmpty())
    }

    @Test
    fun aFailureShowsTheErrorsDetailAndCanBeDismissed() {
        var dismissed = 0
        sheet(
            state(transfer = TransferState.Failed("Kitchen", AppError.Forbidden("player not permitted"))),
            PlayOnActions(onControl = {}, onBackToPhone = {}, onDismissTransfer = { dismissed++ }),
        )
        compose.onNode(hasText("player not permitted", substring = true)).assertIsDisplayed()
        compose.onNode(hasText("Couldn't move playback to Kitchen.", substring = true)).assertIsDisplayed()
        compose.onNodeWithText("Dismiss").performClick()
        assertEquals(1, dismissed)
    }

    @Test
    fun theMiniPlayerShowsProgressAndPlayOnWontMoveAgain() {
        var brought = 0
        val running = state(selected = KITCHEN, transfer = TransferState.Running(PHONE, "Phone", TransferStage.GROUPING))
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                androidx.compose.foundation.layout.Column {
                    MiniPlayer(running, onOpen = {}, onTogglePlay = {}, onNext = {})
                    PlayOnList(running, PlayOnActions(onControl = {}, onBackToPhone = {}, onBringToPhone = { brought++ }))
                }
            }
        }
        compose.onAllNodesWithText("Moving playback to Phone…")[0].assertIsDisplayed()
        // Moving playback here (Play on…) is not offered again while one is running.
        compose.onNodeWithTag("play-on-phone-more").performClick()
        compose.onNodeWithTag("play-on-phone-move").assertIsNotEnabled()
        assertEquals(0, brought)
    }

    @Test
    fun everyOutcomeHasPlainWords() {
        for (r in TransferRefusal.entries) {
            val text = transferText(res, TransferState.Refused("Kitchen", r))!!
            assertTrue(text, text.isNotBlank())
        }
        assertEquals(
            "Kitchen can't take over radio or app items. Start them on Kitchen instead.",
            transferText(res, TransferState.Refused("Kitchen", TransferRefusal.REMOTE)),
        )
        assertEquals("Nothing to move: the queue is empty.", transferText(res, TransferState.Refused("Kitchen", TransferRefusal.EMPTY)))
        assertNull(transferText(res, TransferState.Idle))
        assertTrue(transferText(res, TransferState.PhoneNotReady)!!.startsWith("This phone isn't ready"))
        for (stage in TransferStage.entries) {
            assertTrue(transferText(res, TransferState.Running(KITCHEN, "Kitchen", stage))!!.contains("Kitchen"))
        }
    }

    @Test
    fun errorDetailsAreShownButNeverTwice() {
        assertEquals("player not permitted", errorDetail(AppError.Forbidden(" player not permitted ")))
        assertNull(errorDetail(AppError.Forbidden("")))
        assertNull(errorDetail(AppError.SignedOut))
        assertNull(errorDetail(AppError.RateLimited(5)))
        assertEquals(200, errorDetail(AppError.Server(500, "x".repeat(500)))!!.length)
        val offline = errorWithDetail(res, AppError.Offline("connection refused"))
        assertEquals(1, offline.split("connection refused").size - 1)
        assertTrue(errorWithDetail(res, AppError.Server(409, "the queue changed")).endsWith("(the queue changed)"))
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class SyncScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theSyncToggleJoinsAndLeavesTheControlledPlayersGroup() {
        val calls = mutableListOf<Pair<String, Boolean>>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                PlayOnList(
                    state(selected = KITCHEN, groups = listOf(SyncGroup(listOf(KITCHEN, LOUNGE)))),
                    PlayOnActions(onControl = {}, onBackToPhone = {}, onSync = { id, on -> calls += id to on }),
                )
            }
        }
        // In Group players… (Wave 3.5).
        compose.onNodeWithTag("group-players").performClick()
        compose.onNodeWithTag("sync-$LOUNGE").assertIsOn().performClick()
        compose.onNodeWithTag("sync-$PHONE").assertDoesNotExist()
        assertEquals(listOf(LOUNGE to false), calls)
        // The group is listed with its members' names.
        compose.onNodeWithText("Player $KITCHEN + Player $LOUNGE").assertIsDisplayed()
    }

    @Test
    fun aPlayerOutsideTheGroupCanJoin() {
        val calls = mutableListOf<Pair<String, Boolean>>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                PlayOnList(state(selected = PHONE), PlayOnActions(onControl = {}, onBackToPhone = {}, onSync = { id, on -> calls += id to on }))
            }
        }
        compose.onNodeWithTag("group-players").performClick()
        compose.onNodeWithTag("sync-$KITCHEN").assertIsOff().performClick()
        assertEquals(listOf(KITCHEN to true), calls)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class QueueScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private val removed = mutableListOf<Int>()
    private val moves = mutableListOf<Pair<Int, Int>>()
    private val jumps = mutableListOf<Int>()

    private fun queue() = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            val drag = androidx.compose.runtime.remember { QueueDrag() }
            LazyColumn {
                queueItems(
                    state().queue,
                    currentIndex = 0,
                    drag = drag,
                    actions = QueueActions(onJump = { jumps += it }, onRemove = { removed += it }, onMove = { a, b -> moves += a to b }),
                )
            }
        }
    }

    @Test
    fun swipingLeftRemovesTheRow() {
        queue()
        compose.onNodeWithTag("queue-row-1").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(1), removed)
    }

    /** Regression: a removal LMS refuses puts the same entry back, and it must reappear. */
    @Test
    fun aRefusedRemovalShowsTheRowAgain() {
        // onRemove does nothing: the list keeps the entry, as after a refused removal.
        queue()
        compose.onNodeWithTag("queue-row-1").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(1), removed)
        compose.onNodeWithText("Track 2").assertIsDisplayed()
        compose.onNodeWithText("Track 2").performClick()
        assertEquals(listOf(1), jumps)
    }

    @Test
    fun swipingRightDoesNothing() {
        queue()
        compose.onNodeWithTag("queue-row-1").performTouchInput { swipeRight() }
        compose.waitForIdle()
        assertTrue(removed.isEmpty())
    }

    @Test
    fun tappingPlaysThatTrack() {
        queue()
        compose.onNodeWithText("Track 3").performClick()
        assertEquals(listOf(2), jumps)
    }

    @Test
    fun screenReadersCanMoveAndRemoveRows() {
        queue()
        // The actions are on the node TalkBack focuses: the one that plays the track.
        val first = compose.onNodeWithTag("queue-track-0").fetchSemanticsNode().config
        assertTrue(SemanticsActions.OnClick in first)
        val actions = first[SemanticsActions.CustomActions]
        assertEquals(listOf("Move down", "Remove from queue"), actions.map { it.label })
        actions.first { it.label == "Move down" }.action()
        val middle = compose.onNodeWithTag("queue-track-1").fetchSemanticsNode().config[SemanticsActions.CustomActions]
        assertEquals(listOf("Move up", "Move down", "Remove from queue"), middle.map { it.label })
        middle.first { it.label == "Move up" }.action()
        middle.first { it.label == "Remove from queue" }.action()
        assertEquals(listOf(0 to 1, 1 to 0), moves)
        assertEquals(listOf(1), removed)
        // Not on the swipe wrapper as well, where TalkBack would never reach them.
        val wrapper = compose.onNodeWithTag("queue-row-1", useUnmergedTree = true).fetchSemanticsNode().config
        assertTrue(SemanticsActions.CustomActions !in wrapper)
    }

    @Test
    fun keyboardMovesAndRemovesTheFocusedRow() {
        val actions = QueueActions(onJump = { jumps += it }, onRemove = { removed += it }, onMove = { a, b -> moves += a to b })
        val alt = android.view.KeyEvent.META_ALT_ON or android.view.KeyEvent.META_ALT_LEFT_ON
        assertTrue(queueKey(keyDown(android.view.KeyEvent.KEYCODE_DPAD_UP, alt), 1, 3, actions))
        assertTrue(queueKey(keyDown(android.view.KeyEvent.KEYCODE_DPAD_DOWN, alt), 1, 3, actions))
        // Not past either end.
        assertTrue(queueKey(keyDown(android.view.KeyEvent.KEYCODE_DPAD_UP, alt), 0, 3, actions))
        assertTrue(queueKey(keyDown(android.view.KeyEvent.KEYCODE_DPAD_DOWN, alt), 2, 3, actions))
        assertEquals(listOf(1 to 0, 1 to 2), moves)
        // Plain arrows move focus, as usual.
        assertFalse(queueKey(keyDown(android.view.KeyEvent.KEYCODE_DPAD_UP), 1, 3, actions))
        assertTrue(queueKey(keyDown(android.view.KeyEvent.KEYCODE_FORWARD_DEL), 1, 3, actions))
        assertTrue(queueKey(keyDown(android.view.KeyEvent.KEYCODE_DEL), 2, 3, actions))
        assertEquals(listOf(1, 2), removed)
        assertFalse(queueKey(keyDown(android.view.KeyEvent.KEYCODE_A), 1, 3, actions))
    }

    @Test
    fun theCurrentTrackIsMarkedForScreenReaders() {
        queue()
        compose.onNodeWithTag("queue-track-0")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "Now playing"))
        compose.onNodeWithTag("queue-track-1")
            .assert(SemanticsMatcher.keyNotDefined(SemanticsProperties.StateDescription))
    }

    @Test
    fun clearingAsksFirst() {
        var cleared = 0
        compose.setContent { LyrionTheme(dynamicColour = false) { QueueHeader(3, "Kitchen", onClear = { cleared++ }) } }
        // The header's actions are in its ⋮ (Wave 3.5).
        compose.onNodeWithTag("queue-actions").performClick()
        compose.onNodeWithText("Clear queue").performClick()
        compose.onNodeWithText("This removes every track from the queue of Kitchen.").assertIsDisplayed()
        compose.onNodeWithText("Cancel").performClick()
        assertEquals(0, cleared)
        compose.onNodeWithTag("queue-actions").performClick()
        compose.onNodeWithText("Clear queue").performClick()
        compose.onNodeWithText("Clear").performClick()
        assertEquals(1, cleared)
    }

    @Test
    fun anEmptyQueueOffersNoClear() {
        compose.setContent { LyrionTheme(dynamicColour = false) { QueueHeader(0, "Kitchen", onClear = {}) } }
        compose.onNodeWithText("Clear queue").assertDoesNotExist()
        compose.onNodeWithTag("queue-actions").assertDoesNotExist()
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class GestureScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun swipingTheArtworkSkips() {
        val calls = mutableListOf<String>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SwipeableArtwork(onNext = { calls += "next" }, onPrevious = { calls += "previous" }, modifier = Modifier.size(300.dp)) {
                    Box(Modifier.size(300.dp))
                }
            }
        }
        val node = compose.onRoot()
        node.performTouchInput { swipeLeft() }
        node.performTouchInput { swipeRight() }
        assertEquals(listOf("next", "previous"), calls)
    }

    @Test
    fun swipingTheMiniPlayerUpOpensNowPlaying() {
        var opened = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                MiniPlayer(state(), onOpen = { opened++ }, onTogglePlay = {}, onNext = {})
            }
        }
        compose.onNodeWithTag("mini-player").performTouchInput { swipeUp(startY = bottom, endY = top - 400f) }
        assertEquals(1, opened)
    }

    @Test
    fun theArtworkOffersNextAndPreviousToScreenReaders() {
        val calls = mutableListOf<String>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(
                    state(),
                    clock = { 0L },
                    actions = nowPlayingActions(onNext = { calls += "next" }, onPrevious = { calls += "previous" }),
                )
            }
        }
        val actions = compose.onNodeWithTag("artwork").fetchSemanticsNode().config[SemanticsActions.CustomActions]
        actions.forEach { it.action() }
        assertEquals(listOf("next", "previous"), calls)
    }
}

internal fun nowPlayingActions(
    onNext: () -> Unit = {},
    onPrevious: () -> Unit = {},
    onClose: (() -> Unit)? = null,
) = NowPlayingActions(
    onTogglePlay = {},
    onNext = onNext,
    onPrevious = onPrevious,
    onSeek = {},
    onVolume = {},
    onShuffle = {},
    onRepeat = {},
    onPower = {},
    onJump = {},
    onRetry = {},
    onClose = onClose,
)

/** Layouts by window size: two panes where they fit. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w1280dp-h800dp")
class WideLayoutScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun nowPlayingPutsTheCoverBesideTheControls() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(state(), clock = { 0L }, actions = nowPlayingActions(), twoPane = true)
            }
        }
        compose.onNodeWithTag("now-playing-two-pane").assertIsDisplayed()
        compose.onNodeWithTag("artwork").assertIsDisplayed()
        compose.onNodeWithTag("play-pause").assertIsDisplayed()
        // The queue is a tap away, in its sheet.
        compose.onNodeWithTag("queue-button").performClick()
        compose.onNodeWithText("Track 3").assertExists()
    }

    @Test
    fun listAndDetailShowSideBySide() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ListDetail(
                    list = { androidx.compose.material3.Text("The list") },
                    detail = null,
                    placeholder = { androidx.compose.material3.Text("Choose one") },
                )
            }
        }
        compose.onNodeWithText("The list").assertIsDisplayed()
        compose.onNodeWithText("Choose one").assertIsDisplayed()
        compose.onNodeWithTag("detail-pane").assertIsDisplayed()
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h900dp")
class PhoneLayoutScreenTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun nowPlayingIsOneColumnOnAPhone() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(state(), clock = { 0L }, actions = nowPlayingActions())
            }
        }
        compose.onNodeWithTag("now-playing-phone").assertIsDisplayed()
        compose.onNodeWithTag("now-playing-two-pane").assertDoesNotExist()
    }

    @Test
    fun swipingDownOnTheCoverCloses() {
        var closed = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(state(), clock = { 0L }, actions = nowPlayingActions(onClose = { closed++ }))
            }
        }
        compose.onNodeWithTag("artwork").performTouchInput {
            swipeDown(startY = top + 10f, endY = bottom - 10f)
        }
        compose.waitForIdle()
        assertEquals(1, closed)
    }
}

/** A Compose key-down event, for testing key handlers directly. */
internal fun keyDown(code: Int, meta: Int = 0) =
    androidx.compose.ui.input.key.KeyEvent(android.view.KeyEvent(0L, 0L, android.view.KeyEvent.ACTION_DOWN, code, 0, meta))
