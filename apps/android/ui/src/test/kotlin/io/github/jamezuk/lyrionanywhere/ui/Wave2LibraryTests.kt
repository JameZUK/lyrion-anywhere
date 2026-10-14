package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeLeft
import androidx.compose.ui.test.swipeRight
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasTestTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.core.IndexEntry
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.RoleCount
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.SearchKind
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.TopResult
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.LibraryAlbumOrder
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.data.LibraryPrefs
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumBrowser
import io.github.jamezuk.lyrionanywhere.ui.library.AppearsAs
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.AtoZ
import io.github.jamezuk.lyrionanywhere.ui.library.BrowseSwipe
import io.github.jamezuk.lyrionanywhere.ui.library.ContributorViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.CreditsSection
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryAlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryArtistsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryPrefsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.PagedState
import io.github.jamezuk.lyrionanywhere.ui.library.SearchActions
import io.github.jamezuk.lyrionanywhere.ui.library.SearchHit
import io.github.jamezuk.lyrionanywhere.ui.library.SearchScreen
import io.github.jamezuk.lyrionanywhere.ui.library.SearchType
import io.github.jamezuk.lyrionanywhere.ui.library.SearchUiState
import io.github.jamezuk.lyrionanywhere.ui.library.SearchViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.SparseState
import io.github.jamezuk.lyrionanywhere.ui.library.SparsePagedViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.TrackRow
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import kotlin.random.Random

private fun idx(vararg pairs: Pair<String, Int>): List<IndexEntry> {
    var o = 0
    return pairs.map { (k, n) -> IndexEntry(k, o.toUInt(), n.toUInt()).also { o += n } }
}

/** Wave 2 §3.1–3.3 view models: A–Z paging, remembered choices, search, "Appears as". */
@OptIn(ExperimentalCoroutinesApi::class)
class Wave2LibraryViewModelTest : MainDispatcherTest() {
    @Test
    fun aLetterFarDownReadsOnlyItsPage() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 1000).apply { index = idx("A" to 400, "M" to 600) }
        val vm = LibraryAlbumsViewModel(library, LibraryAlbumOrder.TITLE)
        assertEquals(1000, vm.state.value.total)
        assertEquals(listOf(0), library.albumCalls.map { it.first })
        assertEquals(2, vm.state.value.index.size)
        // Jump to "M": only the page at 400 is read.
        vm.ensure(AtoZ.target(vm.state.value.index[1], 1000))
        assertEquals(listOf(0, 400), library.albumCalls.map { it.first })
        assertEquals("Album 401", vm.state.value[400]!!.title)
        assertNull(vm.state.value[200])
        // Asking again doesn't read it again; past the end reads nothing.
        vm.ensure(450)
        vm.ensure(5000)
        vm.ensure(-1)
        assertEquals(2, library.albumCalls.size)
    }

    @Test
    fun theIndexIsOnlyForTitleOrderAndASortStartsOver() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 30).apply { index = idx("A" to 30) }
        val vm = LibraryAlbumsViewModel(library, LibraryAlbumOrder.NEWEST)
        assertTrue(vm.state.value.index.isEmpty())
        assertTrue(library.indexCalls.isEmpty())
        vm.setOrder(LibraryAlbumOrder.TITLE)
        assertEquals(AlbumSort.TITLE, library.albumCalls.last().third)
        assertEquals(1, vm.state.value.index.size)
        assertEquals(LibraryAlbumOrder.TITLE, vm.order.value)
    }

    @Test
    fun recentlyPlayedIsTheUsersOwnHistoryInOrderWithNoLmsSort() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 30).apply { index = idx("A" to 30) }
        val me = FakeMe().apply {
            recent = ApiResult.Ok(listOf(42, 7, 19, 3))
            // The server may answer in any order: the list keeps the history's.
            albumsById = { ids -> ApiResult.Ok(ids.sorted().map { sampleAlbum(it) }) }
        }
        val vm = LibraryAlbumsViewModel(library, LibraryAlbumOrder.RECENTLY_PLAYED, me = me)
        assertEquals(listOf(ShelfKind.RECENT to 50), me.shelfCalls)
        assertEquals(listOf(listOf(42L, 7L, 19L, 3L)), me.albumIdCalls)
        assertEquals(4, vm.state.value.total)
        assertEquals(listOf(42L, 7L, 19L, 3L), (0 until 4).map { vm.state.value[it]!!.id })
        // Never LMS's albums order (its "recently played" is everyone's), and no A–Z.
        assertTrue(library.albumCalls.isEmpty())
        assertTrue(library.indexCalls.isEmpty())
        assertTrue(vm.state.value.index.isEmpty())
        // Back to an LMS order: asked of LMS again.
        vm.setOrder(LibraryAlbumOrder.NEWEST)
        assertEquals(AlbumSort.NEWEST, library.albumCalls.single().third)
    }

    @Test
    fun recentlyPlayedIsCappedAtFiftyEachOnceAndSkipsAlbumsGone() = runTest(dispatcher) {
        val me = FakeMe().apply {
            recent = ApiResult.Ok((1L..80L).toList() + listOf(1L, 2L))
            // Album 5 is no longer in the library.
            albumsById = { ids -> ApiResult.Ok(ids.filter { it != 5L }.map { sampleAlbum(it) }) }
        }
        val vm = LibraryAlbumsViewModel(FakeLibrary(), LibraryAlbumOrder.RECENTLY_PLAYED, me = me)
        assertEquals((1L..50L).toList(), me.albumIdCalls.single())
        assertEquals(49, vm.state.value.total)
        assertEquals(6L, vm.state.value[4]!!.id)
        vm.ensure(60)
        assertEquals(1, me.albumIdCalls.size)
    }

    @Test
    fun nothingPlayedYetIsEmptyAndFailuresCanBeRetried() = runTest(dispatcher) {
        val me = FakeMe().apply { recent = ApiResult.Ok(emptyList()) }
        val vm = LibraryAlbumsViewModel(FakeLibrary(), LibraryAlbumOrder.RECENTLY_PLAYED, me = me)
        assertEquals(0, vm.state.value.total)
        assertTrue(me.albumIdCalls.isEmpty())

        me.recent = ApiResult.Err(AppError.Offline())
        val failing = LibraryAlbumsViewModel(FakeLibrary(), LibraryAlbumOrder.RECENTLY_PLAYED, me = me)
        assertTrue(failing.state.value.failedEmpty)
        me.recent = ApiResult.Ok(listOf(9))
        me.albumsById = { ApiResult.Err(AppError.Server(500, "x")) }
        failing.retry()
        assertTrue(failing.state.value.failedEmpty)
        me.albumsById = { ids -> ApiResult.Ok(ids.map { sampleAlbum(it) }) }
        failing.retry()
        assertEquals(9L, failing.state.value[0]!!.id)
    }

    @Test
    fun aShortPageEndsTheListWhateverTheCountSaid() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 30)
        val vm = LibraryAlbumsViewModel(library)
        assertEquals(30, vm.state.value.total)
        assertEquals(SparsePagedViewModel.PAGE, library.albumCalls.single().second)
    }

    @Test
    fun aFailedFirstPageCanBeRetried() = runTest(dispatcher) {
        val library = FakeLibrary().apply { fail = io.github.jamezuk.lyrionanywhere.data.AppError.Offline() }
        val vm = LibraryAlbumsViewModel(library)
        assertTrue(vm.state.value.failedEmpty)
        library.fail = null
        vm.retry()
        assertEquals(250, vm.state.value.total)
    }

    @Test
    fun artistsHaveTheirIndexForTheirRole() = runTest(dispatcher) {
        val library = FakeLibrary().apply { index = idx("A" to 1) }
        val vm = LibraryArtistsViewModel(library)
        assertEquals(1, vm.state.value.count)
        assertEquals(listOf("artists"), library.indexCalls)
        assertEquals(ArtistRole.ALBUM_ARTIST, library.artistFilters.last().role)
        vm.setView(io.github.jamezuk.lyrionanywhere.ui.library.ArtistView.ALL)
        assertNull(library.artistFilters.last().role)
    }

    @Test
    fun theAToZRules() {
        val i = idx("A" to 3, "B" to 2, "Ö" to 4)
        assertEquals(0, AtoZ.currentAt(i, 0))
        assertEquals(0, AtoZ.currentAt(i, 2))
        assertEquals(1, AtoZ.currentAt(i, 3))
        assertEquals(2, AtoZ.currentAt(i, 100))
        assertEquals(-1, AtoZ.currentAt(emptyList(), 3))
        assertEquals(5, AtoZ.target(i[2], 9))
        // An index past the list (LMS's count changed) is kept within it.
        assertEquals(3, AtoZ.target(i[2], 4))
        assertEquals(0, AtoZ.target(i[2], 0))
        assertEquals(2, AtoZ.forChar(i, 'ö'))
        assertEquals(1, AtoZ.forChar(i, 'b'))
        assertEquals(-1, AtoZ.forChar(i, 'z'))
        assertEquals(0, AtoZ.atY(i, 0f, 300f))
        assertEquals(2, AtoZ.atY(i, 299f, 300f))
        assertEquals(2, AtoZ.atY(i, 5000f, 300f))
        assertEquals(-1, AtoZ.atY(i, Float.NaN, 300f))
        assertEquals(-1, AtoZ.atY(i, 10f, 0f))
    }

    /** Whatever an index or touch says, the scroller's sums stay in bounds. */
    @Test
    fun theAToZRulesSurviveArbitraryInput() {
        val r = Random(5)
        repeat(10_000) {
            val i = List(r.nextInt(0, 40)) { IndexEntry("K", r.nextInt().toUInt(), r.nextInt().toUInt()) }
            val total = r.nextInt(-5, 100_000)
            i.forEach { e -> assertTrue(AtoZ.target(e, total) in 0..maxOf(0, total - 1)) }
            val y = AtoZ.atY(i, r.nextFloat() * 2000 - 500, r.nextFloat() * 1000 - 100)
            assertTrue(y in -1 until maxOf(1, i.size))
            assertTrue(AtoZ.currentAt(i, r.nextInt()) in -1 until maxOf(1, i.size))
        }
    }

    @Test
    fun theOrderAndViewAreRemembered() = runTest(dispatcher) {
        // The chips are gone (Wave 3.5: one model); the order and view are still remembered.
        val settings = FakeSettings()
        val vm = LibraryPrefsViewModel(settings)
        vm.setOrder(LibraryAlbumOrder.YEAR)
        vm.setGrid(false)
        assertEquals(LibraryPrefs(null, LibraryAlbumOrder.YEAR, false), settings.state.value.library)
        assertNull(settings.state.value.library.chip)
        vm.setOrder(LibraryAlbumOrder.RECENTLY_PLAYED)
        assertEquals(LibraryAlbumOrder.RECENTLY_PLAYED, settings.state.value.library.albumOrder)
    }

    @Test
    fun searchGroupsAndPagesOneTypeAlone() = runTest(dispatcher) {
        val library = FakeLibrary()
        val vm = SearchViewModel(library, FakeSettings())
        vm.onQueryChange("road")
        advanceTimeBy(SearchViewModel.DEBOUNCE_MS + 1)
        assertEquals(listOf("road"), library.searches)
        assertEquals(listOf("Road trip"), vm.state.value.playlists.map { it.name })
        vm.setType(SearchType.TRACKS)
        assertEquals(Triple("road", 0, SearchViewModel.TYPED_PAGE), library.trackSearches.single())
        assertEquals(SearchViewModel.TYPED_PAGE, vm.typed.value.items.size)
        vm.loadMoreTyped()
        vm.loadMoreTyped()
        assertEquals(120, vm.typed.value.items.size)
        assertTrue(vm.typed.value.endReached)
        vm.loadMoreTyped()
        assertEquals(3, library.trackSearches.size)
        vm.setType(SearchType.ALBUMS)
        assertTrue(vm.typed.value.items.first() is SearchHit.OfAlbum)
        vm.setType(SearchType.ALL)
        assertTrue(vm.typed.value.items.isEmpty())
    }

    @Test
    fun recentSearchesStayOnTheDevice() = runTest(dispatcher) {
        val settings = FakeSettings()
        val vm = SearchViewModel(FakeLibrary(), settings)
        vm.onQueryChange("blue")
        vm.remember()
        vm.remember("x") // too short to keep
        vm.useRecent("red")
        assertEquals(listOf("red", "blue"), vm.state.value.recents)
        assertEquals("red", vm.state.value.query)
        vm.removeRecent("BLUE")
        assertEquals(listOf("red"), vm.state.value.recents)
        vm.clearRecents()
        assertTrue(settings.state.value.recentSearches.isEmpty())
        // Kept for twenty at most.
        repeat(30) { vm.remember("term $it") }
        assertEquals(20, vm.state.value.recents.size)
    }

    @Test
    fun theArtistPageCountsItsRolesAndCopesWithout() = runTest(dispatcher) {
        val library = FakeLibrary().apply { roleCounts = listOf(RoleCount(ArtistRole.COMPOSER, 12u)) }
        val vm = ArtistViewModel(library, 3)
        assertEquals(listOf(RoleCount(ArtistRole.COMPOSER, 12u)), (vm.state.value as Loadable.Ready).value.roles)
        library.roleCountsFail = io.github.jamezuk.lyrionanywhere.data.AppError.Server(500, "x")
        val without = ArtistViewModel(library, 3)
        assertTrue((without.state.value as Loadable.Ready).value.roles.isEmpty())
        val contributor = ContributorViewModel(library, 41, ArtistRole.COMPOSER)
        assertTrue(contributor.state.value is Loadable.Ready)
    }
}

/** Wave 2 §3.1–3.3 on screen. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class Wave2LibraryScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @androidx.compose.runtime.Composable () -> Unit) =
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    @Test
    fun oneListOfSectionsThenBrowse() {
        // Wave 3.5: one model, each section a page of its own (no chips, no second list).
        val opened = mutableListOf<io.github.jamezuk.lyrionanywhere.ui.library.LibraryDestination>()
        show { LibraryScreen(emptyList(), null, {}, onOpen = { opened += it }) }
        compose.onNodeWithTag("library-chips").assertDoesNotExist()
        compose.onNodeWithText("Artists").performClick()
        compose.onNodeWithText("Downloaded").performClick()
        compose.onNodeWithText("Browse").assertIsDisplayed()
        assertEquals(
            listOf(io.github.jamezuk.lyrionanywhere.ui.library.LibraryDestination.ARTISTS, io.github.jamezuk.lyrionanywhere.ui.library.LibraryDestination.DOWNLOADS),
            opened,
        )
    }

    private fun albums(total: Int, index: List<IndexEntry> = emptyList()) = SparseState(
        total = total,
        pages = mapOf(0 to (0 until minOf(total, 100)).map { sampleAlbum(it + 1L) }),
        index = index,
    )

    @Test
    fun albumsSortAndSwitchBetweenGridAndList() {
        val sorts = mutableListOf<LibraryAlbumOrder>()
        val grids = mutableListOf<Boolean>()
        show {
            AlbumBrowser(
                albums(3), LibraryAlbumOrder.TITLE, grid = true, onSort = { sorts += it }, onGrid = { grids += it },
                onEnsure = {}, onRetry = {}, onAlbum = {}, onMore = {},
            )
        }
        compose.onNodeWithTag("album-grid").assertIsDisplayed()
        compose.onNodeWithText("Sort: A–Z").performClick()
        compose.onNodeWithText("Recently added").performClick()
        assertEquals(listOf(LibraryAlbumOrder.NEWEST), sorts)
        compose.onNodeWithContentDescription("Show as a list").performClick()
        assertEquals(listOf(false), grids)
    }

    @Test
    fun listRowsSwipeToPlayNextAndAddToTheQueue() {
        val queued = mutableListOf<Pair<Long, Placement>>()
        show {
            AlbumBrowser(
                albums(3), LibraryAlbumOrder.NEWEST, grid = false, onSort = {}, onGrid = {},
                onEnsure = {}, onRetry = {}, onAlbum = {}, onMore = {},
                onQueue = { a, p -> queued += a.id to p },
            )
        }
        compose.onNodeWithTag("album-row-1").performTouchInput { swipeRight() }
        compose.waitForIdle()
        compose.onNodeWithTag("album-row-2").performTouchInput { swipeLeft() }
        compose.waitForIdle()
        assertEquals(listOf(2L to Placement.PLAY_NEXT, 3L to Placement.ADD_TO_QUEUE), queued)
        // The row comes back after a swipe.
        compose.onNodeWithText("Album 2").assertIsDisplayed()
    }

    @Test
    fun theAToZScrollerJumpsAndIsAdjustableForTalkBack() {
        val ensured = mutableListOf<Int>()
        show {
            AlbumBrowser(
                albums(1000, idx("A" to 400, "M" to 600)), LibraryAlbumOrder.TITLE, grid = false,
                onSort = {}, onGrid = {}, onEnsure = { ensured += it }, onRetry = {}, onAlbum = {}, onMore = {},
            )
        }
        val scroller = compose.onNodeWithTag("az-scroller")
        scroller.assert(SemanticsMatcher.expectValue(SemanticsProperties.ContentDescription, listOf("Jump to letter")))
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "A"))
        val set = scroller.fetchSemanticsNode().config[SemanticsActions.SetProgress]
        compose.runOnUiThread { set.action?.invoke(1f) }
        compose.waitForIdle()
        // The list is at "M" (entry 400), so that page is asked for.
        assertTrue(ensured.any { it in 400..499 })
        scroller.assert(SemanticsMatcher.expectValue(SemanticsProperties.StateDescription, "M"))
    }

    @Test
    fun noScrollerOutsideTitleOrder() {
        show {
            AlbumBrowser(
                albums(1000, idx("A" to 400, "M" to 600)), LibraryAlbumOrder.YEAR, grid = true,
                onSort = {}, onGrid = {}, onEnsure = {}, onRetry = {}, onAlbum = {}, onMore = {},
            )
        }
        compose.onNodeWithTag("az-scroller").assertDoesNotExist()
    }

    @Test
    fun theSortMenuOffersTheFiveOrdersInTurn() {
        val sorts = mutableListOf<LibraryAlbumOrder>()
        show {
            AlbumBrowser(
                albums(3), LibraryAlbumOrder.TITLE, grid = true, onSort = { sorts += it }, onGrid = {},
                onEnsure = {}, onRetry = {}, onAlbum = {}, onMore = {},
            )
        }
        compose.onNodeWithTag("album-sort").performClick()
        val labels = listOf("Recently added", "Recently played", "A–Z", "Artist", "Year")
        val tops = LibraryAlbumOrder.OFFERED.map { o ->
            compose.onNodeWithTag("album-sort-${o.name.lowercase()}").fetchSemanticsNode().boundsInRoot.top
        }
        assertEquals(tops.sorted(), tops)
        LibraryAlbumOrder.OFFERED.zip(labels).forEach { (o, label) ->
            compose.onNodeWithTag("album-sort-${o.name.lowercase()}").assert(hasText(label))
        }
        compose.onNodeWithTag("album-sort-title").assert(SemanticsMatcher.expectValue(SemanticsProperties.Selected, true))
        compose.onNodeWithTag("album-sort-recently_played").assert(SemanticsMatcher.expectValue(SemanticsProperties.Selected, false))
        compose.onNodeWithTag("album-sort-recently_played").performClick()
        assertEquals(listOf(LibraryAlbumOrder.RECENTLY_PLAYED), sorts)
    }

    @Test
    fun recentlyPlayedShowsItsOwnLabelNoScrollerAndBothViews() {
        val grid = androidx.compose.runtime.mutableStateOf(true)
        show {
            AlbumBrowser(
                albums(1000, idx("A" to 400, "M" to 600)), LibraryAlbumOrder.RECENTLY_PLAYED, grid = grid.value,
                onSort = {}, onGrid = { grid.value = it }, onEnsure = {}, onRetry = {}, onAlbum = {}, onMore = {},
            )
        }
        compose.onNodeWithText("Sort: Recently played").assertIsDisplayed()
        compose.onNodeWithTag("album-grid").assertIsDisplayed()
        compose.onNodeWithTag("az-scroller").assertDoesNotExist()
        compose.onNodeWithContentDescription("Show as a list").performClick()
        compose.onNodeWithTag("album-list").assertIsDisplayed()
        compose.onNodeWithTag("az-scroller").assertDoesNotExist()
    }

    @Test
    fun nothingPlayedYetSaysSo() {
        show {
            AlbumBrowser(
                SparseState(total = 0), LibraryAlbumOrder.RECENTLY_PLAYED, grid = true,
                onSort = {}, onGrid = {}, onEnsure = {}, onRetry = {}, onAlbum = {}, onMore = {},
            )
        }
        compose.onNodeWithText("Nothing played yet. What you play shows here.").assertIsDisplayed()
    }

    @Test
    fun aRecentlyPlayedFailureShowsTheErrorWithRetry() {
        var retried = 0
        show {
            AlbumBrowser(
                SparseState(error = AppError.Offline()), LibraryAlbumOrder.RECENTLY_PLAYED, grid = true,
                onSort = {}, onGrid = {}, onEnsure = {}, onRetry = { retried++ }, onAlbum = {}, onMore = {},
            )
        }
        compose.onNodeWithText("Try again").performClick()
        assertEquals(1, retried)
    }

    @Test
    fun creditsAreLinksToTheirRole() {
        val followed = mutableListOf<Credit>()
        val credits = listOf(
            Credit(ArtistRole.COMPOSER, "Some Composer", 41),
            Credit(ArtistRole.ARTIST, "Singer One", 21),
            Credit(ArtistRole.ARTIST, "Writer, Jr., Other", null),
        )
        show { CreditsSection(credits, onCredit = { followed += it }) }
        compose.onNodeWithText("Composer").assertIsDisplayed()
        val link = compose.onNodeWithText("Some Composer")
        assertEquals("Open their albums as Composer", link.fetchSemanticsNode().config[SemanticsActions.OnClick].label)
        link.performClick()
        compose.onNodeWithText("Singer One").performClick()
        // A name without an id is shown but isn't a link.
        compose.onNodeWithText("Writer, Jr., Other").assert(SemanticsMatcher.keyNotDefined(SemanticsActions.OnClick))
        assertEquals(listOf(41L, 21L), followed.map { it.artistId })
    }

    @Test
    fun appearsAsChipsSayHowManyAndOpenTheRole() {
        val roles = mutableListOf<ArtistRole>()
        show { AppearsAs(listOf(RoleCount(ArtistRole.COMPOSER, 12u), RoleCount(ArtistRole.CONDUCTOR, 1u)), onRole = { roles += it }) }
        compose.onNodeWithText("Composer (12)").assertIsDisplayed()
        compose.onNodeWithContentDescription("Conductor: 1 album").performClick()
        assertEquals(listOf(ArtistRole.CONDUCTOR), roles)
    }

    @Test
    fun nothingForAnArtistWithNoRoles() {
        show { AppearsAs(emptyList(), onRole = {}) }
        compose.onNodeWithTag("appears-as").assertDoesNotExist()
    }

    private fun searchActions(log: MutableList<String>) = SearchActions(
        onQuery = { log += "query $it" },
        onSubmit = { log += "submit" },
        onType = { log += "type $it" },
        onRetry = {},
        onLoadMore = {},
        onRecent = { log += "recent $it" },
        onRemoveRecent = { log += "remove $it" },
        onClearRecents = { log += "clear" },
        onArtist = { log += "artist ${it.id}" },
        onAlbum = { log += "album ${it.id}" },
        onPlaylist = { log += "playlist ${it.id}" },
        onPlay = { s, p -> log += "play $s $p" },
        onMore = { log += "more" },
    )

    @Test
    fun recentSearchesCanBeUsedRemovedAndCleared() {
        val log = mutableListOf<String>()
        show { SearchScreen(SearchUiState(recents = listOf("blue", "red")), PagedState(), searchActions(log)) }
        compose.onNodeWithText("Recent searches").assertIsDisplayed()
        compose.onNodeWithText("blue").performClick()
        compose.onNodeWithContentDescription("Remove red from recent searches").performClick()
        compose.onNodeWithText("Clear all").performClick()
        assertEquals(listOf("recent blue", "remove red", "clear"), log)
    }

    @Test
    fun resultsAreGroupedWithATopResultAndPlayFromTheirRows() {
        val log = mutableListOf<String>()
        val results = SearchResults(
            artists = listOf(Artist(3, "Blue Band")),
            albums = listOf(sampleAlbum(7)),
            tracks = listOf(sampleTrack(9)),
            artistsTotal = 1u,
            albumsTotal = 1u,
            tracksTotal = 40u,
            top = TopResult(SearchKind.TRACK, 0u),
        )
        show {
            SearchScreen(
                SearchUiState(query = "blue", results = results, playlists = listOf(io.github.jamezuk.lyrionanywhere.core.SavedPlaylist(5, "Road trip"))),
                PagedState(),
                searchActions(log),
            )
        }
        compose.onNodeWithTag("search-top").assertIsDisplayed()
        compose.onNodeWithText("Top result").assertIsDisplayed()
        // An album row has one trailing control, ⋮, whose sheet plays it (Wave 3.5); tracks
        // play from their rows.
        compose.onNodeWithContentDescription("Play Album 7").assertDoesNotExist()
        compose.onNodeWithContentDescription("More options for Album 7").performClick()
        assertEquals("more", log.last())
        // There are more tracks than shown: "See all" shows that type alone.
        compose.onNodeWithContentDescription("See all: Tracks").performClick()
        assertEquals("type TRACKS", log.last())
        compose.onNodeWithText("Road trip").assertIsDisplayed()
        compose.onNode(hasText("Playlists") and hasAnyAncestor(hasTestTag("search-types"))).performClick()
        assertEquals("type PLAYLISTS", log.last())
    }

    @Test
    fun oneTypePagesOnItsOwn() {
        val log = mutableListOf<String>()
        val typed = PagedState<SearchHit>(listOf(SearchHit.OfTrack(sampleTrack(9))), total = 1)
        show { SearchScreen(SearchUiState(query = "blue", results = SearchResults(emptyList(), emptyList(), emptyList()), type = SearchType.TRACKS), typed, searchActions(log)) }
        compose.onNodeWithTag("search-typed").assertIsDisplayed()
        compose.onNodeWithText("Track 9").performClick()
        assertEquals("play ${Selection.Track(9)} PLAY_NOW", log.last())
    }

    @Test
    fun browseRowsOfferTheirSwipesToTalkBack() {
        val queued = mutableListOf<Placement>()
        show {
            Box {
                BrowseSwipe({ queued += it }) { m -> TrackRow(sampleTrack(1), onClick = {}, modifier = m.testTag("row")) }
            }
        }
        val actions = compose.onNodeWithTag("row").fetchSemanticsNode().config[SemanticsActions.CustomActions]
        assertEquals(listOf("Play next", "Add to queue"), actions.map { it.label })
        actions.forEach { it.action() }
        assertEquals(listOf(Placement.PLAY_NEXT, Placement.ADD_TO_QUEUE), queued)
    }

    @Test
    fun aLibraryWithoutChipsStillShowsTheOverview() {
        show { LibraryScreen(emptyList(), null, {}, onOpen = {}) }
        compose.onNodeWithTag("library-chips").assertDoesNotExist()
        compose.onNodeWithText("Albums").assertIsDisplayed()
    }

    @Test
    fun settingsDefaultToTheOverview() {
        assertEquals(LibraryPrefs(), AppSettings().library)
    }
}

