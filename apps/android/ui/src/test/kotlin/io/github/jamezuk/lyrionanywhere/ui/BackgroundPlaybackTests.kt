package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.ui.settings.BackgroundPlaybackPrompt
import io.github.jamezuk.lyrionanywhere.ui.settings.BackgroundPlaybackSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Playing with the screen off: the one-time offer and the row in Settings › Playback. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class BackgroundPlaybackTests {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theOfferExplainsAndItsButtonsDoWhatTheySay() {
        val calls = mutableListOf<String>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                BackgroundPlaybackPrompt(refused = false, onAllow = { calls += "allow" }, onDismiss = { calls += "dismiss" })
            }
        }
        compose.onNodeWithText("Playing with the screen off").assertIsDisplayed()
        compose.onNodeWithText("battery optimisation", substring = true).assertIsDisplayed()
        compose.onNodeWithTag("background-playback-allow").performClick()
        compose.onNodeWithTag("background-playback-dismiss").performClick()
        assertEquals(listOf("allow", "dismiss"), calls)
    }

    @Test
    fun afterARefusalTheOfferSaysSo() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                BackgroundPlaybackPrompt(refused = true, onAllow = {}, onDismiss = {})
            }
        }
        compose.onNodeWithText("already stopped", substring = true).assertIsDisplayed()
    }

    @Test
    fun theSettingsIndexShowsTheOfferOnlyWhenGiven() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = SettingsUiState(),
                    appVersion = "1",
                    libraryVersion = "1",
                    onSignOut = {},
                    onRename = {},
                    onLicences = {},
                    backgroundPrompt = { BackgroundPlaybackPrompt(refused = false, onAllow = {}, onDismiss = {}) },
                )
            }
        }
        compose.onNodeWithTag("background-playback-prompt").assertIsDisplayed()
    }

    @Test
    fun theRowInPlaybackOpensTheSystemPage() {
        var opened = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = SettingsUiState(),
                    appVersion = "1",
                    libraryVersion = "1",
                    section = SettingsSection.PLAYBACK,
                    onSignOut = {},
                    onRename = {},
                    onLicences = {},
                    background = { BackgroundPlaybackSetting(onAllow = { opened++ }) },
                )
            }
        }
        compose.onNodeWithTag("background-playback-setting").assertIsDisplayed()
        compose.onNodeWithTag("background-playback-setting-allow").performClick()
        assertEquals(1, opened)
    }
}
