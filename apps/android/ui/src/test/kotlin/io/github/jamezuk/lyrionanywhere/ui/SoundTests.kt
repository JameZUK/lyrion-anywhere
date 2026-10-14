package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.assertTextContains
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.NotBitPerfect
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.data.CrossfeedMode
import io.github.jamezuk.lyrionanywhere.data.DitherMode
import io.github.jamezuk.lyrionanywhere.data.EqProfile
import io.github.jamezuk.lyrionanywhere.data.LevellingMode
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.TransitionsMode
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.settings.EqMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.formatDb
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private class RecordingActions : SoundActions {
    val calls = mutableListOf<String>()

    override fun levelling(mode: LevellingMode) {
        calls += "levelling $mode"
    }

    override fun bitPerfectUsb(on: Boolean) {
        calls += "usb $on"
    }

    override fun limiter(on: Boolean) {
        calls += "limiter $on"
    }

    override fun eqEnabled(on: Boolean) {
        calls += "eq $on"
    }

    override fun importEq(name: String, text: String) {
        calls += "import $name: $text"
    }

    override fun removeEq() {
        calls += "remove"
    }

    override fun crossfeed(mode: CrossfeedMode) {
        calls += "crossfeed $mode"
    }

    override fun dither(mode: DitherMode) {
        calls += "dither $mode"
    }

    override fun messageShown() {
        calls += "shown"
    }

    override fun transitions(mode: TransitionsMode) {
        calls += "transitions $mode"
    }

    override fun crossfadeSecs(secs: Double) {
        calls += "crossfade $secs"
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h3000dp")
class SoundScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private val actions = RecordingActions()

    private fun show(state: SoundUiState) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) { SoundScreen(state, actions) }
        }
    }

    private fun scrollTo(text: String) {
        compose.onNode(hasScrollAction()).performScrollToNode(hasText(text))
    }

    /** Open the picker of the choice row tagged [tag] (Wave 3.5: modes are rows with pickers). */
    private fun open(tag: String) {
        compose.onNode(hasScrollAction()).performScrollToNode(androidx.compose.ui.test.hasTestTag(tag))
        compose.onNodeWithTag(tag).performClick()
    }

    private fun option(text: String) = compose.onNode(hasText(text) and androidx.compose.ui.test.isSelectable())

    @Test
    fun theDefaultIsBitPerfect() {
        show(SoundUiState())
        compose.onNodeWithTag("sound-indicator").assertTextContains("Bit-perfect")
        compose.onNodeWithText("Every stage is off: samples leave the player exactly as decoded.").assertIsDisplayed()
        compose.onNode(hasText("Bit-perfect USB output") and isToggleable()).assertIsOn()
        open("sound-levelling")
        option("Off").assertIsSelected()
    }

    @Test
    fun transitionsAreOffByDefaultAndCanBeChosen() {
        show(SoundUiState())
        compose.onNodeWithTag("crossfade").assertDoesNotExist()
        open("sound-transitions")
        option("Gapless").assertIsSelected()
        option("Smart transitions").performClick()
        assertEquals(listOf("transitions SMART"), actions.calls)
    }

    @Test
    fun aCrossfadeShowsItsLengthAndIsNotBitPerfect() {
        show(SoundUiState(SoundSettings(transitions = TransitionsMode.CROSSFADE, crossfadeSecs = 4.0)))
        compose.onNodeWithTag("sound-indicator").assertTextContains("Processing")
        compose.onNodeWithText(
            "These change the samples: crossfade. Turn them off for bit-perfect playback.",
        ).assertIsDisplayed()
        scrollTo("Crossfade: up to 4 s")
        compose.onNodeWithText("Crossfade: up to 4 s").assertIsDisplayed()
        open("sound-transitions")
        option("Crossfade").assertIsSelected()
    }

    @Test
    fun anActiveStageReplacesTheBitPerfectIndicator() {
        show(SoundUiState(SoundSettings(preampDb = -3.0, levelling = LevellingMode.R128)))
        compose.onNodeWithTag("sound-indicator").assertTextContains("Processing")
        compose.onNodeWithText(
            "These change the samples: preamp, levelling. Turn them off for bit-perfect playback.",
        ).assertIsDisplayed()
        compose.onNodeWithText("Preamp: −3.0 dB").assertIsDisplayed()
        open("sound-levelling")
        option("Server analysis (EBU R128)").assertIsSelected()
    }

    @Test
    fun choicesReachTheActions() {
        show(SoundUiState())
        open("sound-levelling")
        option("ReplayGain, album").performClick()
        compose.onNode(hasText("Bit-perfect USB output") and isToggleable()).performClick()
        compose.onNode(hasText("True-peak limiter") and isToggleable()).performClick()
        open("sound-crossfeed")
        option("Natural").performClick()
        open("sound-dither")
        option("TPDF, noise-shaped").performClick()
        assertEquals(
            listOf(
                "levelling REPLAYGAIN_ALBUM",
                "usb false",
                "limiter false",
                "crossfeed NATURAL",
                "dither TPDF_SHAPED",
            ),
            actions.calls,
        )
    }

    @Test
    fun bitPerfectUsbIsUnavailableBeforeAndroid14() {
        show(SoundUiState(bitPerfectAvailable = false))
        compose.onNode(hasText("Bit-perfect USB output") and isToggleable()).assertIsNotEnabled()
        compose.onNodeWithText("Needs Android 14 or later. On this phone, audio goes through Android's mixer.")
            .assertIsDisplayed()
    }

    @Test
    fun aProfileIsPastedAndImported() {
        show(SoundUiState())
        scrollTo("Paste a profile")
        compose.onNodeWithText("Paste a profile").performClick()
        compose.onNodeWithText("Paste an AutoEq profile").assertIsDisplayed()
        compose.onNodeWithText("Import").assertIsNotEnabled()
        compose.onNodeWithTag("eq-paste").performTextInput("Filter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1")
        compose.onNodeWithText("Import").performClick()
        assertEquals(listOf("import Pasted profile: Filter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1"), actions.calls)
        compose.onNodeWithText("Paste an AutoEq profile").assertDoesNotExist()
    }

    @Test
    fun pastingCanBeCancelled() {
        show(SoundUiState())
        scrollTo("Paste a profile")
        compose.onNodeWithText("Paste a profile").performClick()
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithText("Paste an AutoEq profile").assertDoesNotExist()
        assertTrue(actions.calls.isEmpty())
    }

    @Test
    fun withoutAProfileTheEqCantBeSwitchedOn() {
        show(SoundUiState())
        scrollTo("Parametric EQ")
        compose.onNode(hasText("Parametric EQ") and isToggleable()).assertIsNotEnabled()
        compose.onNodeWithText("Remove the profile").assertDoesNotExist()
    }

    @Test
    fun aLoadedProfileIsDescribedAndRemovable() {
        val eq = EqProfile("Headphones.txt", "Filter 1: …", 7, -6.4)
        show(SoundUiState(SoundSettings(eq = eq, eqEnabled = true)))
        scrollTo("Parametric EQ")
        compose.onNodeWithText("Headphones.txt · 7 filters · preamp −6.4 dB").assertIsDisplayed()
        compose.onNode(hasText("Parametric EQ") and isToggleable()).assertIsOn().performClick()
        scrollTo("Remove the profile")
        compose.onNodeWithText("Remove the profile").performClick()
        assertEquals(listOf("eq false", "remove"), actions.calls)
    }

    @Test
    fun aRefusedImportSaysWhy() {
        show(SoundUiState(message = EqMessage.Invalid("line 2: unknown filter type")))
        compose.onNodeWithText("Not imported: line 2: unknown filter type").assertIsDisplayed()
        compose.onNodeWithText("OK").performClick()
        assertEquals(listOf("shown"), actions.calls)
    }

    @Test
    fun theSignalPathExplainsWhyItIsntBitPerfect() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignalPathPanel(
                    signalPath(
                        verdict = SignalVerdict.LOSSLESS_PROCESSED,
                        engineBitPerfect = false,
                        stages = listOf(
                            SignalStage("Decode (lossless)", true, false),
                            SignalStage("Preamp −3.0 dB", false, false),
                        ),
                        reasons = listOf(NotBitPerfect.NO_USB_DAC, NotBitPerfect.DSP_ACTIVE),
                    ),
                )
            }
        }
        compose.onNodeWithText("Not bit-perfect end to end:").assertIsDisplayed()
        compose.onNodeWithText("• the output isn't a USB DAC, so Android's mixer handles it").assertIsDisplayed()
        compose.onNodeWithText("• a Sound setting changes the samples").assertIsDisplayed()
    }

    @Test
    fun aBitPerfectDacShowsItsFormatAndTheVolumeNote() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignalPathPanel(signalPath(bitPerfect = true))
            }
        }
        compose.onNodeWithText("Example DAC · 96 kHz · 24-bit integer").assertIsDisplayed()
        compose.onNodeWithText("Every sample reaches the device unchanged.").assertIsDisplayed()
        compose.onNodeWithText(
            "Volume: use your DAC's. The server's volume stays at 100 % so no sample is changed.",
        ).assertIsDisplayed()
        compose.onNodeWithText("Not bit-perfect end to end:").assertDoesNotExist()
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class SoundViewModelTest : MainDispatcherTest() {
    @Test
    fun changesAreSavedThroughThePlayerAndClamped() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val vm = SoundViewModel(local, sdk = 34)
        assertTrue(vm.state.value.bitPerfectAvailable)
        vm.setPreamp(-3.26)
        assertEquals(-3.5, vm.state.value.sound.preampDb, 1e-9)
        vm.setPreamp(40.0)
        assertEquals(12.0, local.sounds.last().preampDb, 0.0)
        vm.setCeiling(-0.34)
        assertEquals(-0.3, local.sounds.last().limiterCeilingDb, 1e-9)
        vm.setLevelling(LevellingMode.REPLAYGAIN_TRACK)
        vm.setCrossfeed(CrossfeedMode.MODERATE)
        vm.setDither(DitherMode.OFF)
        vm.setBitPerfectUsb(false)
        val s = local.state.value.sound
        assertEquals(LevellingMode.REPLAYGAIN_TRACK, s.levelling)
        assertEquals(CrossfeedMode.MODERATE, s.crossfeed)
        assertEquals(DitherMode.OFF, s.dither)
        assertFalse(s.bitPerfectUsb)
        // No profile: the EQ can't be switched on.
        vm.setEqEnabled(true)
        assertFalse(local.sounds.last().eqEnabled)
        assertFalse(SoundViewModel(local, sdk = 33).state.value.bitPerfectAvailable)
    }

    @Test
    fun transitionSettingsAreSavedRoundedAndClamped() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val vm = SoundViewModel(local, sdk = 34)
        vm.setTransitions(TransitionsMode.SMART)
        vm.setCrossfadeSecs(4.4)
        assertEquals(TransitionsMode.SMART, local.state.value.sound.transitions)
        assertEquals(4.0, local.sounds.last().crossfadeSecs, 0.0)
        vm.setCrossfadeSecs(50.0)
        assertEquals(10.0, local.sounds.last().crossfadeSecs, 0.0)
        vm.setTransitions(TransitionsMode.OFF)
        assertTrue("gapless again: bit-perfect", local.state.value.sound.bitPerfect)
    }

    @Test
    fun profilesAreImportedOnlyWhenTheEngineAcceptsThem() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val vm = SoundViewModel(local, sdk = 34)
        vm.importEq("nonsense.txt", "hello")
        assertEquals(EqMessage.Invalid("line 1: not a profile"), vm.state.value.message)
        assertNull(local.state.value.sound.eq)
        vm.messageShown()
        assertNull(vm.state.value.message)

        vm.importEq("  Head\u0000phones.txt ", "Preamp: -3 dB\nFilter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1\nFilter 2: ON PK Fc 90 Hz Gain 2 dB Q 1")
        val s = local.state.value.sound
        assertEquals("Headphones.txt", s.eq?.name)
        assertEquals(2, s.eq?.filters)
        assertTrue("an import turns the EQ on", s.eqEnabled)
        assertFalse(s.bitPerfect)
        assertEquals(EqMessage.Imported("Headphones.txt"), vm.state.value.message)

        vm.setEqEnabled(false)
        assertTrue(local.state.value.sound.bitPerfect)
        vm.removeEq()
        assertNull(local.state.value.sound.eq)
        vm.eqUnreadable()
        assertEquals(EqMessage.Unreadable, vm.state.value.message)
    }

    @Test
    fun decibelsAreFormattedWithATypographicMinus() {
        assertEquals("−3.5 dB", formatDb(-3.5))
        assertEquals("+2.0 dB", formatDb(2.0))
        assertEquals("0.0 dB", formatDb(0.0))
    }
}
