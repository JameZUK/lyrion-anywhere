package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerChoice
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerChoice.Start
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import kotlinx.coroutines.ExperimentalCoroutinesApi
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

/** D20, "player-first": the rules on their own. */
class PlayerChoiceTest {
    private val all = listOf(samplePlayer(KITCHEN), samplePlayer(LOUNGE, playing = true), samplePlayer(PHONE))

    @Test
    fun thisPhoneIsTheDefault() {
        assertEquals(Start.Control(PHONE), PlayerChoice.start(all, PHONE, true, AppSettings()))
        assertTrue(PlayerChoice.wantsPhone(AppSettings()))
        // Not listed yet: wait for it rather than grab another player.
        val notYet = all.filter { it.id != PHONE }
        assertEquals(Start.WaitForPhone, PlayerChoice.start(notYet, PHONE, true, AppSettings()))
        assertEquals(Start.WaitForPhone, PlayerChoice.start(notYet, null, true, AppSettings()))
    }

    @Test
    fun lastUsedAndSpecificFallBackToThePhone() {
        val last = AppSettings(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = KITCHEN)
        assertEquals(Start.Control(KITCHEN), PlayerChoice.start(all, PHONE, true, last))
        assertFalse(PlayerChoice.wantsPhone(last))
        val specific = AppSettings(defaultPlayer = DefaultPlayer.SPECIFIC, defaultPlayerId = LOUNGE, selectedPlayerId = KITCHEN)
        assertEquals(Start.Control(LOUNGE), PlayerChoice.start(all, PHONE, true, specific))
        // Not available now: this phone.
        val gone = all.filter { it.id != LOUNGE }
        assertEquals(Start.Control(PHONE), PlayerChoice.start(gone, PHONE, true, specific))
    }

    @Test
    fun withoutThePhoneTheLastUsedThenAPlayingPlayer() {
        val noPhone = all.filter { it.id != PHONE }
        assertEquals(Start.Control(KITCHEN), PlayerChoice.start(noPhone, null, false, AppSettings(selectedPlayerId = KITCHEN)))
        assertEquals(Start.Control(LOUNGE), PlayerChoice.start(noPhone, null, false, AppSettings()))
        assertEquals(Start.Nothing, PlayerChoice.start(emptyList(), null, false, AppSettings()))
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class PlayerFirstViewModelTest : MainDispatcherTest() {
    private fun vm(players: FakePlayers, local: FakeLocalPlayer, settings: FakeSettings = FakeSettings()) =
        PlayerViewModel(players, FakeEvents(), settings, local) { 0L }

    /** The phone's player says hello and LMS lists it. */
    private fun phoneAppears(players: FakePlayers, local: FakeLocalPlayer) {
        players.players = players.players + samplePlayer(PHONE)
        local.state.value = local.state.value.copy(status = localStatus(PHONE))
    }

    @Test
    fun startsOnThisPhoneSwitchingItOnAndWaitingForIt() = runTest(dispatcher) {
        val players = FakePlayers()
        val local = FakeLocalPlayer()
        val vm = vm(players, local)
        vm.start()
        assertEquals(listOf(true), local.enabled)
        // Not another player while the phone starts.
        assertNull(vm.state.value.selectedId)
        assertTrue(vm.state.value.waitingForPhone)
        phoneAppears(players, local)
        runCurrent()
        assertEquals(PHONE, vm.state.value.selectedId)
        assertTrue(vm.state.value.localSelected)
        assertFalse(vm.state.value.controllingOther)
        assertFalse(vm.state.value.waitingForPhone)
        vm.stop()
    }

    @Test
    fun controllingAnotherPlayerSaysSoAndBackToThePhoneReturns() = runTest(dispatcher) {
        val players = FakePlayers()
        val local = FakeLocalPlayer()
        val vm = vm(players, local)
        vm.start()
        phoneAppears(players, local)
        runCurrent()
        vm.select(KITCHEN)
        assertTrue(vm.state.value.controllingOther)
        assertEquals("Player $KITCHEN", vm.state.value.selected?.name)
        // The user's choice holds when the list is read again.
        vm.refresh()
        assertEquals(KITCHEN, vm.state.value.selectedId)
        vm.backToPhone()
        assertEquals(PHONE, vm.state.value.selectedId)
        assertFalse(vm.state.value.controllingOther)
        vm.stop()
    }

    @Test
    fun backToThePhoneStartsItsPlayerIfItIsOff() = runTest(dispatcher) {
        val players = FakePlayers()
        val local = FakeLocalPlayer()
        val settings = FakeSettings(AppSettings(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = KITCHEN))
        val vm = vm(players, local, settings)
        vm.start()
        // "Last used" doesn't switch the phone on by itself.
        assertTrue(local.enabled.isEmpty())
        assertEquals(KITCHEN, vm.state.value.selectedId)
        assertTrue(vm.state.value.controllingOther)
        vm.backToPhone()
        assertEquals(listOf(true), local.enabled)
        assertTrue(vm.state.value.waitingForPhone)
        phoneAppears(players, local)
        runCurrent()
        assertEquals(PHONE, vm.state.value.selectedId)
        vm.stop()
    }

    @Test
    fun aSpecificDefaultPlayerIsUsedAndTheSettingIsSaved() = runTest(dispatcher) {
        val players = FakePlayers()
        val local = FakeLocalPlayer()
        val settings = FakeSettings(AppSettings(defaultPlayer = DefaultPlayer.SPECIFIC, defaultPlayerId = LOUNGE))
        val vm = vm(players, local, settings)
        vm.start()
        assertEquals(LOUNGE, vm.state.value.selectedId)
        assertEquals(DefaultPlayer.SPECIFIC, vm.state.value.defaultPlayer)
        vm.setDefaultPlayer(DefaultPlayer.THIS_PHONE)
        assertEquals(DefaultPlayer.THIS_PHONE, settings.state.value.defaultPlayer)
        assertEquals(DefaultPlayer.THIS_PHONE, vm.state.value.defaultPlayer)
        vm.setDefaultPlayer(DefaultPlayer.SPECIFIC, KITCHEN)
        assertEquals(KITCHEN, settings.state.value.defaultPlayerId)
        vm.stop()
    }

    @Test
    fun thePlayOnSheetListsOtherPlayersWhatTheyPlayAndGroups() = runTest(dispatcher) {
        val players = FakePlayers().apply { groups = listOf(SyncGroup(listOf(KITCHEN, LOUNGE))) }
        val local = FakeLocalPlayer()
        val vm = vm(players, local)
        vm.start()
        phoneAppears(players, local)
        runCurrent()
        vm.loadOutputs()
        val s = vm.state.value
        assertEquals(listOf(KITCHEN, LOUNGE), s.otherPlayers.map { it.id })
        assertEquals("Track 1", s.playingOn[LOUNGE]?.title)
        assertEquals(1, s.syncGroups.size)
        vm.stop()
    }
}
