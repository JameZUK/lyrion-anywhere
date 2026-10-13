package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasTestTag
import androidx.compose.ui.test.hasScrollAction
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.assertTextContains
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.NotBitPerfect
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.SoundStage
import io.github.jamezuk.lyrionanywhere.data.dspFor
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundActions
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val LOUDNESS_STAGE = "Loudness +5.0 dB bass, +2.0 dB treble"
private const val LOUDNESS_BYPASSED = "Loudness bypassed (bit-perfect output)"

private val loudnessPath = signalPath(
    verdict = SignalVerdict.LOSSLESS_PROCESSED,
    engineBitPerfect = false,
    stages = listOf(
        SignalStage("Decode (lossless)", true, false),
        SignalStage("Volume −20.0 dB", false, false),
        SignalStage(LOUDNESS_STAGE, false, false),
        SignalStage("True-peak limiter −1.0 dBTP", false, false),
    ),
    reasons = listOf(NotBitPerfect.NO_USB_DAC, NotBitPerfect.DSP_ACTIVE, NotBitPerfect.OTHER_PROCESSING),
)

private val bypassedPath = signalPath(
    bitPerfect = true,
    stages = listOf(
        SignalStage("Decode (lossless)", true, false),
        SignalStage(LOUDNESS_BYPASSED, true, false),
    ),
)

/** Loudness-compensated volume in Settings › Sound and on the signal path. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h3000dp")
class LoudnessScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private val calls = mutableListOf<Boolean>()
    private val actions = object : SoundActions {
        override fun loudness(on: Boolean) {
            calls += on
        }
    }

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private fun scrollToSwitch() {
        compose.onNode(hasScrollAction()).performScrollToNode(hasTestTag("loudness-volume"))
    }

    @Test
    fun offByDefaultNextToPerOutputEqWithItsExplanation() {
        show { SoundScreen(SoundUiState(), actions) }
        scrollToSwitch()
        compose.onNodeWithTag("loudness-volume").assertIsOff().assertIsDisplayed()
        compose.onNodeWithText("Loudness-compensated volume").assertIsDisplayed()
        compose.onNodeWithText(
            "Adds bass and treble as this phone's player is turned down, where the ear hears less of them. Bypassed for bit-perfect output.",
        ).assertIsDisplayed()
        // It sits in the EQ section, right after per-output EQ.
        val eqBottom = compose.onNodeWithTag("output-eq").fetchSemanticsNode().boundsInRoot.bottom
        val switchTop = compose.onNodeWithTag("loudness-volume").fetchSemanticsNode().boundsInRoot.top
        assertTrue("$eqBottom ≤ $switchTop", eqBottom <= switchTop + 1f)
        compose.onNodeWithTag("loudness-volume").performClick()
        assertEquals(listOf(true), calls)
    }

    @Test
    fun onItIsAStageThatChangesTheSamples() {
        show { SoundScreen(SoundUiState(SoundSettings(loudness = true)), actions) }
        compose.onNodeWithTag("sound-indicator").assertTextContains("Processing")
        compose.onNodeWithText(
            "These change the samples: loudness compensation. Turn them off for bit-perfect playback.",
        ).assertIsDisplayed()
        scrollToSwitch()
        compose.onNodeWithTag("loudness-volume").assertIsOn().performClick()
        assertEquals(listOf(false), calls)
    }

    @Test
    fun theSignalPathShowsTheLoudnessStage() {
        show { SignalPathPanel(loudnessPath) }
        compose.onNodeWithText(LOUDNESS_STAGE).assertIsDisplayed()
        compose.onNodeWithContentDescription("$LOUDNESS_STAGE, changes samples").assertExists()
        compose.onNodeWithText("• a Sound setting changes the samples").assertIsDisplayed()
    }

    @Test
    fun aBitPerfectRouteSaysLoudnessWasBypassed() {
        show { SignalPathPanel(bypassedPath) }
        compose.onNodeWithText(LOUDNESS_BYPASSED).assertIsDisplayed()
        compose.onNodeWithContentDescription("$LOUDNESS_BYPASSED, bit-transparent").assertExists()
        compose.onNodeWithText("Every sample reaches the device unchanged.").assertIsDisplayed()
        compose.onNodeWithText("Not bit-perfect end to end:").assertDoesNotExist()
    }

    @Test
    fun theSwitchIsOneTargetWithItsNameAndState() {
        show { SoundScreen(SoundUiState(SoundSettings(loudness = true)), actions) }
        scrollToSwitch()
        val node = compose.onNodeWithTag("loudness-volume").fetchSemanticsNode()
        assertTrue(node.config.contains(SemanticsProperties.ToggleableState))
        assertEquals(androidx.compose.ui.semantics.Role.Switch, node.config[SemanticsProperties.Role])
    }
}

/** The new control and stages, audited as TalkBack sees them. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h3000dp")
class LoudnessAccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    @Test
    fun soundWithLoudnessOff() = audit { SoundScreen(SoundUiState(), object : SoundActions {}) }

    @Test
    fun soundWithLoudnessOn() = audit { SoundScreen(SoundUiState(SoundSettings(loudness = true)), object : SoundActions {}) }

    @Test
    fun signalPathWithLoudness() = audit { SignalPathPanel(loudnessPath) }

    @Test
    fun signalPathWithLoudnessBypassed() = audit { SignalPathPanel(bypassedPath) }
}

@OptIn(ExperimentalCoroutinesApi::class)
class LoudnessViewModelTest : MainDispatcherTest() {
    @Test
    fun theSwitchIsSavedThroughThePlayerAndReachesTheEngine() = runTest(dispatcher) {
        val local = FakeLocalPlayer()
        val vm = SoundViewModel(local, sdk = 34)
        assertFalse(vm.state.value.sound.loudness)
        vm.setLoudness(true)
        assertTrue(local.sounds.last().loudness)
        assertTrue(vm.state.value.sound.loudness)
        assertEquals(listOf(SoundStage.LOUDNESS), local.state.value.sound.activeStages)
        assertTrue(dspFor(local.sounds.last()).loudness)
        vm.setLoudness(false)
        assertFalse(local.sounds.last().loudness)
        assertTrue("off again: bit-perfect", local.state.value.sound.bitPerfect)
    }
}
