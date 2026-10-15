package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.ui.player.JoinGroup
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.SyncBlock
import io.github.jamezuk.lyrionanywhere.ui.player.SyncTarget
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
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
private const val STUDY = "aa:00:00:00:00:04"
private const val HALL = "aa:00:00:00:00:05"

/**
 * Joining another player's group from *Play on…* (2026-10-15): the controlled player joins
 * the group chosen (`<chosen> sync <this player>` in LMS's terms), or leaves its own.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class JoinGroupViewModelTest : PlayerTestBase() {
    private fun messagesOf(vm: io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel, scope: kotlinx.coroutines.CoroutineScope): MutableList<PlayerMessage> {
        val out = mutableListOf<PlayerMessage>()
        scope.launch { vm.messages.collect { out += it } }
        return out
    }

    @Test
    fun joiningSendsTheChosenPlayerTakingThisOneInAndShowsItAtOnce() = runTest(dispatcher) {
        val vm = vm()
        try {
            vm.start()
            phoneListed()
            runCurrent()
            vm.select(KITCHEN)
            runCurrent()
            assertTrue(JoinGroup.partners(vm.state.value).isEmpty())
            val gate = CompletableDeferred<Unit>()
            players.syncGate = gate
            vm.joinGroupOf(LOUNGE)
            // Shown at once: Kitchen is now with Lounge.
            assertEquals(listOf(LOUNGE), JoinGroup.partners(vm.state.value))
            players.groups = listOf(SyncGroup(listOf(LOUNGE, KITCHEN)))
            gate.complete(Unit)
            runCurrent()
            // Kitchen joins Lounge's group: `<Lounge> sync <Kitchen>` (the argument joins the
            // subject's group), never the other way round, which would pull Lounge onto Kitchen.
            assertEquals(listOf("sync $LOUNGE $KITCHEN true"), players.commands.filter { it.startsWith("sync") })
            assertEquals(listOf(SyncGroup(listOf(LOUNGE, KITCHEN))), vm.state.value.syncGroups)
        } finally {
            vm.stop()
        }
    }

    @Test
    fun leavingTakesOnlyThisPlayerOut() = runTest(dispatcher) {
        players.groups = listOf(SyncGroup(listOf(KITCHEN, LOUNGE, STUDY)))
        players.players = players.players + samplePlayer(STUDY)
        val vm = vm()
        try {
            vm.start()
            runCurrent()
            vm.select(KITCHEN)
            vm.loadOutputs()
            runCurrent()
            assertEquals(listOf(LOUNGE, STUDY), JoinGroup.partners(vm.state.value))
            val gate = CompletableDeferred<Unit>()
            players.syncGate = gate
            vm.leaveGroup()
            assertTrue(JoinGroup.partners(vm.state.value).isEmpty())
            // The others stay together.
            assertEquals(listOf(SyncGroup(listOf(LOUNGE, STUDY))), vm.state.value.syncGroups)
            players.groups = listOf(SyncGroup(listOf(LOUNGE, STUDY)))
            gate.complete(Unit)
            runCurrent()
            assertTrue(players.commands.toString(), "sync $KITCHEN $KITCHEN false" in players.commands)
        } finally {
            vm.stop()
        }
    }

    @Test
    fun aRefusedJoinIsPutBackAndSaid() = runTest(dispatcher) {
        val vm = vm()
        try {
            val got = messagesOf(vm, backgroundScope)
            vm.start()
            runCurrent()
            vm.select(KITCHEN)
            runCurrent()
            players.commandResult = ApiResult.Err(AppError.Forbidden("player not permitted"))
            vm.joinGroupOf(LOUNGE)
            runCurrent()
            assertTrue(JoinGroup.partners(vm.state.value).isEmpty())
            assertTrue(got.any { it is PlayerMessage.Failed })
        } finally {
            vm.stop()
        }
    }

    @Test
    fun anAnswerForAPlayerNoLongerControlledIsNotApplied() = runTest(dispatcher) {
        val vm = vm()
        try {
            vm.start()
            phoneListed()
            runCurrent()
            vm.select(KITCHEN)
            runCurrent()
            val gate = CompletableDeferred<Unit>()
            players.syncGate = gate
            players.commandResult = ApiResult.Err(AppError.Offline("gone"))
            vm.joinGroupOf(LOUNGE)
            runCurrent()
            val shown = vm.state.value.syncGroups
            // The user controls another player before the answer comes.
            vm.select(PHONE)
            runCurrent()
            players.groups = listOf(SyncGroup(listOf(STUDY, HALL)))
            gate.complete(Unit)
            runCurrent()
            // Kitchen's late answer changes nothing on screen (Play on… reads the groups afresh).
            assertEquals(PHONE, vm.state.value.selectedId)
            assertEquals(shown, vm.state.value.syncGroups)
        } finally {
            vm.stop()
        }
    }

    @Test
    fun aLaterChangeIsNotUndoneByAnEarlierFailure() = runTest(dispatcher) {
        players.players = players.players + samplePlayer(STUDY)
        val vm = vm()
        try {
            vm.start()
            runCurrent()
            vm.select(KITCHEN)
            runCurrent()
            val first = CompletableDeferred<Unit>()
            players.syncGate = first
            vm.joinGroupOf(LOUNGE)
            runCurrent()
            val second = CompletableDeferred<Unit>()
            players.syncGate = second
            vm.joinGroupOf(STUDY)
            runCurrent()
            assertEquals(listOf(STUDY), JoinGroup.partners(vm.state.value))
            players.commandResult = ApiResult.Err(AppError.Offline("gone"))
            first.complete(Unit)
            runCurrent()
            // The first join's failure doesn't put back the groups from before the second.
            assertEquals(listOf(STUDY), JoinGroup.partners(vm.state.value))
        } finally {
            vm.stop()
        }
    }

    @Test
    fun offlineOrBusyNothingIsSent() = runTest(dispatcher) {
        val vm = vm()
        try {
            vm.start()
            runCurrent()
            vm.select(KITCHEN)
            runCurrent()
            vm.setConnection(ConnectionUi(unreachable = true))
            assertEquals(SyncBlock.OFFLINE, JoinGroup.block(vm.state.value))
            vm.joinGroupOf(LOUNGE)
            vm.leaveGroup()
            vm.setConnection(ConnectionUi(busy = true))
            assertEquals(SyncBlock.BUSY, JoinGroup.block(vm.state.value))
            vm.joinGroupOf(LOUNGE)
            runCurrent()
            assertFalse(players.commands.toString(), players.commands.any { it.startsWith("sync") })
            vm.setConnection(ConnectionUi())
            assertNull(JoinGroup.block(vm.state.value))
        } finally {
            vm.stop()
        }
    }

    @Test
    fun nothingIsSentForItselfAnUnknownPlayerOrWithoutAControlledPlayer() = runTest(dispatcher) {
        val vm = vm()
        try {
            vm.joinGroupOf(LOUNGE)
            vm.leaveGroup()
            vm.start()
            runCurrent()
            vm.select(KITCHEN)
            runCurrent()
            vm.joinGroupOf(KITCHEN)
            vm.joinGroupOf("bb:00:00:00:00:99")
            // Leaving when not in a group: nothing to do.
            vm.leaveGroup()
            runCurrent()
            assertFalse(players.commands.toString(), players.commands.any { it.startsWith("sync") })
        } finally {
            vm.stop()
        }
    }
}

/** The picker's model: other players, a group as one entry, this player's own group marked. */
class JoinGroupModelTest {
    private val all = listOf(samplePlayer(PHONE), samplePlayer(KITCHEN), samplePlayer(LOUNGE), samplePlayer(STUDY), samplePlayer(HALL))

    private fun state(selected: String, groups: List<SyncGroup>) =
        PlayerUiState(players = all, selectedId = selected, syncGroups = groups, loading = false)

    @Test
    fun groupsAreOneEntryAndTheOwnGroupIsMarked() {
        val s = state(KITCHEN, listOf(SyncGroup(listOf(KITCHEN, LOUNGE)), SyncGroup(listOf(STUDY, HALL))))
        assertEquals(
            // The own group first; a group is joined through its first member.
            listOf(
                SyncTarget(KITCHEN, listOf(KITCHEN, LOUNGE), own = true),
                SyncTarget(PHONE, listOf(PHONE), own = false),
                SyncTarget(STUDY, listOf(STUDY, HALL), own = false),
            ),
            JoinGroup.targets(s),
        )
        assertEquals(listOf(LOUNGE), JoinGroup.partners(s))
    }

    @Test
    fun aGroupIsJoinedThroughItsFirstMember() {
        // LMS lists Hall first: Hall is the group's subject, whatever the players' order.
        val s = state(KITCHEN, listOf(SyncGroup(listOf(HALL, STUDY))))
        assertEquals(SyncTarget(HALL, listOf(HALL, STUDY), own = false), JoinGroup.targets(s).single { STUDY in it.memberIds })
    }

    @Test
    fun aloneEveryOtherPlayerIsItsOwnEntry() {
        val s = state(KITCHEN, emptyList())
        assertEquals(listOf(PHONE, LOUNGE, STUDY, HALL), JoinGroup.targets(s).map { it.playerId })
        assertTrue(JoinGroup.targets(s).none { it.own })
        assertTrue(JoinGroup.partners(s).isEmpty())
    }

    @Test
    fun unknownIdsInAGroupAreIgnored() {
        val s = state(KITCHEN, listOf(SyncGroup(listOf("bb:00:00:00:00:99", STUDY))))
        assertEquals(SyncTarget(STUDY, listOf(STUDY), own = false), JoinGroup.targets(s).first { it.playerId == STUDY })
        assertEquals(1, JoinGroup.targets(s).count { STUDY in it.memberIds })
        assertTrue(JoinGroup.targets(state(KITCHEN, listOf(SyncGroup(listOf("bb:00:00:00:00:99", KITCHEN))))).none { it.own })
    }
}

/** The Play on… sheet's join entries (phone and large windows share the one list). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w411dp-h1800dp")
class JoinGroupSheetTests {
    @get:Rule
    val compose = createComposeRule()

    private val joined = mutableListOf<String>()
    private var left = 0
    private val synced = mutableListOf<Pair<String, Boolean>>()

    private fun actions() = PlayOnActions(
        onControl = {},
        onBackToPhone = {},
        onSync = { id, on -> synced += id to on },
        onJoinGroup = { joined += it },
        onLeaveGroup = { left++ },
    )

    private fun show(state: PlayerUiState) = compose.setContent { LyrionTheme(dynamicColour = false) { PlayOnList(state, actions()) } }

    private fun alone() = auditPlayerState(KITCHEN).copy(syncGroups = emptyList())

    private fun grouped() = auditPlayerState(KITCHEN).copy(syncGroups = listOf(SyncGroup(listOf(KITCHEN, LOUNGE))))

    @Test
    fun theRowSaysSyncWithWhenAlone() {
        show(alone())
        compose.onNodeWithTag("sync-row").assertIsDisplayed().assertIsEnabled().assertTextContains("Sync with…")
    }

    @Test
    fun theRowNamesTheGroupWhenGrouped() {
        show(grouped())
        compose.onNodeWithTag("sync-row").assertTextContains("In sync with Player $LOUNGE")
    }

    @Test
    fun theRowSitsUnderThePhoneWhenThePhoneIsControlled() {
        show(auditPlayerState(PHONE).copy(syncGroups = emptyList()))
        compose.onNodeWithTag("sync-row").assertTextContains("Sync with…")
    }

    @Test
    fun thePickerCollapsesGroupsMarksTheOwnAndJoins() {
        show(
            auditPlayerState(KITCHEN).copy(
                players = auditPlayerState().players + samplePlayer(STUDY) + samplePlayer(HALL),
                syncGroups = listOf(SyncGroup(listOf(KITCHEN, LOUNGE)), SyncGroup(listOf(STUDY, HALL))),
            ),
        )
        compose.onNodeWithTag("sync-row").performClick()
        compose.onNodeWithText("Sync Player $KITCHEN with…").assertIsDisplayed()
        compose.onNodeWithText("Player $KITCHEN joins that group and plays what it plays.").assertIsDisplayed()
        // Study and Hall: one entry naming the group.
        compose.onNodeWithTag("join-$STUDY").assertTextContains("Player $STUDY + Player $HALL", substring = true)
        compose.onAllNodesWithTag("join-$HALL").assertCountEquals(0)
        // This phone is offered too, by that name.
        compose.onNodeWithTag("join-$PHONE").assertTextContains("This phone", substring = true)
        // Kitchen's own group is marked and isn't a choice.
        compose.onNodeWithTag("join-own").assertTextContains("Player $KITCHEN + Player $LOUNGE", substring = true)
        compose.onNodeWithTag("join-own").assertTextContains("This group", substring = true).assertIsNotEnabled()
        compose.assertAccessible()
        compose.onNodeWithTag("join-$STUDY").performClick()
        // The group is joined through its first member.
        assertEquals(listOf(STUDY), joined)
        // Chosen: the picker closes.
        compose.onAllNodesWithTag("join-$PHONE").assertCountEquals(0)
    }

    @Test
    fun leaveTheGroupFromThePicker() {
        show(grouped())
        compose.onNodeWithTag("sync-row").performClick()
        compose.onNodeWithTag("leave-group").assertTextContains("Leave the group").performClick()
        assertEquals(1, left)
    }

    @Test
    fun aloneThereIsNoLeave() {
        show(alone())
        compose.onNodeWithTag("sync-row").performClick()
        compose.onAllNodesWithTag("leave-group").assertCountEquals(0)
        compose.onAllNodesWithTag("join-own").assertCountEquals(0)
    }

    @Test
    fun eachOtherRowsMenuJoinsItsGroupOrLeavesOurs() {
        show(alone())
        compose.onNodeWithTag("play-on-$LOUNGE-more").performClick()
        compose.onNodeWithTag("play-on-$LOUNGE-join").assertTextContains("Join its group").performClick()
        assertEquals(listOf(LOUNGE), joined)
    }

    @Test
    fun aRowAlreadyWithUsOffersLeaveTheGroup() {
        show(grouped())
        compose.onNodeWithTag("play-on-$LOUNGE-more").performClick()
        compose.onAllNodesWithTag("play-on-$LOUNGE-join").assertCountEquals(0)
        compose.onNodeWithTag("play-on-$LOUNGE-leave").assertTextContains("Leave the group").performClick()
        assertEquals(1, left)
    }

    @Test
    fun offlineTheRowIsDisabledAndSaysWhy() {
        show(alone().copy(connection = ConnectionUi(unreachable = true)))
        compose.onNodeWithTag("sync-row").assertIsNotEnabled()
        compose.onNodeWithText("Can’t change groups while your music server is out of reach.").assertIsDisplayed()
        compose.onNodeWithTag("play-on-$LOUNGE-more").performClick()
        compose.onNodeWithTag("play-on-$LOUNGE-join").assertIsNotEnabled()
    }

    @Test
    fun busyTheRowIsDisabledAndSaysWhy() {
        show(alone().copy(connection = ConnectionUi(busy = true)))
        compose.onNodeWithTag("sync-row").assertIsNotEnabled()
        compose.onNodeWithText("Your music server is busy. Try again in a moment.").assertIsDisplayed()
    }

    @Test
    fun groupPlayersSaysWhichWayItWorks() {
        show(alone())
        compose.onNodeWithTag("group-players").assertTextContains("Bring players into step with Player $KITCHEN…", substring = true).performClick()
        compose.onNodeWithContentDescription("Bring Player $LOUNGE into step").assertExists()
        compose.onNodeWithTag("sync-$LOUNGE").performClick()
        assertEquals(listOf(LOUNGE to true), synced)
    }

    @Test
    fun theSheetIsAccessible() {
        show(grouped())
        compose.assertAccessible()
    }
}

/** A large window (tablet): the same entries in the same sheet. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w1280dp-h800dp")
class JoinGroupSheetLargeTests {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theRowAndPickerAreThereOnALargeWindow() {
        val joined = mutableListOf<String>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                PlayOnList(
                    auditPlayerState(KITCHEN).copy(syncGroups = emptyList()),
                    PlayOnActions(onControl = {}, onBackToPhone = {}, onSync = { _, _ -> }, onJoinGroup = { joined += it }, onLeaveGroup = {}, onAllRooms = {}),
                )
            }
        }
        compose.onNodeWithTag("sync-row").assertTextContains("Sync with…").performClick()
        compose.onNodeWithTag("join-$LOUNGE").performClick()
        assertEquals(listOf(LOUNGE), joined)
    }
}

private fun androidx.compose.ui.test.SemanticsNodeInteractionCollection.assertCountEquals(n: Int) {
    assertEquals(n, fetchSemanticsNodes().size)
}
