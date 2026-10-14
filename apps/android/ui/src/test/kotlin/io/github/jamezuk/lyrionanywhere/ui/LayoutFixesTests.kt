package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.unit.dp
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionFrame
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionMessage
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionTags
import io.github.jamezuk.lyrionanywhere.ui.docs.DocArt
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryChips
import io.github.jamezuk.lyrionanywhere.ui.library.PlayButtons
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMark
import io.github.jamezuk.lyrionanywhere.ui.player.QueueHeader
import io.github.jamezuk.lyrionanywhere.ui.theme.AmbientBackground
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlin.math.abs
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/**
 * Four layout faults the rendered UI review found: the queue heading wrapping a letter per
 * line, the ambient cover painting over the connection line, the album header's ⋮ wrapping
 * onto its own line, and the chosen Library chip left off the edge.
 */
@RunWith(AndroidJUnit4::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(sdk = [35], qualifiers = "w360dp-h800dp-xhdpi")
class LayoutFixesTests {
    @get:Rule
    val compose = createComposeRule()

    @Before
    fun art() {
        // Generated covers, so the ambient background has something to draw.
        DocArt.install(ApplicationProvider.getApplicationContext())
    }

    private fun SemanticsNodeInteraction.lines(): Int {
        val results = mutableListOf<TextLayoutResult>()
        fetchSemanticsNode().config[SemanticsActions.GetTextLayoutResult].action?.invoke(results)
        return results.single().lineCount
    }

    private fun queueHeader(width: Int) {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                Box(Modifier.width(width.dp)) {
                    QueueHeader(5, "Kitchen", onClear = {}, upcoming = 3, onClearUpcoming = {}, onSaveAsPlaylist = {})
                }
            }
        }
    }

    /** Each of the queue's actions is a button in the row, or an item in its ⋮ menu. */
    private fun assertQueueActionsReachable() {
        val tags = listOf("save-queue", "clear-upcoming", "clear-queue")
        val inline = tags.filter { compose.onAllNodes(androidx.compose.ui.test.hasTestTag(it)).fetchSemanticsNodes().isNotEmpty() }
        if (inline.size < tags.size) {
            compose.onNodeWithTag("queue-actions").assertIsDisplayed().performClick()
            (tags - inline.toSet()).forEach { compose.onNodeWithTag(it).assertIsDisplayed() }
        }
    }

    // B1: the queue heading.

    @Test
    fun theQueueHeadingStaysOnOneLineOnAPhone() {
        queueHeader(360)
        assertEquals(1, compose.onNodeWithTag("queue-title", useUnmergedTree = true).lines())
        compose.onNodeWithText("Queue").assertIsDisplayed()
        assertQueueActionsReachable()
    }

    @Test
    fun theQueueHeadingStaysOnOneLineInANarrowPane() {
        queueHeader(240)
        assertEquals(1, compose.onNodeWithTag("queue-title", useUnmergedTree = true).lines())
        assertQueueActionsReachable()
    }

    @Test
    fun aQueueActionInTheMenuStillAsksFirst() {
        var cleared = 0
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                Box(Modifier.width(200.dp)) {
                    QueueHeader(5, "Kitchen", onClear = { cleared++ }, upcoming = 3, onSaveAsPlaylist = {})
                }
            }
        }
        compose.onNodeWithTag("queue-actions").performClick()
        compose.onNodeWithText("Clear queue").performClick()
        compose.onNodeWithText("Clear the queue?").assertIsDisplayed()
        assertEquals(0, cleared)
    }

    @Test
    @Config(sdk = [35], qualifiers = "w1280dp-h800dp-xhdpi")
    fun aWideQueueHeaderNeedsNoMenu() {
        queueHeader(800)
        listOf("save-queue", "clear-upcoming", "clear-queue").forEach { compose.onNodeWithTag(it).assertIsDisplayed() }
        compose.onNodeWithTag("queue-actions").assertDoesNotExist()
    }

    // B2: the connection line over Now playing's ambient background.

    @Test
    fun theAmbientCoverDoesNotPaintOverTheConnectionLine() {
        var container = Color.Unspecified
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                container = MaterialTheme.colorScheme.tertiaryContainer
                ConnectionFrame(ConnectionMessage.UNREACHABLE, onDownloads = {}) { AmbientBackground("1001") }
            }
        }
        compose.waitForIdle()
        val image = compose.onNodeWithTag(ConnectionTags.BANNER).captureToImage().asAndroidBitmap()
        val want = container.toArgb()
        var plain = 0
        var all = 0
        for (y in 0 until image.height step 3) {
            for (x in 0 until image.width step 3) {
                all++
                if (close(image.getPixel(x, y), want)) plain++
            }
        }
        // The line's own colour, bar its text and icons: the cover must not show through.
        assertTrue("only $plain of $all pixels are the line's colour", plain > all * 0.7)
    }

    @Test
    fun theAmbientBackgroundStaysInsideItsBounds() {
        val mark = Color(0xFF00FF00)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                Column(Modifier.size(300.dp)) {
                    Box(Modifier.fillMaxWidth().height(100.dp).background(mark).testTag("above"))
                    AmbientBackground("1001", Modifier.height(200.dp))
                }
            }
        }
        compose.waitForIdle()
        val image = compose.onNodeWithTag("above").captureToImage().asAndroidBitmap()
        for (y in 0 until image.height step 5) {
            for (x in 0 until image.width step 5) {
                assertTrue("the cover bled out at ($x, $y)", close(image.getPixel(x, y), mark.toArgb()))
            }
        }
    }

    // B4: the album and playlist header.

    private var width by mutableStateOf(0)

    private fun playButtons(width: Int, more: () -> Unit = {}, download: () -> Unit = {}, add: () -> Unit = {}) {
        this.width = width
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                Box(Modifier.width(this.width.dp)) {
                    PlayButtons(
                        onPlay = { if (it == io.github.jamezuk.lyrionanywhere.core.Placement.ADD_TO_QUEUE) add() },
                        onShuffle = {},
                        onMore = more,
                        onDownload = download,
                        downloadMark = DownloadMark.SOME,
                    )
                }
            }
        }
    }

    private fun centreY(n: SemanticsNodeInteraction): Float {
        val node = n.fetchSemanticsNode()
        return node.positionInRoot.y + node.size.height / 2f
    }

    @Test
    fun theHeaderMenuStaysOnThePlayRow() {
        // A 411 dp phone, less the header's padding.
        playButtons(379)
        listOf(379, 328, 300).forEach { width ->
            this.width = width
            compose.waitForIdle()
            val play = compose.onNodeWithText("Play").assertIsDisplayed()
            val shuffle = compose.onNodeWithText("Shuffle").assertIsDisplayed()
            val more = compose.onNodeWithContentDescription("More options").assertIsDisplayed()
            val y = centreY(play)
            assertTrue("Shuffle wrapped at $width dp", abs(centreY(shuffle) - y) < 2f)
            assertTrue("⋮ wrapped at $width dp", abs(centreY(more) - y) < 2f)
        }
    }

    @Test
    fun aMovedHeaderActionIsInTheMenuWithItsLabel() {
        var added = 0
        var downloads = 0
        var more = 0
        playButtons(300, more = { more++ }, download = { downloads++ }, add = { added++ })
        compose.onNodeWithContentDescription("Add to queue").assertDoesNotExist()
        compose.onNodeWithContentDescription("More options").performClick()
        compose.onNodeWithText("Add to queue").performClick()
        compose.onNodeWithContentDescription("More options").performClick()
        compose.onNodeWithText("Download the rest").performClick()
        compose.onNodeWithContentDescription("More options").performClick()
        compose.onNodeWithText("More…").performClick()
        assertEquals(listOf(1, 1, 1), listOf(added, downloads, more))
    }

    @Test
    @Config(sdk = [35], qualifiers = "w1280dp-h800dp-xhdpi")
    fun aWideHeaderShowsEveryButtonAndTheMenuOpensMore() {
        var more = 0
        playButtons(600, more = { more++ })
        compose.onNodeWithContentDescription("Add to queue").assertIsDisplayed()
        compose.onNodeWithContentDescription("Download the rest").assertIsDisplayed()
        compose.onNodeWithContentDescription("More options").performClick()
        assertEquals(1, more)
    }

    // B5: the chosen Library chip.

    private fun assertWhollyOnScreen(tag: String) {
        val root = compose.onRoot().fetchSemanticsNode().size.width
        val node = compose.onNodeWithTag(tag).assertIsDisplayed().fetchSemanticsNode()
        val left = node.positionInRoot.x
        assertTrue("$tag starts off-screen at $left", left >= 0f)
        assertTrue("$tag ends off-screen (${left + node.size.width} of $root)", left + node.size.width <= root)
    }

    @Test
    fun theChosenChipIsScrolledIntoView() {
        compose.setContent { LyrionTheme(dynamicColour = false) { LibraryChips(LibraryChip.DOWNLOADED, {}) } }
        compose.waitForIdle()
        assertWhollyOnScreen("chip-downloaded")
    }

    @Test
    fun choosingAnotherChipBringsItIntoView() {
        var chosen by mutableStateOf<LibraryChip?>(LibraryChip.ALBUMS)
        compose.setContent { LyrionTheme(dynamicColour = false) { LibraryChips(chosen, { chosen = it }) } }
        compose.waitForIdle()
        assertWhollyOnScreen("chip-albums")
        chosen = LibraryChip.DOWNLOADED
        compose.waitForIdle()
        assertWhollyOnScreen("chip-downloaded")
        chosen = LibraryChip.ALBUMS
        compose.waitForIdle()
        assertWhollyOnScreen("chip-albums")
    }

    private fun close(a: Int, b: Int): Boolean {
        fun ch(c: Int, s: Int) = (c shr s) and 0xFF
        return listOf(0, 8, 16).all { abs(ch(a, it) - ch(b, it)) <= 12 }
    }
}
