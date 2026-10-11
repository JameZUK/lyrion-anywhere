package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.TransferOutcome
import io.github.jamezuk.lyrionanywhere.core.TransferRefusal
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.NowPlayingHub
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEdit
import io.github.jamezuk.lyrionanywhere.ui.player.SyncChoice
import io.github.jamezuk.lyrionanywhere.ui.player.TransferState
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val PHONE = "02:4c:41:00:00:01"
private const val KITCHEN = "aa:00:00:00:00:01"
private const val LOUNGE = "aa:00:00:00:00:02"

/** Collects the view model's one-off messages while a test runs. */
private fun TestScope.messages(flow: Flow<PlayerMessage>): MutableList<PlayerMessage> {
    val out = mutableListOf<PlayerMessage>()
    backgroundScope.launch { flow.collect { out += it } }
    return out
}

@OptIn(ExperimentalCoroutinesApi::class)
abstract class PlayerTestBase : MainDispatcherTest() {
    protected val players = FakePlayers()
    protected val local = FakeLocalPlayer()

    protected fun vm(hub: NowPlayingHub? = null) =
        PlayerViewModel(players, FakeEvents(), FakeSettings(), local, nowPlaying = hub) { 0L }

    /** The phone's player has said hello and LMS lists it. */
    protected fun phoneListed() {
        players.players = players.players + samplePlayer(PHONE)
        local.state.value = local.state.value.copy(enabled = true, status = localStatus(PHONE))
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class TransferViewModelTest : PlayerTestBase() {
    @Test
    fun moveHereRunsTheRecipeFromTheControlledPlayerAndControlsTheTarget() = runTest(dispatcher) {
        val vm = vm()
        val got = messages(vm.messages)
        vm.start()
        phoneListed()
        runCurrent()
        assertEquals(PHONE, vm.state.value.selectedId)
        val gate = CompletableDeferred<Unit>()
        players.transferGate = gate
        vm.moveTo(KITCHEN)
        // While it runs: the last stage reported, for the target by name.
        val running = vm.state.value.transfer as TransferState.Running
        assertEquals(KITCHEN, running.targetId)
        assertEquals("Player $KITCHEN", running.targetName)
        assertEquals(TransferStage.STARTING, running.stage)
        // A second tap while it runs does nothing.
        vm.moveTo(LOUNGE)
        assertEquals(listOf(PHONE to KITCHEN), players.transfers)
        gate.complete(Unit)
        runCurrent()
        assertEquals(TransferState.Idle, vm.state.value.transfer)
        assertEquals(KITCHEN, vm.state.value.selectedId)
        assertTrue(vm.state.value.controllingOther)
        assertTrue(got.contains(PlayerMessage.Moved("Player $KITCHEN")))
        vm.stop()
    }

    @Test
    fun bringToThisPhoneMovesFromTheControlledPlayerAndControlsThePhoneAgain() = runTest(dispatcher) {
        val vm = vm()
        vm.start()
        phoneListed()
        runCurrent()
        vm.select(KITCHEN)
        vm.bringToPhone()
        runCurrent()
        assertEquals(listOf(KITCHEN to PHONE), players.transfers)
        assertEquals(PHONE, vm.state.value.selectedId)
        assertTrue(vm.state.value.localSelected)
        vm.stop()
    }

    @Test
    fun bringToThisPhoneBeforeThePhoneIsAPlayerStartsItAndSaysSo() = runTest(dispatcher) {
        val vm = vm()
        players.players = listOf(samplePlayer(KITCHEN))
        vm.start()
        vm.select(KITCHEN)
        local.enabled.clear()
        local.state.value = local.state.value.copy(enabled = false)
        vm.bringToPhone()
        assertEquals(TransferState.PhoneNotReady, vm.state.value.transfer)
        assertEquals(listOf(true), local.enabled)
        assertTrue(players.transfers.isEmpty())
        vm.dismissTransfer()
        assertEquals(TransferState.Idle, vm.state.value.transfer)
        vm.stop()
    }

    @Test
    fun refusalsAndErrorsStayUntilDismissedAndNothingIsSelected() = runTest(dispatcher) {
        val vm = vm()
        vm.start()
        phoneListed()
        runCurrent()
        players.transferResult = ApiResult.Ok(TransferOutcome.Refused(TransferRefusal.REMOTE))
        vm.moveTo(KITCHEN)
        runCurrent()
        assertEquals(TransferState.Refused("Player $KITCHEN", TransferRefusal.REMOTE), vm.state.value.transfer)
        assertEquals(PHONE, vm.state.value.selectedId)

        val refused = AppError.Forbidden("player not permitted")
        players.transferResult = ApiResult.Err(refused)
        vm.moveTo(LOUNGE)
        runCurrent()
        assertEquals(TransferState.Failed("Player $LOUNGE", refused), vm.state.value.transfer)
        assertEquals(PHONE, vm.state.value.selectedId)
        vm.dismissTransfer()
        assertEquals(TransferState.Idle, vm.state.value.transfer)
        vm.stop()
    }

    @Test
    fun withoutAControlledPlayerThereIsNothingToMove() = runTest(dispatcher) {
        val vm = vm()
        val got = messages(vm.messages)
        vm.moveTo(KITCHEN)
        runCurrent()
        assertTrue(players.transfers.isEmpty())
        assertEquals(listOf<PlayerMessage>(PlayerMessage.NoPlayer), got)
    }

    @Test
    fun theWidgetLearnsWhatTheControlledPlayerPlays() = runTest(dispatcher) {
        val hub = NowPlayingHub()
        val vm = vm(hub)
        vm.start()
        phoneListed()
        runCurrent()
        val n = hub.selected.value!!
        assertEquals(PHONE, n.playerId)
        assertEquals("Track 1", n.title)
        assertTrue(n.playing)
        assertTrue(n.isPhone)
        players.statusResult = ApiResult.Ok(sampleStatus(PlaybackMode.PAUSED))
        vm.select(KITCHEN)
        runCurrent()
        assertEquals(KITCHEN, hub.selected.value?.playerId)
        assertFalse(hub.selected.value!!.playing)
        assertFalse(hub.selected.value!!.isPhone)
        vm.stop()
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class QueueEditViewModelTest : PlayerTestBase() {
    private suspend fun TestScope.ready(): PlayerViewModel {
        val vm = vm()
        vm.start()
        phoneListed()
        runCurrent()
        vm.loadQueue()
        runCurrent()
        assertEquals(listOf(1L, 2L), vm.state.value.queue.map { it.track.id })
        return vm
    }

    @Test
    fun removingShowsAtOnceSendsTheIndexAndOffersUndo() = runTest(dispatcher) {
        val vm = ready()
        val got = messages(vm.messages)
        vm.removeFromQueue(0)
        assertTrue("remove 0 $PHONE" in players.commands)
        val removed = got.filterIsInstance<PlayerMessage.Removed>().single()
        assertEquals("Track 1", removed.title)
        assertTrue(removed.canUndo)

        vm.undoRemove(removed.token)
        runCurrent()
        assertTrue("restore 1 0 $PHONE" in players.commands)
        // Undo is used up.
        vm.undoRemove(removed.token)
        assertEquals(1, players.commands.count { it.startsWith("restore") })
        vm.stop()
    }

    @Test
    fun anOlderUndoOrAnotherPlayersDoesNothing() = runTest(dispatcher) {
        val vm = ready()
        val got = messages(vm.messages)
        vm.removeFromQueue(0)
        vm.removeFromQueue(0)
        val (first, second) = got.filterIsInstance<PlayerMessage.Removed>()
        vm.undoRemove(first.token)
        assertFalse(players.commands.any { it.startsWith("restore") })
        // Controlling another player since: its queue is a different list.
        vm.select(KITCHEN)
        vm.undoRemove(second.token)
        assertFalse(players.commands.any { it.startsWith("restore") })
        vm.stop()
    }

    @Test
    fun aFailedEditPutsTheListBackAndSaysWhy() = runTest(dispatcher) {
        val vm = ready()
        val got = messages(vm.messages)
        val before = vm.state.value.queue
        val error = AppError.Server(500, "LMS said no")
        players.commandResult = ApiResult.Err(error)
        vm.moveInQueue(0, 1)
        runCurrent()
        assertEquals(before.map { it.track.id }, vm.state.value.queue.map { it.track.id })
        assertTrue(got.contains(PlayerMessage.QueueFailed(error)))
        vm.removeFromQueue(1)
        runCurrent()
        assertEquals(2, vm.state.value.queue.size)
        assertFalse(got.any { it is PlayerMessage.Removed })
        vm.stop()
    }

    @Test
    fun aFailedUndoIsReportedAsSuch() = runTest(dispatcher) {
        val vm = ready()
        val got = messages(vm.messages)
        vm.removeFromQueue(1)
        val removed = got.filterIsInstance<PlayerMessage.Removed>().single()
        val error = AppError.Server(409, "the queue changed meanwhile, so the track was added at the end")
        players.restoreResult = ApiResult.Err(error)
        vm.undoRemove(removed.token)
        runCurrent()
        assertTrue(got.contains(PlayerMessage.QueueFailed(error, undo = true)))
        vm.stop()
    }

    @Test
    fun movingAndClearingSendTheRightCommands() = runTest(dispatcher) {
        val vm = ready()
        vm.moveInQueue(1, 0)
        assertTrue("move 1 0 $PHONE" in players.commands)
        // Out of range or no move: nothing sent.
        vm.moveInQueue(0, 0)
        vm.moveInQueue(0, 5)
        vm.moveInQueue(-1, 0)
        assertEquals(1, players.commands.count { it.startsWith("move") })
        vm.clearQueue()
        assertTrue("clear $PHONE" in players.commands)
        vm.stop()
    }

    @Test
    fun radioAndAppItemsCanBeRemovedButNotPutBack() = runTest(dispatcher) {
        val vm = ready()
        val got = messages(vm.messages)
        // A remote entry has no library id.
        val radio = vm.state.value.queue[0].track.copy(id = null, title = "Radio")
        assertFalse(QueueEdit.canRestore(radio))
        assertTrue(QueueEdit.canRestore(vm.state.value.queue[0].track))
        vm.removeFromQueue(7)
        assertTrue(got.isEmpty())
        vm.stop()
    }
}

class QueueEditTest {
    @Test
    fun reorderMatchesLmsPlaylistMove() {
        val l = listOf("a", "b", "c", "d")
        assertEquals(listOf("b", "c", "a", "d"), QueueEdit.reorder(l, 0, 2))
        assertEquals(listOf("d", "a", "b", "c"), QueueEdit.reorder(l, 3, 0))
        // Out of range or no move: unchanged.
        for ((from, to) in listOf(0 to 0, -1 to 2, 0 to 4, 9 to 1)) assertEquals(l, QueueEdit.reorder(l, from, to))
    }

    @Test
    fun removeAndInsertAreSafeAtTheEdges() {
        val l = listOf(1, 2, 3)
        assertEquals(listOf(1, 3), QueueEdit.removeAt(l, 1))
        assertEquals(l, QueueEdit.removeAt(l, 3))
        assertEquals(l, QueueEdit.removeAt(l, -1))
        assertEquals(listOf(0, 1, 2, 3), QueueEdit.insertAt(l, -5, 0))
        assertEquals(listOf(1, 2, 3, 4), QueueEdit.insertAt(l, 99, 4))
        assertEquals(listOf(1, 9, 2, 3), QueueEdit.insertAt(l, 1, 9))
    }

    @Test
    fun aDragLandsOnTheNearestRowWithinTheList() {
        assertEquals(2, QueueEdit.dropIndex(0, 130f, 64f, 5))
        assertEquals(0, QueueEdit.dropIndex(0, 20f, 64f, 5))
        assertEquals(0, QueueEdit.dropIndex(3, -1000f, 64f, 5))
        assertEquals(4, QueueEdit.dropIndex(3, 1000f, 64f, 5))
        // Nonsense measurements never move anything.
        assertEquals(2, QueueEdit.dropIndex(2, Float.NaN, 64f, 5))
        assertEquals(2, QueueEdit.dropIndex(2, 100f, 0f, 5))
        assertEquals(2, QueueEdit.dropIndex(2, 100f, Float.POSITIVE_INFINITY, 5))
        assertEquals(7, QueueEdit.dropIndex(7, 100f, 64f, 5))
        assertEquals(0, QueueEdit.dropIndex(0, 100f, 64f, 0))
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class SyncViewModelTest : PlayerTestBase() {
    @Test
    fun joiningIsSentForTheControlledPlayerNamingTheOther() = runTest(dispatcher) {
        val vm = vm()
        vm.start()
        phoneListed()
        runCurrent()
        vm.select(KITCHEN)
        vm.setSynced(LOUNGE, true)
        assertTrue("sync $KITCHEN $LOUNGE true" in players.commands)
        vm.setSynced(LOUNGE, false)
        assertTrue("sync $KITCHEN $LOUNGE false" in players.commands)
        vm.stop()
    }

    @Test
    fun theSheetShowsTheChangeAtOnceAndPutsItBackOnFailure() = runTest(dispatcher) {
        val vm = vm()
        val got = messages(vm.messages)
        vm.start()
        phoneListed()
        runCurrent()
        vm.select(KITCHEN)
        val gate = CompletableDeferred<Unit>()
        players.syncGate = gate
        players.commandResult = ApiResult.Err(AppError.Forbidden("player not permitted"))
        vm.setSynced(LOUNGE, true)
        // Shown at once, before LMS answers.
        assertTrue(SyncChoice.together(KITCHEN, LOUNGE, vm.state.value.syncGroups))
        gate.complete(Unit)
        runCurrent()
        // Refused: put back, and said.
        assertFalse(SyncChoice.together(KITCHEN, LOUNGE, vm.state.value.syncGroups))
        assertTrue(got.any { it is PlayerMessage.Failed })
        vm.stop()
    }

    @Test
    fun withoutAControlledPlayerNothingIsSent() = runTest(dispatcher) {
        val vm = vm()
        vm.setSynced(LOUNGE, true)
        assertFalse(players.commands.any { it.startsWith("sync") })
    }
}

class SyncChoiceTest {
    private val g = listOf(SyncGroup(listOf(KITCHEN, LOUNGE)))

    @Test
    fun togetherOnlyInTheSameGroup() {
        assertTrue(SyncChoice.together(KITCHEN, LOUNGE, g))
        assertFalse(SyncChoice.together(KITCHEN, PHONE, g))
        assertFalse(SyncChoice.together(KITCHEN, KITCHEN, g))
        assertFalse(SyncChoice.together(null, LOUNGE, g))
    }

    @Test
    fun toggledMatchesWhatLmsWillDo() {
        // Joining a group of one makes a group of two.
        assertEquals(listOf(SyncGroup(listOf(PHONE, KITCHEN))), SyncChoice.toggled(emptyList(), PHONE, KITCHEN, true))
        // Joining an existing group adds to it; the joiner leaves its old group.
        assertEquals(
            listOf(SyncGroup(listOf(KITCHEN, LOUNGE, PHONE))),
            SyncChoice.toggled(g, KITCHEN, PHONE, true),
        )
        val moved = SyncChoice.toggled(listOf(SyncGroup(listOf(KITCHEN, LOUNGE)), SyncGroup(listOf(PHONE, "aa:00:00:00:00:09"))), KITCHEN, PHONE, true)
        assertEquals(listOf(SyncGroup(listOf(KITCHEN, LOUNGE, PHONE))), moved)
        // Leaving a group of two ends it.
        assertTrue(SyncChoice.toggled(g, KITCHEN, LOUNGE, false).isEmpty())
        // Itself: unchanged.
        assertEquals(g, SyncChoice.toggled(g, KITCHEN, KITCHEN, true))
        assertNull(SyncChoice.toggled(g, KITCHEN, LOUNGE, false).firstOrNull())
    }
}
