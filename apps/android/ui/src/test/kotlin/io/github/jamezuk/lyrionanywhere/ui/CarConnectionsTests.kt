package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsSection
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Settings › About › Car connections (Android Auto diagnostics, on this phone only). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class CarConnectionsTests {
    @get:Rule
    val compose = createComposeRule()

    private fun about(lines: List<String>?) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SettingsScreen(
                    state = SettingsUiState(),
                    appVersion = "1",
                    libraryVersion = "1",
                    section = SettingsSection.ABOUT,
                    onSignOut = {},
                    onLicences = {},
                    carConnections = lines,
                )
            }
        }
    }

    @Test
    fun noneYet() {
        about(emptyList())
        compose.onNodeWithTag("car-connections").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Car connections").assertIsDisplayed()
        compose.onNodeWithText("None yet").assertIsDisplayed()
        compose.assertAccessible()
    }

    @Test
    fun theLastDecisionsAreListed() {
        about(
            listOf(
                "Android Auto · refused (certificate not recognised) · 12:04\nCertificates offered: ab:cd:ef:01",
                "Android Auto · allowed · 12:03",
            ),
        )
        compose.onNodeWithTag("car-connections").performScrollTo()
        compose.onNodeWithText("Android Auto · allowed · 12:03").assertIsDisplayed()
        compose.onNodeWithText("Certificates offered: ab:cd:ef:01", substring = true).assertIsDisplayed()
        compose.onNodeWithText("None yet").assertDoesNotExist()
    }

    @Test
    fun notShownWithoutTheLog() {
        about(null)
        compose.onNodeWithTag("car-connections").assertDoesNotExist()
    }
}
