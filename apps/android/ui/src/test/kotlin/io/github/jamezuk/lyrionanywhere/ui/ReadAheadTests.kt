package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.ReadAheadSettings
import io.github.jamezuk.lyrionanywhere.ui.player.CushionParts
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.player.cushionParts
import io.github.jamezuk.lyrionanywhere.ui.player.formatCushion
import io.github.jamezuk.lyrionanywhere.ui.settings.ReadAheadSetting
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** "4 min buffered ahead": how the cushion reads (pure, so every edge is checked). */
class CushionFormatTest {
    @Test
    fun secondsMinutesAndHours() {
        assertEquals("45 s", formatCushion(45_000))
        assertEquals("45 s", formatCushion(45_999))
        assertEquals("1 s", formatCushion(1_000))
        assertEquals("59 s", formatCushion(59_999))
        assertEquals("1 min", formatCushion(60_000))
        assertEquals("4 min", formatCushion(4 * 60_000 + 30_000))
        assertEquals("59 min", formatCushion(3_599_999))
        assertEquals("1 h", formatCushion(3_600_000))
        assertEquals("1 h 5 min", formatCushion(3_600_000 + 5 * 60_000 + 59_000))
        assertEquals("26 h 1 min", formatCushion(26 * 3_600_000L + 60_000))
    }

    @Test
    fun nothingBelowASecondOrForNonsense() {
        for (ms in listOf(0L, 999L, -1L, Long.MIN_VALUE)) {
            assertNull("$ms", formatCushion(ms))
            assertNull("$ms", cushionParts(ms))
        }
        // The largest value the engine could report still reads sensibly.
        assertEquals(CushionParts(Long.MAX_VALUE / 3_600_000, Long.MAX_VALUE % 3_600_000 / 60_000, 0), cushionParts(Long.MAX_VALUE))
    }
}

/** Settings › Playback › Gaps in the signal, and the cushion in the signal path. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class ReadAheadSettingTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun bothSwitchesAreOnByDefaultAndTheMobileOneNeedsTheFirst() {
        val changes = mutableListOf<ReadAheadSettings>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                var s by remember { mutableStateOf(ReadAheadSettings()) }
                ReadAheadSetting(s) {
                    changes += it
                    s = it
                }
            }
        }
        compose.onNodeWithText("Gaps in the signal").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading))
        compose.onNodeWithTag("read-ahead-setting").assertIsOn()
        compose.onNodeWithTag("read-ahead-mobile-setting").assertIsOn().assertIsEnabled()
        compose.onNodeWithTag("read-ahead-mobile-setting").performClick()
        compose.onNodeWithTag("read-ahead-mobile-setting").assertIsOff()
        compose.onNodeWithTag("read-ahead-setting").performClick()
        compose.onNodeWithTag("read-ahead-setting").assertIsOff()
        // Without reading ahead, the mobile-data switch can't be changed.
        compose.onNodeWithTag("read-ahead-mobile-setting").assertIsOff().assertIsNotEnabled()
        compose.onNodeWithText("Turn on reading ahead first").assertExists()
        assertEquals(
            listOf(ReadAheadSettings(true, false), ReadAheadSettings(false, false)),
            changes,
        )
    }

    @Test
    fun theSignalPathShowsTheCushionAndReadsItInWords() {
        var cushion by mutableStateOf<Long?>(3_600_000L + 5 * 60_000)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignalPathPanel(signalPath(), cushionMs = cushion)
            }
        }
        compose.onNodeWithText("1 h 5 min buffered ahead").assertExists()
        compose.onNodeWithTag("signal-cushion").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("1 hour 5 minutes buffered ahead")),
        )
        cushion = 45_000
        compose.onNodeWithText("45 s buffered ahead").assertExists()
        compose.onNodeWithTag("signal-cushion").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("45 seconds buffered ahead")),
        )
        cushion = null
        compose.onNodeWithTag("signal-cushion").assertDoesNotExist()
        cushion = 400
        compose.onNodeWithTag("signal-cushion").assertDoesNotExist()
    }
}
