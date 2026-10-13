package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.ServerEvent
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import io.github.jamezuk.lyrionanywhere.data.VisualFrame
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingLayouts
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.RoomAlarm
import io.github.jamezuk.lyrionanywhere.ui.player.RoomDrop
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.VisualBlock
import io.github.jamezuk.lyrionanywhere.ui.player.VisualMotion
import io.github.jamezuk.lyrionanywhere.ui.player.VisualState
import io.github.jamezuk.lyrionanywhere.ui.player.VuCalibration
import io.github.jamezuk.lyrionanywhere.ui.player.VuMeter
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The visualiser's ballistics and smoothing, layout order and drop targets (Wave 2 §4). */
class VisualiserLogicTest {
    @Test
    fun theVuNeedleRisesAndFallsInAboutThreeHundredMilliseconds() {
        val m = VuMeter()
        // Full scale, in 10 ms steps: most of the way by 150 ms, 99 % by 300 ms.
        repeat(15) { m.step(0f, 10) }
        assertTrue("${m.needle}", m.needle in 0.8f..0.95f)
        repeat(15) { m.step(0f, 10) }
        assertTrue("${m.needle}", m.needle >= 0.985f)
        // Silence: back down by 300 ms; the peak mark holds meanwhile.
        repeat(30) { m.step(VisualFrame.FLOOR_DB, 10) }
        assertTrue("${m.needle}", m.needle <= 0.015f)
        assertTrue("peak held: ${m.peak}", m.peak >= 0.98f)
        // After the hold (1.5 s), the peak falls too.
        repeat(200) { m.step(VisualFrame.FLOOR_DB, 10) }
        assertTrue("${m.peak}", m.peak < 0.1f)
    }

    @Test
    fun theVuScaleAndJunkLevels() {
        // −20 VU to +3 VU, 0 VU at the reference (−18 dBFS until calibrated).
        assertEquals(0f, VuMeter.position(-90f))
        assertEquals(0f, VuMeter.position(-38f))
        assertEquals(VuMeter.ZERO_VU, VuMeter.position(-18f), 1e-4f)
        assertEquals(1f, VuMeter.position(-15f))
        assertEquals(1f, VuMeter.position(12f))
        assertEquals(0f, VuMeter.position(Float.NaN))
        assertEquals(VuMeter.ZERO_VU, VuMeter.position(-6f, -6f), 1e-4f)
        assertEquals(VuMeter.ZERO_VU, VuMeter.position(-18f, Float.NaN), 1e-4f)
        val m = VuMeter()
        m.step(Float.NaN, 100)
        m.step(0f, Long.MAX_VALUE)
        m.step(0f, -5)
        assertTrue(m.needle in 0f..1f && m.peak in 0f..1f)
    }

    @Test
    fun theVuCalibrationFollowsTheMasteringLevel() {
        // Regression: a fixed scale left the needles at the end stop on modern masters.
        fun settle(c: VuCalibration, db: Float, ms: Long): Float {
            var t = 0L
            while (t < ms) { c.step(floatArrayOf(db, db), 33); t += 33 }
            return c.referenceDb
        }
        val loud = VuCalibration()
        val ref = settle(loud, -9f, 15_000)
        assertEquals(-6f, ref, 0.5f)
        val pos = VuMeter.position(-9f, ref)
        assertTrue("about −3 VU: $pos", pos in 0.65f..0.8f)
        assertTrue("loud passages reach the red", VuMeter.position(-4f, ref) > VuMeter.ZERO_VU)
        val quiet = VuCalibration()
        val qref = settle(quiet, -24f, 15_000)
        assertEquals(pos, VuMeter.position(-24f, qref), 0.05f)
        // Silence, garbage and the limits.
        settle(loud, -90f, 20_000)
        assertEquals(ref, loud.referenceDb, 0.1f)
        loud.step(floatArrayOf(Float.NaN, Float.POSITIVE_INFINITY), 33)
        assertTrue(loud.referenceDb.isFinite())
        assertEquals(VuCalibration.MAX_REFERENCE_DB, settle(VuCalibration(), -1f, 20_000))
        assertEquals(VuCalibration.MIN_REFERENCE_DB, settle(VuCalibration(), -50f, 20_000))
        // Quick to settle, then slow: a one-second quiet bar barely moves it.
        val c = VuCalibration()
        settle(c, -9f, 3_000)
        assertEquals(-6f, c.referenceDb, 0.5f)
        settle(c, -20f, 1_000)
        assertTrue("${c.referenceDb}", c.referenceDb > -8f)
        c.reset()
        assertEquals(VuMeter.DEFAULT_REFERENCE_DB, c.referenceDb)
        // Through VisualState: the meters stop pinning.
        val st = VisualState()
        repeat(450) { st.update(frame(0.5f).let { f -> VisualFrame(f.bands, f.scope, floatArrayOf(-9f, -9f), floatArrayOf(-3f, -3f), 0f) }, 33) }
        assertTrue("needle ${st.vu[0].needle}", st.vu[0].needle < 0.9f)
    }

    private fun frame(level: Float, bass: Float = 0f) = VisualFrame(
        FloatArray(VisualFrame.BANDS) { level },
        FloatArray(VisualFrame.SCOPE) { level },
        floatArrayOf(-6f, -6f),
        floatArrayOf(-1f, -1f),
        bass,
    )

    @Test
    fun barsJumpUpFallBackGentlyAndSettle() {
        val s = VisualState()
        assertTrue(s.update(frame(0.8f, bass = 1f), 33))
        assertEquals(0.8f, s.bars[0])
        assertTrue(s.glow > 0.5f)
        // No new frame: the bars fall, at most 1.8 of their height a second.
        s.update(null, 100)
        assertEquals(0.62f, s.bars[0], 0.001f)
        // Eventually everything rests and nothing more is drawn.
        repeat(100) { s.update(null, 33) }
        assertEquals(0f, s.bars[0])
        assertFalse("settled: no more redraws", s.update(null, 33))
        val tick = s.tick
        s.update(null, 33)
        assertEquals(tick, s.tick)
    }

    @Test
    fun theVisualiserStaysOffInBatterySaverAndWithoutAnimations() {
        assertEquals(VisualBlock.NONE, VisualMotion.block(false, 1f))
        assertEquals(VisualBlock.NONE, VisualMotion.block(false, 0.5f))
        assertEquals(VisualBlock.BATTERY_SAVER, VisualMotion.block(true, 1f))
        assertEquals(VisualBlock.ANIMATIONS_OFF, VisualMotion.block(false, 0f))
    }

    @Test
    fun stylesCycleBothWays() {
        assertEquals(VisualStyle.SCOPE, VisualMotion.next(VisualStyle.BARS))
        assertEquals(VisualStyle.BARS, VisualMotion.next(VisualStyle.VU))
        assertEquals(VisualStyle.VU, VisualMotion.next(VisualStyle.BARS, forward = false))
        // At most 30 frames a second.
        assertTrue(VisualMotion.FRAME_MS >= 33)
    }

    @Test
    fun layoutsStopAtEitherEndAndWideOffersLyricsAndInfo() {
        assertEquals(NowPlayingLayout.LYRICS, NowPlayingLayouts.next(NowPlayingLayout.ART, forward = true))
        assertEquals(NowPlayingLayout.INFO, NowPlayingLayouts.next(NowPlayingLayout.LYRICS, forward = true))
        assertEquals(NowPlayingLayout.INFO, NowPlayingLayouts.next(NowPlayingLayout.INFO, forward = true))
        assertEquals(NowPlayingLayout.ART, NowPlayingLayouts.next(NowPlayingLayout.ART, forward = false))
        assertEquals(NowPlayingLayout.INFO, NowPlayingLayouts.widePane(NowPlayingLayout.ART))
        assertEquals(NowPlayingLayout.LYRICS, NowPlayingLayouts.widePane(NowPlayingLayout.LYRICS))
        assertEquals(
            NowPlayingLayout.LYRICS,
            NowPlayingLayouts.next(NowPlayingLayout.INFO, forward = false, NowPlayingLayouts.WIDE),
        )
    }

    @Test
    fun aDropLandsOnTheCardUnderTheFingerButNeverItself() {
        val cards = mapOf("a" to Rect(0f, 0f, 100f, 100f), "b" to Rect(0f, 110f, 100f, 210f))
        assertEquals("b", RoomDrop.target(Offset(50f, 150f), cards, "a"))
        assertNull(RoomDrop.target(Offset(50f, 50f), cards, "a"))
        assertNull(RoomDrop.target(Offset(50f, 105f), cards, "a"))
        assertNull(RoomDrop.target(Offset(Float.NaN, 50f), cards, "b"))
    }
}

/** The Rooms view model (Wave 2 §4.3): reads, polling only while shown, controls, sync and alarms. */
class RoomsViewModelTest : MainDispatcherTest() {
    private val kitchen = "aa:00:00:00:00:01"
    private val lounge = "aa:00:00:00:00:02"

    private val vms = mutableListOf<RoomsViewModel>()

    private fun roomsTest(body: suspend TestScope.() -> Unit) = runTest(dispatcher) {
        try {
            body()
        } finally {
            vms.forEach { it.stop() }
        }
    }

    private fun setup(
        players: FakePlayers = FakePlayers(),
        events: FakeEvents = FakeEvents(),
        alarms: FakeAlarms? = FakeAlarms(),
    ): RoomsViewModel {
        players.roomStatus[kitchen] = sampleStatus().copy(name = "Kitchen")
        players.roomStatus[lounge] = sampleStatus(PlaybackMode.PAUSED).copy(name = "Lounge", volume = 20u)
        // Monday 06:00: the sample alarm (07:00 on weekdays) rings in an hour.
        return RoomsViewModel(players, events, alarms) { 1 to 6 * 3600 }.also { vms += it }
    }

    @Test
    fun startReadsEveryRoomTheGroupsAndTheAlarms() = roomsTest {
        val players = FakePlayers().apply { groups = listOf(SyncGroup(listOf(kitchen, lounge))) }
        val alarms = FakeAlarms()
        val vm = setup(players, alarms = alarms)
        vm.start()
        runCurrent()
        val s = vm.state.value
        assertEquals(listOf(kitchen, lounge), s.rooms.map { it.id })
        assertEquals("Kitchen", s.room(kitchen)?.name)
        assertEquals(20u, s.room(lounge)?.status?.volume)
        assertEquals(listOf("Lounge"), s.groupOf(kitchen).map { it.name })
        assertEquals(RoomAlarm(1, 7 * 3600), s.room(kitchen)?.alarm)
        assertEquals(listOf("list $kitchen", "list $lounge"), alarms.calls)
        assertEquals(listOf(listOf(kitchen, lounge)), players.statusesCalls)
    }

    @Test
    fun statusesArePolledEveryFiveSecondsOnlyWhileShown() = roomsTest {
        val players = FakePlayers()
        val vm = setup(players)
        vm.start()
        runCurrent()
        assertEquals(1, players.statusesCalls.size)
        advanceTimeBy(RoomsViewModel.POLL_MS + 1)
        assertEquals(2, players.statusesCalls.size)
        advanceTimeBy(RoomsViewModel.POLL_MS)
        assertEquals(3, players.statusesCalls.size)
        // The screen stops: nothing more is read.
        vm.stop()
        assertFalse(vm.polling)
        advanceTimeBy(RoomsViewModel.POLL_MS * 10)
        assertEquals(3, players.statusesCalls.size)
    }

    @Test
    fun aPlayerChangeEventRefreshesAtOnce() = roomsTest {
        val players = FakePlayers()
        val events = FakeEvents()
        val vm = setup(players, events)
        vm.start()
        runCurrent()
        val before = players.statusesCalls.size
        events.flow.tryEmit(StreamUpdate.Event(ServerEvent.PlayerChanged(lounge, "mixer", listOf("volume"))))
        advanceTimeBy(RoomsViewModel.EVENT_COALESCE_MS + 1)
        assertEquals(before + 1, players.statusesCalls.size)
        // A player not shown here: nothing.
        events.flow.tryEmit(StreamUpdate.Event(ServerEvent.PlayerChanged("aa:00:00:00:00:99", "play", emptyList())))
        advanceTimeBy(RoomsViewModel.EVENT_COALESCE_MS + 1)
        assertEquals(before + 1, players.statusesCalls.size)
    }

    @Test
    fun atMostTwentyRooms() = roomsTest {
        val players = FakePlayers()
        players.players = (1..25).map { Player("aa:00:00:00:01:%02x".format(it), "Room $it", null, true, true, false) }
        val vm = setup(players)
        vm.start()
        runCurrent()
        assertEquals(20, vm.state.value.rooms.size)
        assertTrue(vm.state.value.more)
        assertEquals(20, players.statusesCalls.single().size)
    }

    @Test
    fun controlsShowAtOnceAndReachTheRightPlayer() = roomsTest {
        val players = FakePlayers()
        val vm = setup(players)
        vm.start()
        runCurrent()
        vm.setVolume(lounge, 150)
        vm.setPower(kitchen, false)
        vm.togglePlay(lounge)
        vm.togglePlay(kitchen)
        assertEquals(
            listOf("volume 100 $lounge", "power false $kitchen", "play $lounge", "pause true $kitchen"),
            players.commands,
        )
    }

    @Test
    fun aFailedControlIsReported() = roomsTest {
        val players = FakePlayers()
        val vm = setup(players)
        vm.start()
        runCurrent()
        val messages = mutableListOf<AppError>()
        val c = backgroundScope.launch(kotlinx.coroutines.Dispatchers.Unconfined) { vm.messages.collect { messages += it } }
        players.commandResult = ApiResult.Err(AppError.Offline())
        vm.setVolume(kitchen, 10)
        runCurrent()
        assertEquals(1, messages.size)
        c.cancel()
    }

    @Test
    fun syncJoinsTheDraggedRoomToTheOtherAndUnsyncTakesItOut() = roomsTest {
        val players = FakePlayers()
        val vm = setup(players)
        vm.start()
        runCurrent()
        vm.sync(lounge, kitchen)
        assertEquals("sync $kitchen $lounge true", players.commands.last())
        // A room onto itself, or an unknown room: nothing sent.
        vm.sync(kitchen, kitchen)
        vm.sync("aa:00:00:00:00:99", kitchen)
        assertEquals(1, players.commands.size)
        vm.unsync(lounge)
        assertEquals("sync $lounge $lounge false", players.commands.last())
    }

    @Test
    fun aFailedSyncPutsTheGroupsBack() = roomsTest {
        val players = FakePlayers()
        val vm = setup(players)
        vm.start()
        runCurrent()
        players.commandResult = ApiResult.Err(AppError.Offline())
        vm.sync(lounge, kitchen)
        runCurrent()
        assertTrue(vm.state.value.groups.isEmpty())
    }

    @Test
    fun alarmsAreReadAgainWhenThisAppChangesThem() = roomsTest {
        val alarms = FakeAlarms()
        val vm = setup(alarms = alarms)
        vm.start()
        runCurrent()
        alarms.calls.clear()
        alarms.current = alarms.current.copy(enabled = false)
        alarms.changeFlow.tryEmit(kitchen)
        runCurrent()
        assertEquals(listOf("list $kitchen"), alarms.calls)
        assertNull(vm.state.value.room(kitchen)?.alarm)
        // Not a room here: ignored.
        alarms.changeFlow.tryEmit("aa:00:00:00:00:99")
        runCurrent()
        assertEquals(1, alarms.calls.size)
    }

    @Test
    fun playersThatCantBeListedShowAnError() = roomsTest {
        val players = object : io.github.jamezuk.lyrionanywhere.data.PlayersRepository by FakePlayers() {
            override suspend fun list() = ApiResult.Err(AppError.Offline())
        }
        val vm = RoomsViewModel(players).also { vms += it }
        vm.start()
        runCurrent()
        assertTrue(vm.state.value.error != null)
        assertFalse(vm.state.value.loading)
    }
}

/** The player view model's part (Wave 2 §4): this device's choices, the visualiser's source, credits. */
class NowPlayingChoicesViewModelTest : MainDispatcherTest() {
    @Test
    fun choicesAreRememberedTheVisualiserReachesThePhoneAndCreditsAreReadOnce() = runTest(dispatcher) {
        val settings = FakeSettings()
        settings.state.update { it.copy(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = "aa:00:00:00:00:02") }
        val local = FakeLocalPlayer()
        val library = FakeLibrary().apply { credits = listOf(sampleCredit) }
        val vm = PlayerViewModel(FakePlayers(), FakeEvents(), settings, local, library = library) { 0L }
        try {
            vm.start()
            vm.setLayout(NowPlayingLayout.INFO)
            vm.setVisualiser(true)
            vm.setVisualStyle(VisualStyle.VU)
            vm.setQueuePanel(false)
            assertEquals(NowPlayingPrefs(NowPlayingLayout.INFO, true, VisualStyle.VU, false), settings.state.value.nowPlaying)
            assertEquals(settings.state.value.nowPlaying, vm.state.value.nowPlaying)
            // The visualiser on screen switches the phone's analysis, and frames come from it.
            vm.setActive(true)
            vm.setActive(false)
            assertEquals(listOf(true, false), local.visual)
            assertNull(vm.frame())
            local.frame = VisualFrame.SILENT
            assertEquals(VisualFrame.SILENT, vm.frame())
            // Credits for the current track, once.
            vm.loadCredits()
            vm.loadCredits()
            assertEquals(listOf(1L), library.creditCalls)
            assertEquals(1L, vm.state.value.creditsTrackId)
            assertEquals("Example Composer", vm.state.value.credits?.single()?.name)
        } finally {
            vm.close()
        }
    }
}

private val sampleCredit = Credit(ArtistRole.COMPOSER, "Example Composer", 41)
