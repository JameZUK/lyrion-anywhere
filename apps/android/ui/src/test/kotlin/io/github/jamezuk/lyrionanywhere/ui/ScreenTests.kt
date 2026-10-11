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
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.isToggleable
import androidx.compose.ui.test.performScrollTo
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.ui.player.SignalPathPanel
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
        // Not this phone: no signal path.
        compose.onNodeWithText("Signal path").assertDoesNotExist()
    }

    private fun panel(path: io.github.jamezuk.lyrionanywhere.core.SignalPathInfo?) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) { SignalPathPanel(path) }
        }
    }

    @Test
    fun signalPathShowsSourceStagesOutputAndAnHonestVerdict() {
        panel(signalPath())
        compose.onNodeWithText("Bit-perfect").assertIsDisplayed()
        compose.onNodeWithText("FLAC · 96 kHz · 24-bit").assertIsDisplayed()
        compose.onNodeWithText("Headphones · 96 kHz · 32-bit float").assertIsDisplayed()
        compose.onNodeWithContentDescription("Decode (lossless), bit-transparent").assertIsDisplayed()
        // Not claimed past Android's mixer.
        compose.onNodeWithText("Android may still process", substring = true).assertIsDisplayed()
    }

    @Test
    fun signalPathFlagsResamplingAndProcessing() {
        panel(
            signalPath(
                verdict = SignalVerdict.RESAMPLED,
                engineBitPerfect = false,
                stages = listOf(
                    SignalStage("Decode (lossless)", true, false),
                    SignalStage("Volume −6.0 dB", false, false),
                    SignalStage("Resample 96→48 kHz (system)", false, true),
                ),
            ),
        )
        compose.onNodeWithText("Resampled").assertIsDisplayed()
        compose.onNodeWithContentDescription("Volume −6.0 dB, changes samples").assertIsDisplayed()
        compose.onNodeWithContentDescription("Resample 96→48 kHz (system), changes the sample rate").assertIsDisplayed()
        compose.onNodeWithText("The sample rate is converted", substring = true).assertIsDisplayed()
    }

    @Test
    fun signalPathEndToEndOnlyWhenDirect() {
        panel(signalPath(bitPerfect = true))
        compose.onNodeWithText("Every sample reaches the device unchanged.").assertIsDisplayed()
    }

    @Test
    fun signalPathWithNothingPlaying() {
        panel(null)
        compose.onNodeWithText("Nothing playing on this phone yet.").assertIsDisplayed()
    }

    @Test
    fun nowPlayingOnThisPhoneShowsTheSignalPath() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(
                    state = PlayerUiState(
                        players = listOf(samplePlayer("02:4c:41:00:00:01")),
                        selectedId = "02:4c:41:00:00:01",
                        status = sampleStatus(),
                        loading = false,
                        local = LocalPlayerState(
                            enabled = true,
                            status = localStatus("02:4c:41:00:00:01"),
                            signalPath = signalPath(),
                        ),
                    ),
                    clock = { 0L },
                    actions = NowPlayingActions({}, {}, {}, {}, {}, {}, {}, {}, {}, {}),
                )
            }
        }
        compose.onNodeWithText("Signal path").performScrollTo().assertIsDisplayed()
    }
}
