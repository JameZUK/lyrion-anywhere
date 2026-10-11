package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.unit.dp
import androidx.compose.foundation.layout.width
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.unit.Density
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.ui.menus.ItemSheetContent
import io.github.jamezuk.lyrionanywhere.ui.menus.SheetTarget
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayingOnStrip
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsUiState
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * The largest font size Android offers (200%) on a phone: nothing is squeezed to nothing,
 * and text that can't fit ends in an ellipsis rather than being cut off ([assertTextFits]).
 */
@RunWith(AndroidJUnit4::class)
// Real text measurement (Robolectric's legacy graphics don't measure text faithfully).
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class FontScaleTests {
    @get:Rule
    val compose = createComposeRule()

    private fun large(content: @Composable () -> Unit) {
        compose.setContent {
            val d = LocalDensity.current
            CompositionLocalProvider(LocalDensity provides Density(d.density, fontScale = 2f)) {
                LyrionTheme(dynamicColour = false) { content() }
            }
        }
        compose.assertTextFits()
    }

    /** The check itself catches text cut off without an ellipsis, and accepts an ellipsis. */
    @Test
    fun theCheckCatchesCutText() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                androidx.compose.foundation.layout.Column {
                    androidx.compose.material3.Text(
                        "A long line that cannot fit in forty dp",
                        maxLines = 1,
                        softWrap = false,
                        modifier = androidx.compose.ui.Modifier.width(40.dp),
                    )
                    androidx.compose.material3.Text(
                        "Another long line that is ellipsised instead",
                        maxLines = 1,
                        overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                        modifier = androidx.compose.ui.Modifier.width(40.dp),
                    )
                }
            }
        }
        val failure = runCatching { compose.assertTextFits() }.exceptionOrNull()
        assertTrue("$failure", failure is AssertionError && "cannot fit" in failure.message.orEmpty())
        assertTrue("$failure", "ellipsised" !in failure?.message.orEmpty())
    }

    @Test
    fun nowPlaying() = large { NowPlayingScreen(auditPlayerState(), clock = { 0L }, actions = nowPlayingActions()) }

    @Test
    fun miniPlayer() = large {
        MiniPlayer(auditPlayerState(), onOpen = {}, onTogglePlay = {}, onNext = {}, onBringToPhone = {})
    }

    /** The "Playing on" strip keeps the player's name in view, with its buttons below. */
    @Test
    fun playingOnKeepsTheName() {
        large { PlayingOnStrip(auditPlayerState(), onBackToPhone = {}, onBringToPhone = {}) }
        val name = compose.onNodeWithTag("playing-on", useUnmergedTree = true)
        name.assertIsDisplayed()
        val node = name.fetchSemanticsNode()
        val widthDp = node.size.width / node.layoutInfo.density.density
        assertTrue("the name is only $widthDp dp wide", widthDp >= 200f)
        compose.onNodeWithText("Back to this phone").assertIsDisplayed()
        compose.onNodeWithText("Bring to this phone").assertIsDisplayed()
    }

    @Test
    fun moreSheet() {
        val navigator = auditNavigator()
        large { ItemSheetContent(SheetTarget("Track 1", "Artist"), navigator, onPlay = {}, onClose = {}, onDownload = {}) }
        compose.onNodeWithText("Play now").assertIsDisplayed()
        compose.onNodeWithText("Add").assertIsDisplayed()
    }

    @Test
    fun settings() = large {
        SettingsScreen(
            SettingsUiState(
                serverName = "music.example.com",
                username = "alice",
                sound = SoundSettings(),
                downloadsOffered = true,
                deviceName = "Test phone",
            ),
            appVersion = "1",
            libraryVersion = "1",
            onSignOut = {},
            onLicences = {},
        )
    }
}
