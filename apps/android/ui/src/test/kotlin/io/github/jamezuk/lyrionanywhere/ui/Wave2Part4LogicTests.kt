package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import io.github.jamezuk.lyrionanywhere.core.DiscoveryOptions
import io.github.jamezuk.lyrionanywhere.core.DstmProvider
import io.github.jamezuk.lyrionanywhere.core.RadioPlan
import io.github.jamezuk.lyrionanywhere.core.RadioSeed
import io.github.jamezuk.lyrionanywhere.core.StationBatch
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.CarAutoOpen
import io.github.jamezuk.lyrionanywhere.data.CarModeSetting
import io.github.jamezuk.lyrionanywhere.data.EqProfile
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.OutputClass
import io.github.jamezuk.lyrionanywhere.data.OutputEq
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.VisualFrame
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.ui.car.CarAction
import io.github.jamezuk.lyrionanywhere.ui.car.CarAutoOpener
import io.github.jamezuk.lyrionanywhere.ui.car.CarDevice
import io.github.jamezuk.lyrionanywhere.ui.car.CarMessage
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeViewModel
import io.github.jamezuk.lyrionanywhere.ui.car.CarSettingFlow
import io.github.jamezuk.lyrionanywhere.ui.components.HapticSteps
import io.github.jamezuk.lyrionanywhere.ui.components.SwitchedHaptics
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryEvent
import io.github.jamezuk.lyrionanywhere.ui.discovery.startRadioFrom
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.EqMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import kotlinx.coroutines.flow.first
import androidx.compose.ui.graphics.toArgb
import androidx.lifecycle.ViewModel
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val KITCHEN = "aa:00:00:00:00:01"
private const val CAR = "0A:1B:2C:3D:4E:5F"
private const val HEADPHONES = "0A:1B:2C:3D:4E:60"
private val carKit = CarDevice(CAR, "Example Car Kit", carAudio = true)
private val buds = CarDevice(HEADPHONES, "Example Headphones", carAudio = false)
private val anyCar = CarModeSetting(CarAutoOpen.ANY_CAR)
private val thisCar = CarModeSetting(CarAutoOpen.THIS_DEVICE, "Example Car Kit", CAR)

/** Car mode opening and leaving by itself (Wave 2 §5.1). */
class CarAutoOpenerTest {
    @Test
    fun opensForAMatchingCarOnlyWhileInFront() {
        val o = CarAutoOpener()
        assertEquals(CarAction.NONE, o.connected(buds, anyCar, foreground = true))
        assertEquals("not in front: wait", CarAction.NONE, o.connected(carKit, anyCar, foreground = false))
        // The app comes to the front with the car connected.
        assertEquals(CarAction.OPEN, o.check(anyCar, foreground = true))
        assertTrue(o.open)
        assertEquals(CAR, o.openedBy)
        // Already showing: nothing more.
        assertEquals(CarAction.NONE, o.check(anyCar, foreground = true))
        // Another device leaving changes nothing; the car leaving closes it.
        assertEquals(CarAction.NONE, o.disconnected(HEADPHONES))
        assertEquals(CarAction.LEAVE, o.disconnected(CAR.lowercase()))
        assertFalse(o.open)
    }

    @Test
    fun offOrAnotherDeviceNeverOpens() {
        val o = CarAutoOpener()
        assertEquals(CarAction.NONE, o.connected(carKit, CarModeSetting(), foreground = true))
        // "This device" is that one device, car audio or not.
        val budsChosen = CarModeSetting(CarAutoOpen.THIS_DEVICE, "Example Headphones", HEADPHONES)
        assertEquals(CarAction.NONE, o.check(budsChosen, true))
        assertEquals(CarAction.OPEN, o.connected(buds, budsChosen, true))
        // A device with a nonsense address is ignored.
        val o2 = CarAutoOpener()
        assertEquals(CarAction.NONE, o2.connected(CarDevice("not an address", "Car", true), anyCar, true))
        assertEquals(CarAction.NONE, o2.connected(CarDevice("", "Car", true), anyCar, true))
        // A chosen device with a bad address matches nothing.
        assertFalse(o2.matches(CarModeSetting(CarAutoOpen.THIS_DEVICE, "x", ""), CarDevice("", "x", true)))
    }

    @Test
    fun openedByHandItNeverLeavesByItself() {
        val o = CarAutoOpener()
        o.openedByHand()
        o.connected(carKit, anyCar, true)
        assertEquals(CarAction.NONE, o.disconnected(CAR))
        assertTrue(o.open)
    }

    @Test
    fun leftByHandItStaysClosedUntilTheCarReconnects() {
        val o = CarAutoOpener()
        assertEquals(CarAction.OPEN, o.connected(carKit, thisCar, true))
        o.leftByHand(thisCar)
        assertEquals(CarAction.NONE, o.check(thisCar, true))
        assertEquals(CarAction.NONE, o.snapshot(listOf(carKit), thisCar, true))
        // The car goes and comes back: it opens again.
        o.disconnected(CAR)
        assertEquals(CarAction.OPEN, o.connected(carKit, thisCar, true))
    }

    @Test
    fun theSnapshotOnComingToTheFrontOpensAndLeaves() {
        val o = CarAutoOpener()
        assertEquals(CarAction.OPEN, o.snapshot(listOf(buds, carKit), anyCar, foreground = true))
        // The car went while the app was in the background: car mode leaves next time.
        assertEquals(CarAction.LEAVE, o.snapshot(listOf(buds), anyCar, foreground = true))
        assertEquals(CarAction.NONE, o.snapshot(emptyList(), anyCar, foreground = true))
        // Junk in a snapshot is ignored.
        assertEquals(CarAction.NONE, o.snapshot(listOf(CarDevice("zz", "Car", true)), anyCar, true))
    }
}

/** Choosing how car mode opens: the permission only for the automatic options. */
class CarSettingFlowTest {
    @Test
    fun thePermissionIsAskedOnlyWhenNeeded() {
        assertEquals(CarSettingFlow.Step.APPLY, CarSettingFlow.choose(CarAutoOpen.OFF, granted = false))
        assertEquals(CarSettingFlow.Step.ASK, CarSettingFlow.choose(CarAutoOpen.ANY_CAR, granted = false))
        assertEquals(CarSettingFlow.Step.ASK, CarSettingFlow.choose(CarAutoOpen.THIS_DEVICE, granted = false))
        assertEquals(CarSettingFlow.Step.APPLY, CarSettingFlow.choose(CarAutoOpen.ANY_CAR, granted = true))
        assertEquals(CarSettingFlow.Step.PICK, CarSettingFlow.choose(CarAutoOpen.THIS_DEVICE, granted = true))
    }

    @Test
    fun refusedGoesBackToOff() {
        assertEquals(CarSettingFlow.Step.REFUSED, CarSettingFlow.answered(CarAutoOpen.ANY_CAR, granted = false))
        assertEquals(CarSettingFlow.Step.REFUSED, CarSettingFlow.answered(CarAutoOpen.THIS_DEVICE, granted = false))
        assertEquals(CarSettingFlow.Step.APPLY, CarSettingFlow.answered(CarAutoOpen.ANY_CAR, granted = true))
        assertEquals(CarSettingFlow.Step.PICK, CarSettingFlow.answered(CarAutoOpen.THIS_DEVICE, granted = true))
        // Revoked later: back to Off.
        assertEquals(CarModeSetting(), CarSettingFlow.withoutPermission(thisCar, granted = false))
        assertEquals(thisCar, CarSettingFlow.withoutPermission(thisCar, granted = true))
    }

    @Test
    fun aPickedDeviceIsCheckedBeforeItIsKept() {
        assertEquals(thisCar, CarSettingFlow.picked(carKit))
        assertEquals(CarModeSetting(), CarSettingFlow.picked(CarDevice("bad", "Car", true)))
        val noisy = CarSettingFlow.picked(CarDevice(CAR, "Car\u0000\n" + "x".repeat(300), true))
        assertTrue(noisy.deviceName.none { it.isISOControl() } && noisy.deviceName.length <= 64)
    }
}

/** Haptics (Wave 2 §5.4): when a tick is due, and the one switch. */
class HapticsTest {
    @Test
    fun ticksAtEachStep() {
        assertTrue(HapticSteps.crossed(0.05f, 0.11f, HapticSteps.SCRUB))
        assertTrue("backwards too", HapticSteps.crossed(0.31f, 0.29f, HapticSteps.SCRUB))
        assertFalse(HapticSteps.crossed(0.11f, 0.19f, HapticSteps.SCRUB))
        assertTrue(HapticSteps.crossed(44f, 45f, HapticSteps.VOLUME))
        assertFalse(HapticSteps.crossed(45f, 49.9f, HapticSteps.VOLUME))
        for ((a, b) in listOf(Float.NaN to 1f, 0f to Float.POSITIVE_INFINITY)) assertFalse(HapticSteps.crossed(a, b, 0.1f))
        assertFalse(HapticSteps.crossed(0f, 1f, 0f))
        assertFalse(HapticSteps.crossed(0f, 1f, -1f))
    }

    @Test
    fun theSwitchStopsEveryTick() {
        val felt = mutableListOf<HapticFeedbackType>()
        val inner = object : HapticFeedback {
            override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
                felt += hapticFeedbackType
            }
        }
        var on = true
        val h = SwitchedHaptics({ on }, inner)
        h.performHapticFeedback(HapticSteps.tick)
        on = false
        h.performHapticFeedback(HapticSteps.confirm)
        h.performHapticFeedback(HapticSteps.pickUp)
        assertEquals(listOf(HapticSteps.tick), felt)
    }
}

private val plan = RadioPlan(null, null, DstmProvider("PLUGIN_DSTM_SIMILAR", "Similar tracks"), true)

/** "Start radio" from the playing track (car mode and the app shortcut). */
class QuickRadioTest {
    @Test
    fun startsWhatTheServerOffersOrSaysWhy() = runTest {
        val d = FakeDiscovery()
        assertEquals(DiscoveryEvent.NoPlayer, startRadioFrom(d, null, RadioSeed.Track(7)))
        assertEquals(DiscoveryEvent.NoRadio, startRadioFrom(d, KITCHEN, null))
        assertEquals("nothing offered", DiscoveryEvent.NoRadio, startRadioFrom(d, KITCHEN, RadioSeed.Track(7)))
        assertTrue(d.started.isEmpty())
        d.answer = ApiResult.Err(AppError.Forbidden("no"))
        assertTrue(startRadioFrom(d, KITCHEN, RadioSeed.Track(7)) is DiscoveryEvent.Failed)
        d.answer = ApiResult.Ok(DiscoveryOptions(plan, null, null))
        assertEquals(DiscoveryEvent.RadioStarted(endless = true), startRadioFrom(d, KITCHEN, RadioSeed.Track(7)))
        assertEquals(KITCHEN to (RadioSeed.Track(7) as RadioSeed), d.started.single().let { it.first to it.second })
        d.startAnswer = ApiResult.Err(AppError.Unexpected("x"))
        assertTrue(startRadioFrom(d, KITCHEN, RadioSeed.Track(7)) is DiscoveryEvent.Failed)
    }
}

/** Car mode's lists and actions. */
class CarModeViewModelTest : MainDispatcherTest() {
    @Test
    fun oneLevelOfFavouritesAndWorkingStations() = runTest(dispatcher) {
        val me = FakeMe()
        val store = ViewModelStore()
        val vm = inStore(store) { CarModeViewModel(FakeLibrary(), me, FakeDiscovery()) { KITCHEN } }
        try {
            assertTrue(vm.state.value.loading)
            vm.load()
            val s = vm.state.value
            assertFalse(s.loading)
            assertEquals(listOf("Radio"), s.favourites.map { it.name })
            assertEquals("the broken station is left out", listOf(1L, 2L), s.stations.map { it.id })
            vm.startStation(s.stations[0])
            assertEquals(listOf(1L to KITCHEN), me.started)
            assertEquals(CarMessage.StationStarted("Forgotten favourites"), vm.messages.first())
            me.startResult = ApiResult.Ok(StationBatch.Broken("gone"))
            vm.startStation(s.stations[1])
            assertTrue(vm.messages.first() is CarMessage.Failed)
        } finally {
            store.clear()
        }
    }

    @Test
    fun withNoPlayerNothingIsSent() = runTest(dispatcher) {
        val me = FakeMe()
        val d = FakeDiscovery().apply { answer = ApiResult.Ok(DiscoveryOptions(plan, null, null)) }
        val store = ViewModelStore()
        val vm = inStore(store) { CarModeViewModel(FakeLibrary(), me, d) { null } }
        try {
            vm.load()
            vm.startStation(vm.state.value.stations[0])
            assertEquals(CarMessage.NoPlayer, vm.messages.first())
            vm.startRadio(7)
            assertEquals(CarMessage.Radio(DiscoveryEvent.NoPlayer), vm.messages.first())
            assertTrue(me.started.isEmpty() && d.started.isEmpty())
        } finally {
            store.clear()
        }
    }

    @Test
    fun radioFromThePlayingTrack() = runTest(dispatcher) {
        val d = FakeDiscovery().apply { answer = ApiResult.Ok(DiscoveryOptions(plan, null, null)) }
        val store = ViewModelStore()
        val vm = inStore(store) { CarModeViewModel(FakeLibrary(), FakeMe(), d) { KITCHEN } }
        try {
            vm.startRadio(7)
            assertEquals(CarMessage.Radio(DiscoveryEvent.RadioStarted(true)), vm.messages.first())
            // A stream (no id) can't seed radio.
            vm.startRadio(null)
            assertEquals(CarMessage.Radio(DiscoveryEvent.NoRadio), vm.messages.first())
            vm.startRadio(-3)
            assertEquals(CarMessage.Radio(DiscoveryEvent.NoRadio), vm.messages.first())
            assertEquals(1, d.started.size)
        } finally {
            store.clear()
        }
    }
}

/** Per-output EQ in the Sound settings (Wave 2 §5.3). */
class OutputEqViewModelTest : MainDispatcherTest() {
    private val profile = "Preamp: -3 dB\nFilter 1: ON PK Fc 1000 Hz Gain -3 dB Q 1"

    @Test
    fun bindingImportingAndRemovingPerOutput() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val store = ViewModelStore()
        val vm = inStore(store) { SoundViewModel(local, sdk = 34) }
        try {
            vm.importEq("Main.txt", profile)
            vm.bindOutput(OutputClass.WIRED, "", null)
            vm.importEqFor(OutputClass.BLUETOOTH, "Example Headphones", "Headphones.txt", profile)
            assertEquals(EqMessage.Imported("Headphones.txt"), vm.state.value.message)
            val s = local.state.value.sound
            assertEquals(2, s.outputEq.size)
            assertEquals("Headphones.txt", s.bindingFor(AudioOutput(OutputClass.BLUETOOTH, "Example Headphones"))!!.profile!!.name)
            assertNull(s.bindingFor(AudioOutput(OutputClass.WIRED))!!.profile)
            // A bad profile for an output binds nothing.
            vm.importEqFor(OutputClass.USB, "Example DAC", "bad.txt", "not a profile")
            assertTrue(vm.state.value.message is EqMessage.Invalid)
            assertEquals(2, local.state.value.sound.outputEq.size)
            // The curve shows what's in use on the current output.
            local.state.value = local.state.value.copy(output = AudioOutput(OutputClass.WIRED))
            assertNull("no EQ on wired: the main one shows", vm.state.value.effective.eq)
            assertEquals("Main.txt", vm.state.value.shownProfile?.name)
            vm.unbindOutput(OutputClass.WIRED, "")
            vm.unbindOutput(OutputClass.BLUETOOTH, "Example Headphones")
            assertTrue(local.state.value.sound.outputEq.isEmpty())
        } finally {
            store.clear()
        }
    }

    @Test
    fun theComparisonNeverOutlivesTheScreen() = runTest(dispatcher) {
        val bypass = mutableListOf<Boolean>()
        val local = object : io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository by FakeLocalPlayer() {
            override fun setEqBypass(on: Boolean) {
                bypass += on
            }
        }
        val store = ViewModelStore()
        val vm = inStore(store) { SoundViewModel(local, sdk = 34) }
        vm.setBypass(true)
        store.clear()
        assertEquals(listOf(true, false), bypass)
    }

    @Test
    fun theSwitchNeedsAProfileSomewhere() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val store = ViewModelStore()
        val vm = inStore(store) { SoundViewModel(local, sdk = 34) }
        try {
            vm.setEqEnabled(true)
            assertFalse(local.state.value.sound.eqEnabled)
            local.state.value = local.state.value.copy(
                sound = SoundSettings(outputEq = listOf(OutputEq(OutputClass.WIRED, "", EqProfile("W", profile, 1, -3.0)))),
            )
            vm.setEqEnabled(true)
            assertTrue(local.state.value.sound.eqEnabled)
        } finally {
            store.clear()
        }
    }
}

/** The offline Now playing's visualiser (Wave 2 §5.7). */
class OfflineVisualiserViewModelTest : MainDispatcherTest() {
    @Test
    fun theVisualiserIsTheEnginesAndItsChoicesAreRemembered() = runTest(dispatcher) {
        val settings = FakeSettings(AppSettings(nowPlaying = NowPlayingPrefs(visualiser = true, visualStyle = VisualStyle.VU)))
        val player = object : io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository by FakeOfflinePlayer(OfflinePlayerState()) {
            val switched = mutableListOf<Boolean>()

            override fun setVisualiser(on: Boolean) {
                switched += on
            }

            override fun visualFrame(): VisualFrame = VisualFrame.SILENT
        }
        val store = ViewModelStore()
        val vm = inStore(store) { OfflinePlayerViewModel(player, { 0L }, settings) }
        try {
            assertEquals(NowPlayingPrefs(visualiser = true, visualStyle = VisualStyle.VU), vm.state.value.nowPlaying)
            vm.setActive(true)
            vm.setActive(false)
            assertEquals(listOf(true, false), player.switched)
            assertEquals(VisualFrame.SILENT, vm.frame())
            vm.setVisualStyle(VisualStyle.SCOPE)
            vm.setVisualiser(false)
            assertEquals(NowPlayingPrefs(visualiser = false, visualStyle = VisualStyle.SCOPE), settings.state.value.nowPlaying)
            // Without a settings store the choice still shows, unremembered.
            val bare = inStore(store) { OfflinePlayerViewModel(player, { 0L }) }
            bare.setVisualiser(true)
            assertTrue(bare.state.value.nowPlaying.visualiser)
        } finally {
            store.clear()
        }
    }
}

/** A view model in [store], as a screen would hold it: [ViewModelStore.clear] ends it (onCleared). */
internal inline fun <reified T : ViewModel> inStore(store: ViewModelStore, noinline make: () -> T): T {
    val factory = object : ViewModelProvider.Factory {
        @Suppress("UNCHECKED_CAST")
        override fun <V : ViewModel> create(modelClass: Class<V>): V = make() as V
    }
    return ViewModelProvider(store, factory)["vm-" + System.identityHashCode(make), T::class.java]
}

/** Car mode is high contrast: every pair it uses is over 7:1 (WCAG AAA). */
class CarColoursTest {
    @Test
    fun everyPairIsOverSevenToOne() {
        val s = io.github.jamezuk.lyrionanywhere.ui.car.CarColours
        val pairs = listOf(
            "text" to (s.onBackground to s.background),
            "secondary text" to (s.onSurfaceVariant to s.background),
            "play button" to (s.onPrimary to s.primary),
            "buttons" to (s.onSecondaryContainer to s.secondaryContainer),
            "art placeholder" to (s.onSurfaceVariant to s.surfaceVariant),
        )
        for ((name, p) in pairs) {
            val c = io.github.jamezuk.lyrionanywhere.ui.theme.contrast(p.first.toArgb(), p.second.toArgb())
            assertTrue("$name: $c", c >= 7.0)
        }
    }
}
