package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.OutputClock
import io.github.jamezuk.lyrionanywhere.data.OutputTiming
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The output clock in the signal path: whether what this phone reports to LMS as heard
 * comes from the device's own timestamp or an estimate, and the latency, so sync problems
 * can be told apart on the phone itself.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class OutputClockTest {
    @get:Rule
    val compose = createComposeRule()

    private fun spoken(text: String) = SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf(text))

    @Test
    fun theSignalPathSaysWhichClockAndTheLatency() {
        var timing by mutableStateOf<OutputTiming?>(OutputTiming(OutputClock.DEVICE, 38, 0))
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignalPathPanel(signalPath(), outputTiming = timing)
            }
        }
        compose.onNodeWithText("Output clock: device · latency 38 ms").assertExists()
        compose.onNodeWithTag("signal-output-clock")
            .assert(spoken("Output clock: the device's own timestamp, latency 38 milliseconds"))

        timing = OutputTiming(OutputClock.ESTIMATED, 90, 0)
        compose.onNodeWithText("Output clock: estimated · latency about 90 ms").assertExists()
        compose.onNodeWithTag("signal-output-clock")
            .assert(spoken("Output clock: estimated, latency about 90 milliseconds"))
        compose.onNodeWithText("clock error", substring = true).assertDoesNotExist()

        // Errors show only when there are any.
        timing = OutputTiming(OutputClock.ESTIMATED, 90, 12)
        compose.onNodeWithText("Output clock: estimated · latency about 90 ms · 12 clock errors a minute").assertExists()
        compose.onNodeWithTag("signal-output-clock")
            .assert(spoken("Output clock: estimated, latency about 90 milliseconds, 12 clock errors a minute"))
        timing = OutputTiming(OutputClock.DEVICE, 1, 1)
        compose.onNodeWithText("Output clock: device · latency 1 ms · 1 clock error a minute").assertExists()

        // No output open: no line.
        timing = null
        compose.onNodeWithTag("signal-output-clock").assertDoesNotExist()
    }

    /** Robustness: absurd values from the engine still render, clamped. */
    @Test
    fun absurdValuesStillRender() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignalPathPanel(signalPath(), outputTiming = OutputTiming(OutputClock.DEVICE, Long.MAX_VALUE, -5))
            }
        }
        compose.onNodeWithText("Output clock: device · latency ${Int.MAX_VALUE} ms").assertExists()
    }
}
