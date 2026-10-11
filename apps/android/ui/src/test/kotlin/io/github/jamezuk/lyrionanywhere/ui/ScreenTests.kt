package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInScreen
import io.github.jamezuk.lyrionanywhere.ui.onboarding.SignInUiState
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class ScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun signInButton() = compose.onNode(hasText("Sign in") and hasClickAction())

    private fun signIn(state: SignInUiState) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                SignInScreen(
                    serverName = "music.example.com",
                    state = state,
                    onUsernameChange = {},
                    onPasswordChange = {},
                    onCodeChange = {},
                    onSubmit = {},
                    onPair = {},
                    onChangeServer = {},
                )
            }
        }
    }

    @Test
    fun signInAsksForTheCodeOnlyWhenNeeded() {
        signIn(SignInUiState(username = "alice", password = "pw"))
        compose.onNodeWithText("Two-step code").assertDoesNotExist()
        signInButton().assertIsDisplayed()
        compose.onNodeWithContentDescription("Show password").assertIsDisplayed()
    }

    @Test
    fun signInWithTwoStepNeedsAllSixDigits() {
        signIn(SignInUiState(username = "alice", password = "pw", needsCode = true, code = "123"))
        compose.onNodeWithText("Two-step code").assertIsDisplayed()
        signInButton().assertIsNotEnabled()
    }

    @Test
    fun aRefusedSignInSaysSoPlainly() {
        signIn(SignInUiState(username = "alice", password = "pw", error = AppError.SignedOut))
        compose.onNodeWithText("Wrong user name, password or code.").assertIsDisplayed()
        signInButton().assertIsEnabled()
    }

    @Test
    fun rateLimitsGiveTheWait() {
        var retried = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                ErrorState(AppError.RateLimited(600), onRetry = { retried++ })
            }
        }
        compose.onNodeWithText("Too many attempts. Try again in 10 minutes.").assertIsDisplayed()
        compose.onNodeWithText("Try again").performClick()
        assertEquals(1, retried)
    }

    @Test
    fun nowPlayingControlsAreLabelledForAccessibility() {
        var toggled = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(
                    state = PlayerUiState(
                        players = listOf(samplePlayer("aa:00:00:00:00:01")),
                        selectedId = "aa:00:00:00:00:01",
                        status = sampleStatus(),
                        loading = false,
                    ),
                    clock = { 0L },
                    actions = NowPlayingActions(
                        onTogglePlay = { toggled++ },
                        onNext = {},
                        onPrevious = {},
                        onSeek = {},
                        onVolume = {},
                        onShuffle = {},
                        onRepeat = {},
                        onPower = {},
                        onJump = {},
                        onRetry = {},
                    ),
                )
            }
        }
        for (label in listOf("Pause", "Next track", "Previous track", "Shuffle", "Repeat", "Volume", "Position in track")) {
            compose.onNodeWithContentDescription(label).assertIsDisplayed()
        }
        compose.onNodeWithText("Track 1").assertIsDisplayed()
        compose.onNodeWithContentDescription("Pause").performClick()
        assertEquals(1, toggled)
    }
}
