package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** Now playing on a small phone (360 dp wide). */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w360dp-h1800dp")
class NarrowPhoneTests {
    @get:Rule
    val compose = createComposeRule()

    /** With every action offered, the bottom row keeps four and the volume's popover a usable slider. */
    @Test
    fun theVolumeSliderKeepsAUsableWidth() {
        val state = auditPlayerState().copy(
            lyrics = TrackLyrics(false, true, listOf(LyricLine(0u, "A line"))),
            lyricsTrackId = 1,
        )
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(
                    state,
                    clock = { 0L },
                    actions = nowPlayingActions().copy(onAlarms = {}, onMore = {}),
                )
            }
        }
        // The actions are all still there, in their own row; the rest is in ⋮.
        compose.onNodeWithTag("now-playing-actions").assertExists()
        compose.onNodeWithTag("play-on-button").assertExists()
        compose.onNodeWithTag("lyrics-action").assertExists()
        compose.onNodeWithTag("queue-button").assertExists()
        compose.onNodeWithTag("now-playing-more").performClick()
        compose.onNodeWithTag("sleep-button").assertExists()
        compose.onNodeWithTag("volume-button").performClick()
        val node = compose.onNodeWithTag("volume-slider").fetchSemanticsNode()
        val lengthDp = node.size.width / node.layoutInfo.density.density
        assertTrue("the volume slider is only $lengthDp dp long", lengthDp >= 160f)
    }
}
