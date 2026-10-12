package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.data.StreamingSettings
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.settings.StreamingQualitySetting
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Settings › Streaming quality (docs/plans/streaming-quality.md). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class StreamingQualitySettingTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun showsTheLevelsAndChangesEachNetworkSeparately() {
        val changes = mutableListOf<StreamingSettings>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                var s by remember { mutableStateOf(StreamingSettings()) }
                StreamingQualitySetting(s) {
                    changes += it
                    s = it
                }
            }
        }
        compose.onNodeWithText("Wi-Fi: Original · Mobile data: High").assertExists()
        compose.onNodeWithTag("stream-quality-setting").performClick()
        compose.onNodeWithTag("stream-quality-wifi-original").assertIsSelected()
        compose.onNodeWithTag("stream-quality-mobile-high").assertIsSelected()
        compose.onNodeWithTag("stream-quality-mobile-low").assertIsNotSelected()
        compose.onNodeWithTag("stream-quality-mobile-low").performClick()
        compose.onNodeWithTag("stream-quality-mobile-low").assertIsSelected()
        compose.onNodeWithTag("stream-quality-wifi-high").performClick()
        assertEquals(
            listOf(
                StreamingSettings(StreamQuality.ORIGINAL, StreamQuality.LOW),
                StreamingSettings(StreamQuality.HIGH, StreamQuality.LOW),
            ),
            changes,
        )
        compose.onNodeWithText("Done").performClick()
        compose.onNodeWithText("Wi-Fi: High · Mobile data: Data saver").assertExists()
    }

    @Test
    fun theSignalPathSaysWhenTheStreamIsReEncoded() {
        var quality by mutableStateOf(StreamQuality.HIGH)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignalPathPanel(signalPath(), quality = quality, metered = true)
            }
        }
        compose.onNodeWithText("High on mobile data (lossless tracks re-encoded to Opus by the server)").assertExists()
        quality = StreamQuality.ORIGINAL
        compose.onNodeWithText("Streaming").assertDoesNotExist()
    }
}
