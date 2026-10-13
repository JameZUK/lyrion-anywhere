package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.core.IndexEntry
import io.github.jamezuk.lyrionanywhere.core.RoleCount
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.SearchKind
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.TopResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LibraryAlbumOrder
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumBrowser
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistActions
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistBrowser
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistDetail
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistScreen
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistView
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.PagedState
import io.github.jamezuk.lyrionanywhere.ui.library.SearchActions
import io.github.jamezuk.lyrionanywhere.ui.library.SearchHit
import io.github.jamezuk.lyrionanywhere.ui.library.SearchScreen
import io.github.jamezuk.lyrionanywhere.ui.library.SearchType
import io.github.jamezuk.lyrionanywhere.ui.library.SearchUiState
import io.github.jamezuk.lyrionanywhere.ui.library.SparseState
import io.github.jamezuk.lyrionanywhere.ui.menus.ItemSheetContent
import io.github.jamezuk.lyrionanywhere.ui.menus.SheetTarget
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.ShuffleButton
import io.github.jamezuk.lyrionanywhere.ui.player.UpNext
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The Wave 2 part 2 screens, audited as TalkBack sees them ([assertAccessible]): Library
 * chips, album and artist browsers with the A–Z scroller, search (recents, grouped, one
 * type), credits, "Appears as", the queue with sections and gapless marks, the queue peek
 * and the shuffle button.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class Wave2AccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    private val index = listOf(IndexEntry("A", 0u, 2u), IndexEntry("B", 2u, 1u), IndexEntry("C", 3u, 2u))

    private fun albums() = SparseState(total = 5, pages = mapOf(0 to (1L..5L).map { sampleAlbum(it) }), index = index)

    @Test
    fun libraryOverviewWithChips() = audit {
        LibraryScreen(emptyList(), null, {}, onNewMusic = {}, onOpen = {}, chip = null, onChip = {})
    }

    @Test
    fun albumsAsAGrid() = audit {
        LibraryScreen(emptyList(), null, {}, onNewMusic = {}, onOpen = {}, chip = LibraryChip.ALBUMS, onChip = {}, chipContent = {
            AlbumBrowser(albums(), LibraryAlbumOrder.TITLE, true, {}, {}, {}, {}, {}, {}, onQueue = { _, _ -> })
        })
    }

    @Test
    fun albumsAsAList() = audit {
        AlbumBrowser(albums(), LibraryAlbumOrder.TITLE, false, {}, {}, {}, {}, {}, {}, onQueue = { _, _ -> })
    }

    @Test
    fun albumsStillLoading() = audit {
        AlbumBrowser(SparseState(total = 300, index = index), LibraryAlbumOrder.TITLE, false, {}, {}, {}, {}, {}, {})
    }

    @Test
    fun albumsRecentlyPlayedAsAGrid() = audit {
        AlbumBrowser(albums(), LibraryAlbumOrder.RECENTLY_PLAYED, true, {}, {}, {}, {}, {}, {}, onQueue = { _, _ -> })
    }

    @Test
    fun albumsRecentlyPlayedAsAList() = audit {
        AlbumBrowser(albums(), LibraryAlbumOrder.RECENTLY_PLAYED, false, {}, {}, {}, {}, {}, {}, onQueue = { _, _ -> })
    }

    @Test
    fun albumsNothingPlayedYet() = audit {
        AlbumBrowser(SparseState(total = 0), LibraryAlbumOrder.RECENTLY_PLAYED, true, {}, {}, {}, {}, {}, {})
    }

    @Test
    fun albumsRecentlyPlayedFailed() = audit {
        AlbumBrowser(SparseState(error = AppError.Offline()), LibraryAlbumOrder.RECENTLY_PLAYED, true, {}, {}, {}, {}, {}, {})
    }

    @Test
    fun albumSortMenuOpen() {
        audit { AlbumBrowser(albums(), LibraryAlbumOrder.RECENTLY_PLAYED, true, {}, {}, {}, {}, {}, {}) }
        compose.onNodeWithTag("album-sort").performClick()
        compose.waitForIdle()
        compose.assertAccessible()
    }

    @Test
    fun artists() = audit {
        ArtistBrowser(
            SparseState(total = 2, pages = mapOf(0 to listOf(Artist(1, "Example One"), Artist(2, "Example Two"))), index = index),
            views = listOf(ArtistView.ALBUM_ARTISTS, ArtistView.ALL, ArtistView.COMPOSERS),
            view = ArtistView.ALBUM_ARTISTS,
            onView = {}, onEnsure = {}, onRetry = {}, onArtist = {}, onMore = {},
        )
    }

    private val noSearch = SearchActions({}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, { _, _ -> }, {})

    @Test
    fun searchRecents() = audit { SearchScreen(SearchUiState(recents = listOf("blue", "red")), PagedState(), noSearch) }

    @Test
    fun searchGrouped() = audit {
        SearchScreen(
            SearchUiState(
                query = "blue",
                results = SearchResults(
                    listOf(Artist(3, "Blue Band")), listOf(sampleAlbum(7)), listOf(sampleTrack(9)),
                    1u, 9u, 30u, TopResult(SearchKind.ARTIST, 0u),
                ),
                playlists = listOf(SavedPlaylist(5, "Road trip")),
            ),
            PagedState(),
            noSearch,
        )
    }

    @Test
    fun searchOneType() = audit {
        SearchScreen(
            SearchUiState(query = "blue", results = SearchResults(emptyList(), emptyList(), emptyList()), type = SearchType.ALBUMS),
            PagedState(listOf(SearchHit.OfAlbum(sampleAlbum(1)), SearchHit.OfAlbum(sampleAlbum(2))), total = 2),
            noSearch,
        )
    }

    @Test
    fun trackCreditsInTheMoreSheet() = audit {
        ItemSheetContent(
            target = SheetTarget("Track 1", canPlay = true),
            navigator = auditNavigator(),
            onPlay = {},
            onClose = {},
            credits = listOf(
                Credit(ArtistRole.ARTIST, "Singer One", 21),
                Credit(ArtistRole.COMPOSER, "Some Composer", 41),
                Credit(ArtistRole.COMPOSER, "Unlinked Name", null),
            ),
        )
    }

    @Test
    fun artistWithAppearsAs() = audit {
        ArtistScreen(
            3,
            "Example Artist",
            Loadable.Ready(
                ArtistDetail.group(listOf(sampleAlbum(1)), listOf(sampleAlbum(1)))
                    .copy(roles = listOf(RoleCount(ArtistRole.COMPOSER, 12u), RoleCount(ArtistRole.CONDUCTOR, 3u))),
            ),
            ArtistActions(onRetry = {}, onAlbum = {}, onPlay = {}, onShuffle = {}, onMore = {}),
        )
    }

    @Test
    fun nowPlayingWithQueueSections() = audit {
        NowPlayingScreen(
            auditPlayerState().copy(upNext = UpNext(0, listOf(2L))),
            clock = { 0L },
            actions = nowPlayingActions().copy(onMore = {}),
        )
    }

    @Test
    fun theQueuePeek() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                NowPlayingScreen(auditPlayerState().copy(upNext = UpNext(0, listOf(2L))), clock = { 0L }, actions = nowPlayingActions())
            }
        }
        compose.onNodeWithTag("queue-button").performClick()
        compose.assertAccessible()
    }

    @Test
    fun shuffleButtonInEachMode() = audit {
        androidx.compose.foundation.layout.Row {
            for (mode in ShuffleMode.entries) ShuffleButton(mode, onCycle = {}, onChoose = {})
        }
    }
}
