package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.SleepEnd
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.SleepChoice
import io.github.jamezuk.lyrionanywhere.ui.player.SleepScope
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The sleep timer (Wave 1 §1) and "Up next" (§3) in the player view model: what is sent to
 * LMS, re-arming, "sleep all", cancelling, this phone's fade, and removing the next track
 * with Undo.
 */
class Wave1ViewModelTest : MainDispatcherTest() {
    private var now = 1_000_000L
    private val me = "aa:00:00:00:00:02" // the playing one, selected at start

    private fun track(id: Long, album: Long, duration: Double?) =
        Track(id, "Track $id", "Artist", "Album", album, 3, duration, null, null, null, null, null, null, null, null)

    private fun setup(local: FakeLocalPlayer = FakeLocalPlayer(), settings: FakeSettings = FakeSettings()): Triple<PlayerViewModel, FakePlayers, FakeLocalPlayer> {
        val players = FakePlayers()
        players.statusResult = ApiResult.Ok(
            sampleStatus(elapsed = 100.0).copy(durationSecs = 300.0, queueIndex = 0u, queueLength = 3u, current = track(1, 7, 300.0)),
        )
        players.upcoming = listOf(track(2, 7, 200.0), track(3, 8, 400.0))
        // Control the playing player from the start (not this phone, which LMS never lists here).
        settings.state.update { it.copy(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = me) }
        val vm = PlayerViewModel(players, FakeEvents(), settings, local) { now }
        vms += vm
        vm.start()
        return Triple(vm, players, local)
    }

    private val vms = mutableListOf<PlayerViewModel>()

    /**
     * A test with view models stopped at the end even when an assertion fails: their poll
     * loop runs on virtual time, so one left running would keep `runTest` going for ever
     * and hide the failure.
     */
    private fun vmTest(body: suspend TestScope.() -> Unit) = runTest(dispatcher) {
        try {
            body()
        } finally {
            vms.forEach { it.stop() }
        }
    }

    private fun sleeps(p: FakePlayers) = p.commands.filter { it.startsWith("sleep ") }

    @Test
    fun aFixedLengthIsLmsSleepInSeconds() = vmTest {
        val (vm, players) = setup()
        val messages = mutableListOf<PlayerMessage>()
        val c = backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.setSleep(SleepChoice.Minutes(30))
        assertEquals(listOf("sleep 1800 $me"), sleeps(players))
        assertNull(vm.state.value.sleepArm) // nothing to keep in step
        assertTrue(messages.contains(PlayerMessage.SleepSet(1800)))
        c.cancel()
        vm.stop()
    }

    @Test
    fun endOfAlbumAndQueueAreWorkedOutFromTheQueue() = vmTest {
        val (vm, players) = setup()
        vm.setSleep(SleepChoice.End(SleepEnd.ALBUM))
        assertEquals(listOf("sleep 400 $me"), sleeps(players))
        assertEquals(listOf(me to 1), players.queueTimesCalls)
        assertEquals(SleepEnd.ALBUM, vm.state.value.sleepArm?.end)
        players.commands.clear()
        vm.setSleep(SleepChoice.End(SleepEnd.QUEUE))
        assertEquals(listOf("sleep 800 $me"), sleeps(players))
        vm.stop()
    }

    @Test
    fun aStreamWithoutALengthIsRefusedWithAReason() = vmTest {
        val (vm, players) = setup()
        players.statusResult = ApiResult.Ok(sampleStatus().copy(durationSecs = null, current = track(1, 7, null).copy(id = null)))
        vm.refresh()
        val messages = mutableListOf<PlayerMessage>()
        val c = backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.setSleep(SleepChoice.End(SleepEnd.TRACK))
        assertTrue(sleeps(players).isEmpty())
        assertTrue(messages.contains(PlayerMessage.SleepUnknown))
        c.cancel()
        vm.stop()
    }

    @Test
    fun anArmedTimerFollowsASeek() = vmTest {
        val (vm, players) = setup()
        vm.setSleep(SleepChoice.End(SleepEnd.TRACK))
        assertEquals(listOf("sleep 200 $me"), sleeps(players))
        players.commands.clear()
        // LMS reports it, in step: nothing sent.
        players.statusResult = ApiResult.Ok(
            sampleStatus(elapsed = 100.0).copy(durationSecs = 300.0, current = track(1, 7, 300.0), sleepSecs = 199.0),
        )
        vm.refresh()
        assertTrue(sleeps(players).isEmpty())
        // Seeked to near the end: set again.
        players.statusResult = ApiResult.Ok(
            sampleStatus(elapsed = 280.0).copy(durationSecs = 300.0, current = track(1, 7, 300.0), sleepSecs = 198.0),
        )
        vm.refresh()
        assertEquals(listOf("sleep 20 $me"), sleeps(players))
        vm.stop()
    }

    @Test
    fun cancelledElsewhereTheTimerIsForgotten() = vmTest {
        val (vm, players) = setup()
        vm.setSleep(SleepChoice.End(SleepEnd.TRACK))
        players.commands.clear()
        now += 60_000 // LMS has long had time to report it
        vm.refresh() // no sleepSecs: cancelled from another controller
        assertNull(vm.state.value.sleepArm)
        assertTrue(sleeps(players).isEmpty())
        vm.stop()
    }

    @Test
    fun sleepAllAndTheSyncGroup() = vmTest {
        val (vm, players) = setup()
        vm.setSleep(SleepChoice.Minutes(15), SleepScope.ALL)
        assertEquals(listOf("sleep 900 $me", "sleep 900 aa:00:00:00:00:01"), sleeps(players))
        players.commands.clear()
        players.groups = listOf(SyncGroup(listOf("aa:00:00:00:00:01", me, "zz:not-a-player")))
        vm.setSleep(SleepChoice.Minutes(15), SleepScope.GROUP)
        assertEquals(listOf("sleep 900 $me", "sleep 900 aa:00:00:00:00:01"), sleeps(players))
        // One tap turns it off on every player it was set on.
        players.commands.clear()
        vm.setSleep(SleepChoice.End(SleepEnd.TRACK), SleepScope.ALL)
        players.commands.clear()
        vm.cancelSleep()
        assertEquals(listOf("sleep 0 $me", "sleep 0 aa:00:00:00:00:01"), sleeps(players))
        assertNull(vm.state.value.sleepArm)
        vm.stop()
    }

    @Test
    fun thisPhonesTimerFadesItsOwnOutput() = vmTest {
        val local = FakeLocalPlayer()
        local.state.update { it.copy(enabled = true, status = localStatus(me)) }
        val (vm, players, _) = setup(local, FakeSettings(AppSettings(sleepFadeSecs = 60)))
        assertEquals(60, vm.state.value.sleepFade)
        vm.setSleep(SleepChoice.Minutes(30))
        assertTrue(local.fades.contains(1800.0 to 60))
        // The fade then follows what LMS reports for this phone (set from any controller).
        players.statusResult = ApiResult.Ok(sampleStatus().copy(sleepSecs = 600.0))
        vm.refresh()
        assertEquals(600.0 to 60, local.fades.last())
        players.statusResult = ApiResult.Ok(sampleStatus()) // LMS: no timer once turned off
        vm.cancelSleep()
        assertEquals(null to 60, local.fades.last())
        vm.stop()
    }

    @Test
    fun aRefusedTimerIsReported() = vmTest {
        val (vm, players) = setup()
        players.commandResult = ApiResult.Err(AppError.Forbidden("no"))
        val messages = mutableListOf<PlayerMessage>()
        val c = backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.setSleep(SleepChoice.Minutes(30))
        assertTrue(messages.any { it is PlayerMessage.Failed })
        c.cancel()
        vm.stop()
    }

    @Test
    fun theFadeSettingIsCheckedAndKept() = vmTest {
        val settings = FakeSettings()
        val (vm, _) = setup(settings = settings)
        vm.setSleepFade(90)
        assertEquals(90, settings.state.value.sleepFadeSecs)
        assertEquals(90, vm.state.value.sleepFade)
        vm.setSleepFade(5_000)
        assertEquals(120, settings.state.value.sleepFadeSecs)
        vm.stop()
    }

    @Test
    fun upNextIsRemovedWithUndo() = vmTest {
        val (vm, players) = setup()
        val next = track(2, 7, 200.0).copy(queueIndex = 1u)
        players.statusResult = ApiResult.Ok(sampleStatus().copy(queueIndex = 0u, next = next))
        vm.refresh()
        val messages = mutableListOf<PlayerMessage>()
        val c = backgroundScope.launch { vm.messages.collect { messages += it } }
        players.statusResult = ApiResult.Ok(sampleStatus().copy(queueIndex = 0u, next = null))
        vm.removeUpNext()
        assertTrue("remove 1 $me" in players.commands)
        assertNull(vm.state.value.status?.next)
        val removed = messages.filterIsInstance<PlayerMessage.Removed>().single()
        assertTrue(removed.canUndo)
        vm.undoRemove(removed.token)
        assertTrue("restore 2 1 $me" in players.commands)
        c.cancel()
        vm.stop()
    }

    @Test
    fun noUpNextNothingRemoved() = vmTest {
        val (vm, players) = setup()
        players.statusResult = ApiResult.Ok(sampleStatus().copy(next = null))
        vm.refresh()
        vm.removeUpNext()
        assertTrue(players.commands.none { it.startsWith("remove") })
        vm.stop()
    }
}
