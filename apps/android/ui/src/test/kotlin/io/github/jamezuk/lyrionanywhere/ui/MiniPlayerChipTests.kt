package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertContentDescriptionEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.text.TextLayoutResult
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.DiagramVerdict
import io.github.jamezuk.lyrionanywhere.core.QualityDiagram
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.ui.player.Amber
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.TEXT_SHARE
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

private const val PHONE = "02:4c:41:00:00:01"

/**
 * The mini player's quality chip (the fault: on a 411 dp phone "Hi-Res · FLAC 24/96" took
 * the line and cut the artist to "T…"). The chip is the verdict word alone, after the
 * artist, and the artist keeps its room (at 411 dp the line is about 147 dp).
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w411dp-h891dp-xxhdpi")
class MiniPlayerChipTest {
    @get:Rule
    val compose = createComposeRule()

    private fun state(diagram: QualityDiagram, artist: String) = PlayerUiState(
        players = listOf(samplePlayer(PHONE)),
        selectedId = PHONE,
        status = sampleStatus().let { s -> s.copy(current = s.current!!.copy(title = "The Lantern Song", artist = artist)) },
        loading = false,
        local = LocalPlayerState(enabled = true, status = localStatus(PHONE)),
        diagram = diagram,
    )

    private fun show(diagram: QualityDiagram, artist: String) = compose.setContent {
        LyrionTheme(dynamicColour = false) {
            MiniPlayer(state(diagram, artist), onOpen = {}, onTogglePlay = {}, onNext = {})
        }
    }

    private fun SemanticsNodeInteraction.layout(): TextLayoutResult {
        val results = mutableListOf<TextLayoutResult>()
        fetchSemanticsNode().config[SemanticsActions.GetTextLayoutResult].action?.invoke(results)
        return results.single()
    }

    private val hiRes get() = QualityVectors.expected(QualityVectors.PASSED_THROUGH)
    private val transcoded get() = QualityVectors.expected(QualityVectors.REPORTED)

    private fun width(tag: String) = compose.onNodeWithTag(tag, useUnmergedTree = true).fetchSemanticsNode().size.width

    private fun dp(px: Int) = px / compose.density.density

    /** The reported case: the artist keeps most of the line, not one letter, beside a chip reading "Hi-Res". */
    @Test
    fun aHiResBadgeIsTheWordAloneAndTheArtistKeepsItsRoom() {
        assertTrue(hiRes.badge!!.verdict == DiagramVerdict.HI_RES)
        show(hiRes, "The Paper Lanterns")
        compose.onNodeWithTag("mini-quality", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("mini-quality").assertTextEquals("Hi-Res").assert(SemanticsMatcher.expectValue(Amber, false))
        // TalkBack still hears the full badge.
        compose.onNodeWithTag("mini-quality").assertContentDescriptionEquals(hiRes.badge!!.text)
        compose.onNodeWithText("Hi-Res · FLAC 24/96").assertDoesNotExist()
        val artist = compose.onNodeWithText("The Paper Lanterns", useUnmergedTree = true).assertIsDisplayed().fetchSemanticsNode()
        val line = width("mini-player-line")
        assertTrue("the artist has only ${dp(artist.size.width)} dp", dp(artist.size.width) >= 85f)
        assertTrue("the chip takes more room than the artist", artist.size.width > 1.5f * width("mini-quality"))
        assertTrue("the artist has less than 60% of the line", artist.size.width >= 0.6f * line)
    }

    /** An artist that fits beside the chip is never ellipsised. */
    @Test
    fun anArtistThatFitsIsShownInFull() {
        show(hiRes, "Paper Lanterns")
        compose.onNodeWithTag("mini-quality").assertTextEquals("Hi-Res")
        val artist = compose.onNodeWithText("Paper Lanterns", useUnmergedTree = true).assertIsDisplayed().layout()
        assertFalse("the artist is cut short", artist.isLineEllipsized(0))
    }

    @Test
    fun aTranscodedBadgeIsTheWordAloneInAmber() {
        show(transcoded, "The Paper Lanterns")
        compose.onNodeWithTag("mini-quality", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("mini-quality").assertTextEquals("Transcoded").assert(SemanticsMatcher.expectValue(Amber, true))
        compose.onNodeWithText("Transcoded · Opus 160").assertDoesNotExist()
        val artist = compose.onNodeWithText("The Paper Lanterns", useUnmergedTree = true).fetchSemanticsNode()
        assertTrue("the artist has only ${dp(artist.size.width)} dp", artist.size.width >= TEXT_SHARE * width("mini-player-line"))
    }

    /** An artist too long for the line is ellipsised, but keeps its share of the line beside the chip. */
    @Test
    fun aLongArtistIsEllipsisedOnlyWhenTheLineRunsOut() {
        val long = "The Paper Lanterns and the Very Long Name Orchestra of the Northern Lights"
        show(hiRes, long)
        compose.onNodeWithTag("mini-quality", useUnmergedTree = true).assertIsDisplayed()
        compose.onNodeWithTag("mini-quality").assertTextEquals("Hi-Res")
        val node = compose.onNodeWithText(long, useUnmergedTree = true)
        assertTrue("the line overflows, so the artist is ellipsised", node.layout().isLineEllipsized(0))
        val artist = node.fetchSemanticsNode().size.width
        assertTrue("the artist has only ${dp(artist)} dp", dp(artist) >= 85f)
        assertTrue("the artist is narrower than the chip", artist > 1.5f * width("mini-quality"))
    }

    /** Lossless and Lossy read as their words alone too. */
    @Test
    fun everyVerdictIsItsWordAlone() {
        val base = hiRes
        val words = mapOf(
            DiagramVerdict.HI_RES to "Hi-Res",
            DiagramVerdict.LOSSLESS to "Lossless",
            DiagramVerdict.TRANSCODED to "Transcoded",
            DiagramVerdict.LOSSY to "Lossy",
        )
        var verdict by mutableStateOf(DiagramVerdict.HI_RES)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                val lowered = verdict == DiagramVerdict.TRANSCODED || verdict == DiagramVerdict.LOSSY
                val d = base.copy(badge = base.badge!!.copy(verdict = verdict, lowered = lowered))
                MiniPlayer(state(d, "Lanterns"), onOpen = {}, onTogglePlay = {}, onNext = {})
            }
        }
        for ((v, word) in words) {
            verdict = v
            compose.waitForIdle()
            val lowered = v == DiagramVerdict.TRANSCODED || v == DiagramVerdict.LOSSY
            compose.onNodeWithTag("mini-quality").assertTextEquals(word).assert(SemanticsMatcher.expectValue(Amber, lowered))
        }
    }
}
