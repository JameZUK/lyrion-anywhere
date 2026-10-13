package io.github.jamezuk.lyrionanywhere.ui

import android.view.View
import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationSpec
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.CarAutoOpen
import io.github.jamezuk.lyrionanywhere.data.CarModeSetting
import io.github.jamezuk.lyrionanywhere.data.EqProfile
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.OutputClass
import io.github.jamezuk.lyrionanywhere.data.OutputEq
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.ui.car.CAR_TARGET
import io.github.jamezuk.lyrionanywhere.ui.car.CarBluetoothAccess
import io.github.jamezuk.lyrionanywhere.ui.car.CarDevice
import io.github.jamezuk.lyrionanywhere.ui.car.CarListsUi
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeActions
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeScreen
import io.github.jamezuk.lyrionanywhere.ui.car.CarModeSettingView
import io.github.jamezuk.lyrionanywhere.ui.components.HapticsSetting
import io.github.jamezuk.lyrionanywhere.ui.components.ProvideHaptics
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerUi
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.RatingControl
import io.github.jamezuk.lyrionanywhere.ui.player.SeekBar
import io.github.jamezuk.lyrionanywhere.ui.player.SleepChoices
import io.github.jamezuk.lyrionanywhere.ui.settings.DynamicColourSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.EqCompareButton
import io.github.jamezuk.lyrionanywhere.ui.settings.EqCurveView
import io.github.jamezuk.lyrionanywhere.ui.settings.OutputEqSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

internal const val EQ_TEXT = "Preamp: -3 dB\nFilter 1: ON PK Fc 1000 Hz Gain -4 dB Q 1.0\n"
internal val mainEq = EqProfile("Example profile", EQ_TEXT, 1, -3.0)

internal val carLists = CarListsUi(
    favourites = listOf(Favourite("f0.0", "Example Radio", null, false), Favourite("f0.1", "Example Station Two", null, false)),
    stations = listOf(Station(1, "Forgotten favourites", StationSpec.Forgotten, null)),
    loading = false,
)

internal fun carActions(calls: MutableList<String> = mutableListOf()) = CarModeActions(
    onExit = { calls += "exit" },
    onTogglePlay = { calls += "toggle" },
    onStop = { calls += "stop" },
    onNext = { calls += "next" },
    onPrevious = { calls += "previous" },
    onStartRadio = { calls += "radio" },
    onVoice = { calls += "voice" },
    onFavourite = { calls += "favourite ${it.id}" },
    onStation = { calls += "station ${it.id}" },
)

/** Records every tick, for the haptics tests. */
internal class RecordingHaptics : HapticFeedback {
    val felt = mutableListOf<HapticFeedbackType>()

    override fun performHapticFeedback(hapticFeedbackType: HapticFeedbackType) {
        felt += hapticFeedbackType
    }
}

/** The settings' Bluetooth side, faked: whether permitted, what Android answers, the paired devices. */
internal class FakeCarAccess(
    var permitted: Boolean = false,
    var answer: Boolean = true,
    private val devices: List<CarDevice> = listOf(
        CarDevice("0A:1B:2C:3D:4E:5F", "Example Car Kit", true),
        CarDevice("0A:1B:2C:3D:4E:60", "Example Headphones", false),
    ),
) : CarBluetoothAccess {
    var asked = 0

    override fun granted() = permitted

    override fun request(onResult: (Boolean) -> Unit) {
        asked++
        if (answer) permitted = true
        onResult(answer)
    }

    override fun bonded() = devices
}

/** Car mode (Wave 2 §5.1): the driving screen. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h900dp")
class CarModeScreenTests {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    private fun show(state: PlayerUiState = auditPlayerState(), lists: CarListsUi = carLists, actions: CarModeActions = carActions()) =
        compose.setContent { LyrionTheme(dynamicColour = false) { CarModeScreen(state, lists, clock = { 0L }, actions = actions) } }

    @Test
    fun theTransportAndALongPressToStop() {
        val calls = mutableListOf<String>()
        show(actions = carActions(calls))
        compose.onNodeWithTag("car-previous").performClick()
        compose.onNodeWithTag("car-play").performClick()
        compose.onNodeWithTag("car-next").performClick()
        compose.onNodeWithTag("car-play").performTouchInput { longClick() }
        // TalkBack: Stop is an action on the play/pause button.
        val stop = compose.onNodeWithTag("car-play").fetchSemanticsNode().config[SemanticsActions.CustomActions].single()
        assertEquals("Stop", stop.label)
        compose.runOnUiThread { stop.action() }
        compose.onNodeWithTag("car-radio").performClick()
        compose.onNodeWithTag("car-voice").performClick()
        assertEquals(listOf("previous", "toggle", "next", "stop", "stop", "radio", "voice"), calls)
        // The time is large and spoken in full; no visualiser here.
        compose.onNodeWithTag("car-elapsed").assertExists()
        compose.onNodeWithContentDescription("0:10 of 3:20").assertExists()
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
    }

    @Test
    fun everyTargetIsAtLeast76dp() {
        show()
        fun check() {
            val nodes = compose.onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick)).fetchSemanticsNodes()
            assertTrue(nodes.isNotEmpty())
            for (n in nodes) {
                val d = n.layoutInfo.density.density
                val w = n.touchBoundsInRoot.width / d
                val h = n.touchBoundsInRoot.height / d
                val what = n.config.getOrNull(SemanticsProperties.TestTag) ?: n.config.getOrNull(SemanticsProperties.ContentDescription)
                assertTrue("$what is $w x $h dp", w >= CAR_TARGET.value - 0.5f && h >= CAR_TARGET.value - 0.5f)
            }
        }
        check()
        compose.onNodeWithTag("car-favourites").performClick()
        check()
    }

    @Test
    fun favouritesAndStationsAreOneLevel() {
        val calls = mutableListOf<String>()
        show(actions = carActions(calls))
        compose.onNodeWithTag("car-favourites").performClick()
        compose.onNodeWithText("Example Radio").assertExists()
        compose.onNodeWithTag("car-favourite-f0.1").performClick()
        // Chosen: back to what's playing.
        compose.onNodeWithTag("car-play").assertExists()
        compose.onNodeWithTag("car-stations").performClick()
        compose.onNodeWithTag("car-station-1").performClick()
        compose.onNodeWithTag("car-stations").performClick()
        compose.onNodeWithTag("car-back").performClick()
        compose.onNodeWithTag("car-play").assertExists()
        assertEquals(listOf("favourite f0.1", "station 1"), calls)
    }

    @Test
    fun emptyListsSaySo() {
        show(lists = CarListsUi(loading = false))
        compose.onNodeWithTag("car-favourites").performClick()
        compose.onNodeWithText("No favourites to play").assertExists()
        compose.onNodeWithTag("car-back").performClick()
        compose.onNodeWithTag("car-stations").performClick()
        compose.onNodeWithText("No stations yet").assertExists()
    }

    @Test
    fun backLeavesCarModeFromThePlayingView() {
        val calls = mutableListOf<String>()
        show(actions = carActions(calls))
        compose.onNodeWithTag("car-favourites").performClick()
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.onNodeWithTag("car-play").assertExists()
        assertTrue(calls.isEmpty())
        compose.runOnUiThread { compose.activity.onBackPressedDispatcher.onBackPressed() }
        compose.waitForIdle()
        assertEquals(listOf("exit"), calls)
        compose.onNodeWithTag("car-exit").performClick()
        assertEquals(listOf("exit", "exit"), calls)
    }

    @Test
    fun theScreenStaysOnOnlyWhilePlaying() {
        var state by mutableStateOf(auditPlayerState())
        lateinit var view: View
        compose.setContent {
            view = LocalView.current
            LyrionTheme(dynamicColour = false) { CarModeScreen(state, carLists, clock = { 0L }, actions = carActions()) }
        }
        compose.waitForIdle()
        assertTrue(view.keepScreenOn)
        compose.runOnUiThread { state = state.copy(status = state.status!!.copy(mode = PlaybackMode.PAUSED)) }
        compose.waitForIdle()
        assertFalse(view.keepScreenOn)
    }

    @Test
    fun withoutVoiceOrRadioTheirButtonsGo() {
        show(actions = carActions().copy(onVoice = null, onStartRadio = null))
        compose.onNodeWithTag("car-voice").assertDoesNotExist()
        compose.onNodeWithTag("car-radio").assertDoesNotExist()
        // Nothing playing: still a usable screen.
        compose.onNodeWithTag("car-play").assertExists()
    }
}

/** Car mode's setting: the permission only when first needed; refused, Off with a note. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1200dp")
class CarModeSettingTests {
    @get:Rule
    val compose = createComposeRule()

    private val kept = mutableListOf<CarModeSetting>()

    private fun show(access: FakeCarAccess, initial: CarModeSetting = CarModeSetting()) {
        var setting by mutableStateOf(initial)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                CarModeSettingView(setting, onChange = {
                    kept += it
                    setting = it
                }, access = access)
            }
        }
    }

    @Test
    fun refusedGoesBackToOffWithANote() {
        val access = FakeCarAccess(permitted = false, answer = false)
        show(access)
        compose.onNodeWithTag("car-auto-ANY_CAR").performClick()
        assertEquals(1, access.asked)
        assertEquals(listOf(CarModeSetting()), kept)
        compose.onNodeWithTag("car-auto-refused").assertExists()
        // Off never asks.
        compose.onNodeWithTag("car-auto-OFF").performClick()
        assertEquals(1, access.asked)
    }

    @Test
    fun grantedAnyCarIsKeptAndNotAskedAgain() {
        val access = FakeCarAccess(permitted = false, answer = true)
        show(access)
        compose.onNodeWithTag("car-auto-ANY_CAR").performClick()
        assertEquals(CarModeSetting(CarAutoOpen.ANY_CAR), kept.last())
        compose.onNodeWithTag("car-auto-refused").assertDoesNotExist()
        compose.onNodeWithTag("car-auto-OFF").performClick()
        compose.onNodeWithTag("car-auto-ANY_CAR").performClick()
        assertEquals(1, access.asked)
    }

    @Test
    fun thisDeviceIsPickedFromThePairedOnes() {
        val access = FakeCarAccess(permitted = true)
        show(access)
        compose.onNodeWithTag("car-auto-THIS_DEVICE").performClick()
        compose.onNodeWithTag("car-pick-list").assertExists()
        compose.onNodeWithText("Car audio").assertExists()
        compose.onNodeWithTag("car-pick-0A:1B:2C:3D:4E:5F").performClick()
        assertEquals(CarModeSetting(CarAutoOpen.THIS_DEVICE, "Example Car Kit", "0A:1B:2C:3D:4E:5F"), kept.last())
        // The chosen device's name shows under the option.
        compose.onNodeWithText("Example Car Kit").assertExists()
        assertEquals(0, access.asked)
    }

    @Test
    fun noPairedDevicesSaysSoAndKeepsTheSetting() {
        val access = FakeCarAccess(permitted = true, devices = emptyList())
        show(access)
        compose.onNodeWithTag("car-auto-THIS_DEVICE").performClick()
        compose.onNodeWithText("No paired Bluetooth devices", substring = true).assertExists()
        compose.onNodeWithText("Cancel").performClick()
        assertTrue(kept.isEmpty())
    }
}

/** Per-output EQ in the Sound settings (Wave 2 §5.3): curve, A/B and bindings. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h2400dp")
class OutputEqScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private class Recorder : SoundActions {
        val calls = mutableListOf<String>()

        override fun bindOutput(kind: OutputClass, name: String, profile: EqProfile?) {
            calls += "bind $kind '$name' ${profile?.name}"
        }

        override fun unbindOutput(kind: OutputClass, name: String) {
            calls += "unbind $kind '$name'"
        }

        override fun importEqFor(kind: OutputClass, deviceName: String, name: String, text: String) {
            calls += "import $kind '$deviceName'"
        }

        override fun eqBypass(on: Boolean) {
            calls += "bypass $on"
        }
    }

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    @Test
    fun theCurveIsDescribed() {
        show { EqCurveView(mainEq) }
        compose.onNodeWithTag("eq-curve").assertExists()
        val described = compose.onNodeWithTag("eq-curve").fetchSemanticsNode().config[SemanticsProperties.ContentDescription].single()
        assertTrue(described, described.startsWith("The curve of Example profile, from −"))
    }

    @Test
    fun abBypassesWhileHeldOrTogglesWithTalkBack() {
        val calls = mutableListOf<Boolean>()
        var bypass by mutableStateOf(false)
        show {
            EqCompareButton(bypass, enabled = true) {
                calls += it
                bypass = it
            }
        }
        compose.onNodeWithTag("eq-compare").performTouchInput { down(center) }
        assertEquals(listOf(true), calls)
        compose.onNodeWithTag("eq-compare").performTouchInput { up() }
        assertEquals(listOf(true, false), calls)
        // TalkBack: a switch, pressed to turn the bypass on, and again to turn it off.
        compose.onNodeWithTag("eq-compare").performSemanticsAction(SemanticsActions.OnClick)
        assertEquals(true, calls.last())
        compose.onNodeWithTag("eq-compare").performSemanticsAction(SemanticsActions.OnClick)
        assertEquals(false, calls.last())
    }

    @Test
    fun abIsOffWhenNothingPlaysHere() {
        val calls = mutableListOf<Boolean>()
        show { EqCompareButton(false, enabled = false) { calls += it } }
        compose.onNodeWithTag("eq-compare").assertIsNotEnabled()
        compose.onNodeWithTag("eq-compare").performTouchInput { down(center) }
        compose.onNodeWithTag("eq-compare").performTouchInput { up() }
        // Only the screen leaving (releasing any bypass) calls it.
        assertTrue(calls.all { !it })
        compose.onNodeWithText("Play something on this phone to compare").assertExists()
    }

    @Test
    fun bindingTheCurrentOutputAndRemovingABinding() {
        val actions = Recorder()
        val state = SoundUiState(
            sound = SoundSettings(eqEnabled = true, eq = mainEq, outputEq = listOf(OutputEq(OutputClass.WIRED, "", null))),
            output = AudioOutput(OutputClass.BLUETOOTH, "Example Headphones"),
        )
        show { OutputEqSection(state, actions) }
        compose.onNodeWithText("Playing through Bluetooth · Example Headphones").assertExists()
        compose.onNodeWithTag("output-eq-target-0").performClick()
        compose.onNodeWithTag("output-eq-none").performClick()
        compose.onNodeWithTag("output-eq-use-main").performClick()
        // Any USB DAC.
        compose.onNodeWithTag("output-eq-target-3").performClick()
        compose.onNodeWithTag("output-eq-none").performClick()
        compose.onNodeWithTag("output-eq-remove-WIRED-").performClick()
        compose.onNodeWithTag("output-eq-paste").performClick()
        compose.onNodeWithTag("eq-paste").assertExists()
        assertEquals(
            listOf(
                "bind BLUETOOTH 'Example Headphones' null",
                "bind BLUETOOTH 'Example Headphones' Example profile",
                "bind USB '' null",
                "unbind WIRED ''",
            ),
            actions.calls,
        )
    }

    @Test
    fun theMainProfileCantBeBoundWithoutOne() {
        show { OutputEqSection(SoundUiState(), Recorder()) }
        compose.onNodeWithTag("output-eq-use-main").assertIsNotEnabled()
        compose.onNodeWithTag("output-eq-none").assertIsEnabled()
        compose.onNodeWithTag("output-eq-now").assertDoesNotExist()
    }

    @Test
    fun theSoundScreenShowsTheCurveAndTheComparison() {
        val state = SoundUiState(sound = SoundSettings(eqEnabled = true, eq = mainEq), output = AudioOutput(OutputClass.SPEAKER))
        show { SoundScreen(state, Recorder()) }
        compose.onNodeWithTag("eq-curve").assertExists()
        compose.onNodeWithTag("eq-compare").assertIsEnabled()
        compose.onNodeWithTag("output-eq").assertExists()
    }

    @Test
    fun theSignalPathSaysWhichProfileIsInUse() {
        val base = phonePlayingState()
        val playing = base.copy(local = base.local.copy(signalPath = signalPath(), sound = SoundSettings(eqEnabled = true, eq = mainEq)))
        var state by mutableStateOf(playing)
        show { NowPlayingScreen(state, clock = { 0L }, actions = nowPlayingActions()) }
        compose.onNodeWithTag("eq-chip").assertExists()
        compose.onNodeWithText("EQ: Example profile").assertExists()
        // Bypassed for a comparison: no chip.
        compose.runOnUiThread { state = playing.copy(local = playing.local.copy(eqBypass = true)) }
        compose.onNodeWithTag("eq-chip").assertDoesNotExist()
    }
}

/** Haptics (Wave 2 §5.4): light ticks where they belong, and none with the switch off. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1600dp")
class HapticsScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(haptics: RecordingHaptics, on: Boolean = true, content: @Composable () -> Unit) = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            CompositionLocalProvider(LocalHapticFeedback provides haptics) { ProvideHaptics(on) { content() } }
        }
    }

    @Test
    fun ratingAndTheSleepTimerTick() {
        val h = RecordingHaptics()
        show(h) {
            androidx.compose.foundation.layout.Column {
                RatingControl(TrackRating(9, null, null, null), RatingStyle.STARS, failed = false) {}
                SleepChoices(left = null, grouped = false, others = false, canEnd = false, fadeSecs = 30, onSet = { _, _ -> }, onCancel = {})
            }
        }
        compose.onNodeWithTag("rating-star-4").performClick()
        compose.onNodeWithTag("sleep-30").performClick()
        assertEquals(2, h.felt.size)
    }

    @Test
    fun scrubbingTicksAtEveryTenth() {
        val h = RecordingHaptics()
        show(h) { SeekBar(elapsed = 0.0, duration = 100.0, enabled = true, onSeek = {}) }
        val bar = compose.onNodeWithContentDescription("Position in track")
        // Past a tenth: a tick. Within the first tenth: none.
        bar.performSemanticsAction(SemanticsActions.SetProgress) { it(55f) }
        assertEquals(listOf(HapticFeedbackType.SegmentTick), h.felt)
        bar.performSemanticsAction(SemanticsActions.SetProgress) { it(5f) }
        assertEquals(1, h.felt.size)
        bar.performSemanticsAction(SemanticsActions.SetProgress) { it(99f) }
        assertEquals(2, h.felt.size)
    }

    @Test
    fun theSwitchOffMeansNoTicks() {
        val h = RecordingHaptics()
        show(h, on = false) { RatingControl(TrackRating(9, null, null, null), RatingStyle.STARS, failed = false) {} }
        compose.onNodeWithTag("rating-star-2").performClick()
        assertTrue(h.felt.isEmpty())
    }

    @Test
    fun theSettingsSwitches() {
        val changed = mutableListOf<String>()
        show(RecordingHaptics()) {
            androidx.compose.foundation.layout.Column {
                HapticsSetting(true, onChange = { changed += "haptics $it" }, systemOn = false)
                DynamicColourSetting(true, onChange = { changed += "colour $it" }, available = true)
            }
        }
        compose.onNodeWithText("Touch feedback is off in your phone’s settings", substring = true).assertExists()
        compose.onNodeWithTag("haptics-setting").performClick()
        compose.onNodeWithTag("dynamic-colour-setting").performClick()
        assertEquals(listOf("haptics false", "colour false"), changed)
    }

    @Test
    fun wallpaperColoursNeedAndroid12() {
        val changed = mutableListOf<Boolean>()
        show(RecordingHaptics()) { DynamicColourSetting(true, onChange = { changed += it }, available = false) }
        compose.onNodeWithTag("dynamic-colour-setting").assertIsNotEnabled()
        compose.onNodeWithText("Needs Android 12 or later").assertExists()
    }
}

/** The offline Now playing's visualiser (Wave 2 §5.7), and Now playing's car-mode button. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1600dp")
class OfflineVisualiserAndCarButtonTests {
    @get:Rule
    val compose = createComposeRule()

    private fun offlineUi(prefs: NowPlayingPrefs = NowPlayingPrefs()): OfflinePlayerUi {
        val tracks = listOf(dlTrack(1), dlTrack(2))
        return OfflinePlayerUi(OfflinePlayerState(status = dlStatus(), tracks = tracks.associateBy { it.key }), at = 0, nowPlaying = prefs)
    }

    @Test
    fun theVisualiserCanTakeTheArtsPlace() {
        val source = FakeVisualSource()
        val chosen = mutableListOf<String>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                OfflineNowPlayingScreen(
                    offlineUi(),
                    clock = { 0L },
                    actions = OfflineNowPlayingActions(onVisualiser = { chosen += "on $it" }, onVisualStyle = { chosen += "style $it" }),
                    visual = source,
                )
            }
        }
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        compose.onNodeWithTag("visualiser-toggle").performClick()
        compose.onNodeWithTag("visualiser").assertExists()
        assertTrue(source.active.contains(true))
        compose.onNodeWithTag("visualiser-style").performClick()
        assertEquals(listOf("on true", "style SCOPE"), chosen)
    }

    @Test
    fun withoutASourceThereIsNoVisualiserButton() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                OfflineNowPlayingScreen(offlineUi(NowPlayingPrefs(visualiser = true)), clock = { 0L }, actions = OfflineNowPlayingActions())
            }
        }
        compose.onNodeWithTag("visualiser-toggle").assertDoesNotExist()
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
    }

    @Test
    fun glowLightsTheBackgroundInsteadOfReplacingTheArt() {
        val source = FakeVisualSource()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                OfflineNowPlayingScreen(
                    offlineUi(NowPlayingPrefs(visualiser = true, visualStyle = VisualStyle.GLOW)),
                    clock = { 0L },
                    actions = OfflineNowPlayingActions(),
                    visual = source,
                )
            }
        }
        // The art stays (no visualiser in its place), and the analysis runs for the glow.
        compose.onNodeWithTag("visualiser").assertDoesNotExist()
        compose.onNodeWithTag("visualiser-style").assertExists()
        assertEquals(listOf(true), source.active)
    }

    @Test
    fun nowPlayingOpensCarMode() {
        var opened = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions().copy(onCarMode = { opened++ }))
            }
        }
        compose.onNodeWithTag("car-mode-button").performClick()
        assertEquals(1, opened)
    }
}
