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
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.ReplayCache
import io.github.jamezuk.lyrionanywhere.data.ReplayCacheUsage
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
import io.github.jamezuk.lyrionanywhere.ui.settings.ReplayCacheSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.replayCacheSizeName
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** How the replay cache's sizes read (pure). */
class ReplayCacheSizeNameTest {
    @Test
    fun offMegabytesAndGigabytes() {
        assertNull(replayCacheSizeName(0))
        assertNull(replayCacheSizeName(-1))
        assertEquals(
            listOf(null, "500 MB", "1 GB", "2 GB", "5 GB"),
            ReplayCache.CHOICES.map(::replayCacheSizeName),
        )
    }
}

/** Settings › Gaps in the signal › Replay cache, and "From cache" in the signal path. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h2400dp")
class ReplayCacheSettingTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun oneGigabyteByDefaultTheChoicesUsageAndClear() {
        val sizes = mutableListOf<Long>()
        var clears = 0
        var usage by mutableStateOf<ReplayCacheUsage?>(ReplayCacheUsage(5L * 1024 * 1024 * 1024 / 10, 12))
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                var bytes by remember { mutableStateOf(ReplayCache.DEFAULT_BYTES) }
                ReplayCacheSetting(
                    bytes = bytes,
                    usage = usage,
                    downloadsAllowed = true,
                    onSize = {
                        sizes += it
                        bytes = it
                    },
                    onClear = { clears++ },
                )
            }
        }
        compose.onNodeWithText("Replay cache").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading))
        // The sizes are in the Size row's picker (Wave 3.5).
        compose.onNodeWithTag("replay-cache-size").performClick()
        compose.onNodeWithTag("replay-cache-size-${ReplayCache.GB}").assertIsSelected()
        compose.onNodeWithText("500 MB").assertExists()
        compose.onNodeWithText("5 GB").assertExists()
        compose.onNodeWithText("Off").assertExists()
        compose.onNodeWithTag("replay-cache-usage").assert(
            SemanticsMatcher.expectValue(SemanticsProperties.Text, listOf(androidx.compose.ui.text.AnnotatedString("512.0 MB for 12 tracks"))),
        )
        compose.onNodeWithTag("replay-cache-size-0").performClick()
        compose.onNodeWithTag("replay-cache-size").performClick()
        compose.onNodeWithTag("replay-cache-size-0").assertIsSelected()
        compose.onNodeWithTag("replay-cache-size-${ReplayCache.GB}").assertIsNotSelected()
        compose.onNodeWithTag("replay-cache-size-${2 * ReplayCache.GB}").performClick()
        assertEquals(listOf(0L, 2 * ReplayCache.GB), sizes)
        compose.onNodeWithTag("replay-cache-clear").assertIsEnabled().performClick()
        assertEquals(1, clears)
        // Nothing kept: nothing to clear.
        usage = ReplayCacheUsage(0, 0)
        compose.onNodeWithText("Nothing kept yet").assertExists()
        compose.onNodeWithTag("replay-cache-clear").assertIsNotEnabled()
        usage = null
        compose.onNodeWithTag("replay-cache-clear").assertIsNotEnabled()
        // One track reads in the singular.
        usage = ReplayCacheUsage(1024 * 1024, 1)
        compose.onNodeWithText("1.0 MB for 1 track").assertExists()
    }

    @Test
    fun withoutTheDownloadPermissionNothingCanBeChosenAndItSaysWhy() {
        val sizes = mutableListOf<Long>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ReplayCacheSetting(
                    bytes = ReplayCache.DEFAULT_BYTES,
                    usage = ReplayCacheUsage(0, 0),
                    downloadsAllowed = false,
                    onSize = { sizes += it },
                    onClear = {},
                )
            }
        }
        compose.onNodeWithTag("replay-cache-explain").assert(
            SemanticsMatcher.expectValue(
                SemanticsProperties.Text,
                listOf(
                    androidx.compose.ui.text.AnnotatedString(
                        "Your account isn't allowed to download, so nothing is kept on this phone: tracks fetched ahead are deleted once they have played.",
                    ),
                ),
            ),
        )
        compose.onNodeWithTag("replay-cache-size").performClick()
        for (c in ReplayCache.CHOICES) {
            compose.onNodeWithTag("replay-cache-size-$c").assertIsNotEnabled().assertIsNotSelected()
        }
        compose.onNodeWithTag("replay-cache-size-0").performClick()
        assertEquals(emptyList<Long>(), sizes)
        compose.onNodeWithTag("replay-cache-clear").assertDoesNotExist()
        compose.onNodeWithTag("replay-cache-usage").assertDoesNotExist()
    }

    @Test
    fun whileThePermissionIsNotKnownTheChoicesWork() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ReplayCacheSetting(ReplayCache.DEFAULT_BYTES, null, downloadsAllowed = null, onSize = {}, onClear = {})
            }
        }
        compose.onNodeWithTag("replay-cache-size").performClick()
        compose.onNodeWithTag("replay-cache-size-0").assertIsEnabled()
        compose.onNodeWithText("Nothing kept yet").assertExists()
    }

    @Test
    fun theSignalPathSaysFromCache() {
        var path by mutableStateOf(signalPath().copy(fromCache = true))
        compose.setContent {
            LyrionTheme(dynamicColour = false) { SignalPathPanel(path) }
        }
        compose.onNodeWithTag("signal-from-cache").assertExists()
        compose.onNodeWithText("From cache").assertExists()
        path = signalPath()
        compose.onNodeWithTag("signal-from-cache").assertDoesNotExist()
    }

    @Test
    fun theSignalPathSaysFromDownloads() {
        var path by mutableStateOf(signalPath().copy(fromDownloads = true))
        compose.setContent {
            LyrionTheme(dynamicColour = false) { SignalPathPanel(path) }
        }
        compose.onNodeWithTag("signal-from-downloads").assertExists()
        compose.onNodeWithText("From downloads").assertExists()
        compose.onNodeWithTag("signal-from-cache").assertDoesNotExist()
        path = signalPath()
        compose.onNodeWithTag("signal-from-downloads").assertDoesNotExist()
    }
}
