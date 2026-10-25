package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.assertTextContains
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.data.StreamingSettings

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
        // A row with a picker each (Wave 3.5), showing the current level.
        compose.onNodeWithTag("stream-quality-setting").assertExists()
        compose.onNodeWithTag("stream-quality-wifi").assertTextContains("Original")
        compose.onNodeWithTag("stream-quality-mobile").assertTextContains("High")
        compose.onNodeWithTag("stream-quality-wifi").performClick()
        compose.onNodeWithTag("stream-quality-wifi-original").assertIsSelected()
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithTag("stream-quality-mobile").performClick()
        compose.onNodeWithTag("stream-quality-mobile-high").assertIsSelected()
        compose.onNodeWithTag("stream-quality-mobile-low").assertIsNotSelected()
        compose.onNodeWithTag("stream-quality-mobile-low").performClick()
        compose.onNodeWithTag("stream-quality-mobile").performClick()
        compose.onNodeWithTag("stream-quality-mobile-low").assertIsSelected()
        compose.onNodeWithText("Cancel").performClick()
        compose.onNodeWithTag("stream-quality-wifi").performClick()
        compose.onNodeWithTag("stream-quality-wifi-high").performClick()
        assertEquals(
            listOf(
                StreamingSettings(StreamQuality.ORIGINAL, StreamQuality.LOW),
                StreamingSettings(StreamQuality.HIGH, StreamQuality.LOW),
            ),
            changes,
        )
        compose.onNodeWithTag("stream-quality-wifi").assertTextContains("High")
        compose.onNodeWithTag("stream-quality-mobile").assertTextContains("Data saver")
    }

    /** An override chosen in the signal path is noted here, and these settings are left as they are. */
    @Test
    fun anOverrideInForceIsNoted() {
        var override by mutableStateOf<StreamQuality?>(StreamQuality.ORIGINAL)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                StreamingQualitySetting(StreamingSettings(), override = override) {}
            }
        }
        compose.onNodeWithTag("stream-quality-override")
            .assertTextContains("For now: Original, chosen in the signal path", substring = true)
        compose.onNodeWithTag("stream-quality-mobile").assertTextContains("High")
        override = null
        compose.onNodeWithTag("stream-quality-override").assertDoesNotExist()
    }
}
