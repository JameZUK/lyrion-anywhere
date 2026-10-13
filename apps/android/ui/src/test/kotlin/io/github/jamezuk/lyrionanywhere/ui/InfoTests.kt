package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AlbumInfo
import io.github.jamezuk.lyrionanywhere.core.ArtistInfo
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumActions
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumDetail
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistActions
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistDetail
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistScreen
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ExpandableInfo
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PHOTO = "imageproxy/https%3A%2F%2Fimages%2Eexample%2Ecom%2Fa%2Ejpg/image"
private val LONG = "The Invented Band formed in 1990. " + "They played many invented concerts in many invented towns. ".repeat(12) +
    "\n\nSecond paragraph, only seen once expanded."
private const val HOSTILE = "<img src=x onerror=alert(1)><script>alert(2)</script>"

/** Biographies, photos and reviews (X5): loaded lazily, quietly absent on failure. */
@OptIn(ExperimentalCoroutinesApi::class)
class InfoViewModelTest : MainDispatcherTest() {
    @Test
    fun theArtistPageLoadsItsInformationAfterItsAlbums() = runTest(dispatcher) {
        val library = FakeLibrary().apply { artistInfo = ArtistInfo("A biography.", PHOTO, "Discogs") }
        val vm = ArtistViewModel(library, 3)
        assertTrue(vm.state.value is Loadable.Ready)
        assertEquals(ArtistInfo("A biography.", PHOTO, "Discogs"), vm.info.value)
        assertEquals(listOf("artist 3"), library.infoCalls)
        // Retrying the albums doesn't ask again.
        vm.load()
        assertEquals(1, library.infoCalls.size)
    }

    @Test
    fun aFailureIsNoInformationAndTheAlbumsStillShow() = runTest(dispatcher) {
        val library = FakeLibrary().apply { infoFail = AppError.Server(503, "the server didn't answer") }
        val artist = ArtistViewModel(library, 3)
        assertTrue(artist.state.value is Loadable.Ready)
        assertNull(artist.info.value)
        val album = AlbumViewModel(library, 7)
        assertTrue(album.state.value is Loadable.Ready)
        assertNull(album.review.value)
        library.infoFail = AppError.RateLimited(5)
        assertNull(AlbumViewModel(library, 8).review.value)
    }

    @Test
    fun theAlbumPageLoadsItsReview() = runTest(dispatcher) {
        val library = FakeLibrary().apply { albumInfo = AlbumInfo("A fine record.") }
        val vm = AlbumViewModel(library, 7)
        assertEquals("A fine record.", vm.review.value)
        assertEquals(listOf("album 7"), library.infoCalls)
        library.albumInfo = AlbumInfo("   ")
        assertNull(AlbumViewModel(library, 8).review.value)
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class InfoScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) =
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    private fun state(s: String) = SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, s)

    @Test
    fun longTextExpandsAndSaysSo() {
        show { ExpandableInfo("Biography", LONG) }
        compose.onNodeWithText("Biography").assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading))
        compose.onNodeWithText("Read more").assertIsDisplayed().assert(state("Collapsed"))
        compose.onNodeWithText("Read more").performClick()
        compose.onNodeWithText("Show less").assertIsDisplayed().assert(state("Expanded"))
        compose.onNodeWithText("Show less").performClick()
        compose.onNodeWithText("Read more").assert(state("Collapsed"))
    }

    @Test
    fun shortTextHasNoButtonAndMarkupIsJustText() {
        show { ExpandableInfo("Review", HOSTILE) }
        compose.onNodeWithText(HOSTILE).assertIsDisplayed()
        assertTrue(compose.onAllNodesWithText("Read more").fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun theArtistPageShowsTheBiographyAndPhoto() {
        show {
            ArtistScreen(
                3,
                "The Invented Band",
                Loadable.Ready(ArtistDetail.group(listOf(sampleAlbum(1)), listOf(sampleAlbum(1)))),
                ArtistActions(onRetry = {}, onAlbum = {}, onPlay = {}, onShuffle = {}, onMore = {}),
                info = ArtistInfo(LONG, PHOTO, "Discogs"),
            )
        }
        compose.onNodeWithText("Biography").performScrollTo().assertIsDisplayed()
        compose.onNodeWithText("Read more").performScrollTo().assertIsDisplayed()
        compose.onNodeWithTag("info-Biography").assertIsDisplayed()
    }

    @Test
    fun theArtistPageWithoutInformationShowsNoSection() {
        show {
            ArtistScreen(
                3,
                "The Invented Band",
                Loadable.Ready(ArtistDetail.group(listOf(sampleAlbum(1)), listOf(sampleAlbum(1)))),
                ArtistActions(onRetry = {}, onAlbum = {}, onPlay = {}, onShuffle = {}, onMore = {}),
                info = ArtistInfo(null, null, null),
            )
        }
        assertTrue(compose.onAllNodesWithText("Biography").fetchSemanticsNodes().isEmpty())
    }

    @Test
    fun theAlbumPageShowsTheReview() {
        show {
            AlbumScreen(
                Loadable.Ready(AlbumDetail(sampleAlbum(7), listOf(sampleTrack(1)))),
                albumActions(),
                review = "A fine record.",
            )
        }
        compose.onNodeWithText("Review").performScrollTo().assertIsDisplayed()
            .assert(SemanticsMatcher.keyIsDefined(SemanticsProperties.Heading))
        compose.onNodeWithText("A fine record.").assertIsDisplayed()
    }
}

private fun albumActions() = AlbumActions(
    onRetry = {},
    onPlay = { _, _ -> },
    onShuffle = {},
    onPlayTrack = {},
    onArtist = { _, _ -> },
    onMore = {},
)

/** The new sections as TalkBack sees them. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class InfoAccessibilityAuditTest {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    @Test
    fun artistWithBiographyAndPhoto() = audit {
        ArtistScreen(
            3,
            "The Invented Band",
            Loadable.Ready(ArtistDetail.group(listOf(sampleAlbum(1)), listOf(sampleAlbum(1)))),
            ArtistActions(onRetry = {}, onAlbum = {}, onPlay = {}, onShuffle = {}, onMore = {}),
            info = ArtistInfo(LONG, PHOTO, "Discogs"),
        )
    }

    @Test
    fun albumWithReview() = audit {
        AlbumScreen(Loadable.Ready(AlbumDetail(sampleAlbum(7), listOf(sampleTrack(1)))), albumActions(), review = LONG)
    }

    @Test
    fun anExpandedReview() {
        compose.setContent { LyrionTheme(dynamicColour = false) { ExpandableInfo("Review", LONG) } }
        compose.onNodeWithText("Read more").performClick()
        compose.assertAccessible()
    }
}
