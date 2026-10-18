package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.MenuOutcome
import io.github.jamezuk.lyrionanywhere.core.MenuPage
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.SuitabilityCache
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumDetail
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistDetail
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistSection
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistView
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.GenresViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeedViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.orKeep
import io.github.jamezuk.lyrionanywhere.ui.library.SearchViewModel
import io.github.jamezuk.lyrionanywhere.ui.menus.AppsViewModel
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuEvent
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuNavigator
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class BrowseViewModelTest : MainDispatcherTest() {
    @Test
    fun albumListsKeepTheirFilterAndFollowTheVirtualLibrary() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 30)
        val lib = MutableStateFlow<String?>(null)
        val vm = AlbumsViewModel(library, LibraryFilter(compilation = true), libraryId = lib)
        assertEquals(LibraryFilter(compilation = true), library.albumFilters.last())
        lib.value = "ab12"
        assertEquals(LibraryFilter(compilation = true, libraryId = "ab12"), library.albumFilters.last())
        assertEquals(0, library.albumCalls.last().first)
        assertEquals(30, vm.state.value.items.size)
    }

    @Test
    fun randomAlbumsComeAsOneBatchAndShuffleAgain() = runTest(dispatcher) {
        val library = FakeLibrary(totalAlbums = 1000)
        val vm = AlbumsViewModel(library, sort = AlbumSort.RANDOM, batch = 60)
        assertEquals(60, vm.state.value.items.size)
        assertTrue(vm.state.value.endReached)
        vm.loadMore()
        assertEquals(1, library.albumCalls.size)
        vm.reload()
        assertEquals(2, library.albumCalls.size)
        assertEquals(AlbumSort.RANDOM, library.albumCalls.last().third)
    }

    @Test
    fun repeatedAlbumsAreShownOnceAndEndTheList() = runTest(dispatcher) {
        // A confused server answering the same page again must not loop.
        val library = object : io.github.jamezuk.lyrionanywhere.data.LibraryRepository by FakeLibrary() {
            var calls = 0

            override suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String?, filter: LibraryFilter) =
                ApiResult.Ok(io.github.jamezuk.lyrionanywhere.core.AlbumPage(10_000u, (1..100).map { sampleAlbum(it.toLong()) })).also { calls++ }
        }
        val vm = AlbumsViewModel(library)
        vm.loadMore()
        assertEquals(100, vm.state.value.items.size)
        assertTrue(vm.state.value.endReached)
        vm.loadMore()
        assertEquals(2, library.calls)
    }

    @Test
    fun artistRoleChipsAppearOnlyWhenTheLibraryHasThem() = runTest(dispatcher) {
        val library = FakeLibrary().apply { roles = setOf(ArtistRole.COMPOSER, ArtistRole.BAND) }
        val vm = ArtistsViewModel(library)
        assertEquals(
            listOf(ArtistView.ALBUM_ARTISTS, ArtistView.ALL, ArtistView.COMPOSERS, ArtistView.BANDS),
            vm.views.value,
        )
        assertEquals(ArtistRole.ALBUM_ARTIST, library.artistFilters.last().role)
        vm.setView(ArtistView.ALL)
        assertNull(library.artistFilters.last().role)
        vm.setView(ArtistView.COMPOSERS)
        assertEquals(ArtistRole.COMPOSER, library.artistFilters.last().role)
        // Inside a genre: all artists of that genre, no role chips.
        val genre = ArtistsViewModel(library, LibraryFilter(genreId = 4))
        assertTrue(genre.views.value.isEmpty())
        assertEquals(LibraryFilter(genreId = 4), library.artistFilters.last())
    }

    @OptIn(ExperimentalCoroutinesApi::class)
    @Test
    fun genresSearchAfterAPause() = runTest(dispatcher) {
        val library = FakeLibrary()
        val vm = GenresViewModel(library)
        assertEquals(listOf<String?>(null), library.genreQueries)
        vm.onQueryChange("ja")
        vm.onQueryChange("jaz")
        advanceTimeBy(SearchViewModel.DEBOUNCE_MS + 1)
        assertEquals(listOf(null, "jaz"), library.genreQueries)
        assertEquals("Jazz", vm.state.value.items.single().name)
    }

    @Test
    fun albumDetailsGroupDiscsAndSumUp() {
        val single = AlbumDetail(sampleAlbum(7), listOf(sampleTrack(1), sampleTrack(2)))
        assertEquals(1, single.discs.size)
        assertNull(single.discs.single().number)
        assertEquals(listOf("Jazz"), single.genres)
        assertEquals(listOf("FLAC 24/96"), single.formats)
        assertEquals(122.0, single.totalSecs, 0.0)
        assertFalse(single.variousArtists)

        val multi = AlbumDetail(
            sampleAlbum(7),
            listOf(sampleTrack(3, disc = 2), sampleTrack(1, disc = 1), sampleTrack(2, disc = 1, artist = "Guest")),
        )
        assertEquals(listOf(1, 2), multi.discs.map { it.number })
        assertEquals(listOf(1L, 2L), multi.discs[0].tracks.map { it.id })
        assertTrue(multi.variousArtists)
    }

    @Test
    fun artistAlbumsAreGroupedByRole() {
        val all = listOf(sampleAlbum(1), sampleAlbum(2), sampleAlbum(3, compilation = true))
        val d = ArtistDetail.group(all, listOf(sampleAlbum(1)))
        assertEquals(
            listOf(ArtistSection.ALBUMS, ArtistSection.APPEARS_ON, ArtistSection.COMPILATIONS),
            d.sections.map { it.first },
        )
        assertEquals(listOf(2L), d.sections[1].second.map { it.id })
        // Without role information: one list.
        val plain = ArtistDetail.group(all, null)
        assertEquals(listOf(ArtistSection.ALBUMS), plain.sections.map { it.first })
        assertEquals(3, plain.sections.single().second.size)
        assertEquals("music/3/cover", plain.headerArt)
    }

    @Test
    fun anArtistPageSurvivesARoleQueryFailing() = runTest(dispatcher) {
        val library = FakeLibrary().apply { albumArtistOf = null }
        val vm = ArtistViewModel(library, 3)
        val ready = vm.state.value as Loadable.Ready
        assertEquals(listOf(ArtistSection.ALBUMS), ready.value.sections.map { it.first })
    }

    @Test
    fun homeLoadsEachCarouselAndReshuffles() = runTest(dispatcher) {
        val library = FakeLibrary()
        val vm = HomeFeedViewModel(library)
        val feed = vm.state.value
        assertTrue(feed.recent is Loadable.Ready)
        assertEquals(HomeFeedViewModel.CAROUSEL, (feed.random as Loadable.Ready).value.size)
        assertEquals("Radio", (feed.favourites as Loadable.Ready).value.single().name)
        assertEquals("Road trip", (feed.playlists as Loadable.Ready).value.single().name)
        assertEquals(setOf(AlbumSort.NEWEST, AlbumSort.RANDOM), library.albumCalls.map { it.third }.toSet())
        vm.reshuffle()
        assertEquals(AlbumSort.RANDOM, library.albumCalls.last().third)
        // A reload that fails (a reconnect) keeps what was shown rather than an error.
        val shown = vm.state.value.recent
        library.fail = AppError.Offline("down")
        vm.refresh()
        assertEquals(shown, vm.state.value.recent)
        assertTrue(vm.state.value.favourites is Loadable.Ready)
        // Nothing shown yet: the error is shown, and one failing carousel doesn't take the
        // others with it.
        val fresh = HomeFeedViewModel(library)
        assertTrue(fresh.state.value.recent is Loadable.Failed)
        assertTrue(fresh.state.value.favourites is Loadable.Ready)
    }

    @Test
    fun aFailedReloadKeepsWhatWasShown() {
        val ready: Loadable<Int> = Loadable.Ready(1)
        val failed: Loadable<Int> = Loadable.Failed(AppError.Offline("down"))
        assertEquals(ready, failed.orKeep(ready))
        assertEquals(failed, failed.orKeep(null))
        assertEquals(failed, failed.orKeep(Loadable.Loading))
        assertEquals(Loadable.Ready(2), Loadable.Ready(2).orKeep(ready))
        assertEquals(Loadable.Loading, (Loadable.Loading as Loadable<Int>).orKeep(ready))
    }
}

class MenuNavigatorTest : MainDispatcherTest() {
    private fun nav(menus: FakeMenus, player: String? = "aa:00:00:00:00:01", scope: kotlinx.coroutines.CoroutineScope) =
        MenuNavigator(scope, menus, { player })

    @Test
    fun opensPagesAndGoesBack() = runTest(dispatcher) {
        val menus = FakeMenus().apply {
            pages["app"] = (1..150).map { menuItem("Item $it", MenuItemKind.FOLDER, go = action("sub", "items")) }
            pages["sub"] = listOf(menuItem("Leaf", MenuItemKind.TEXT))
        }
        val n = nav(menus, scope = backgroundScope)
        n.start("App", action("app", "items"))
        assertEquals(100, n.top!!.items.size)
        assertTrue(n.top!!.canLoadMore)
        n.loadMore()
        assertEquals(150, n.top!!.items.size)
        assertFalse(n.top!!.canLoadMore)
        n.choose(n.top!!.items[0])
        assertEquals(2, n.stack.value.size)
        assertEquals("Item 1", n.top!!.title)
        assertTrue(n.back())
        assertEquals(1, n.stack.value.size)
        val events = mutableListOf<MenuEvent>()
        val job = backgroundScope.launch { n.events.collect { events += it } }
        assertFalse(n.back())
        assertEquals(listOf(MenuEvent.Closed), events)
        job.cancel()
    }

    @Test
    fun playableItemsPlayAndHintsAreFollowed() = runTest(dispatcher) {
        val play = action("app", "playlist", "play", opens = false, next = "nowPlaying")
        val menus = FakeMenus().apply {
            pages["app"] = listOf(menuItem("Track", MenuItemKind.PLAYABLE, play = play))
            runResult = ApiResult.Ok(MenuOutcome.Done("nowPlaying"))
        }
        val n = nav(menus, scope = backgroundScope)
        val events = mutableListOf<MenuEvent>()
        backgroundScope.launch { n.events.collect { events += it } }
        n.start("App", action("app", "items"))
        n.choose(n.top!!.items.single())
        assertEquals(listOf(listOf("app", "playlist", "play")), menus.ran)
        assertEquals(listOf(MenuEvent.Done(io.github.jamezuk.lyrionanywhere.ui.menus.MenuLabel.Verb(io.github.jamezuk.lyrionanywhere.ui.menus.MenuVerb.PLAY)), MenuEvent.NowPlaying), events)
    }

    @Test
    fun anActionThatAnswersWithAMenuShowsIt() = runTest(dispatcher) {
        val menus = FakeMenus().apply {
            pages["app"] = listOf(menuItem("Mix", MenuItemKind.ACTION, go = action("x", "mix", opens = false)))
            runResult = ApiResult.Ok(MenuOutcome.Menu(MenuPage("Choices", 1u, listOf(menuItem("One", MenuItemKind.TEXT)))))
        }
        val n = nav(menus, scope = backgroundScope)
        n.start("App", action("app", "items"))
        n.choose(n.top!!.items.single())
        assertEquals("Choices", n.top!!.title)
        assertEquals("One", n.top!!.items.single().text)
    }

    @Test
    fun searchesSendTheTypedText() = runTest(dispatcher) {
        val search = action("app", "items", input = true)
        val menus = FakeMenus().apply { pages["app"] = listOf(menuItem("Search", MenuItemKind.SEARCH, go = search)) }
        val n = nav(menus, scope = backgroundScope)
        n.start("App", action("app", "items"))
        n.search(n.top!!.items.single(), "   ")
        assertEquals(1, menus.opened.size)
        n.search(n.top!!.items.single(), " abba ")
        assertEquals("abba", menus.inputs.last())
        assertEquals(2, n.stack.value.size)
    }

    @Test
    fun withoutAPlayerNothingIsSent() = runTest(dispatcher) {
        val menus = FakeMenus()
        val n = nav(menus, player = null, scope = backgroundScope)
        val events = mutableListOf<MenuEvent>()
        backgroundScope.launch { n.events.collect { events += it } }
        n.start("App", action("app", "items"))
        assertTrue(menus.opened.isEmpty())
        assertEquals(listOf(MenuEvent.NoPlayer), events)
    }

    @Test
    fun failuresAreReportedAndRetried() = runTest(dispatcher) {
        val menus = FakeMenus().apply {
            pages["app"] = listOf(menuItem("Run", MenuItemKind.ACTION, go = action("x", "do", opens = false)))
            runResult = ApiResult.Err(AppError.Forbidden("not offered"))
        }
        val n = nav(menus, scope = backgroundScope)
        val events = mutableListOf<MenuEvent>()
        backgroundScope.launch { n.events.collect { events += it } }
        n.start("App", action("app", "items"))
        n.choose(n.top!!.items.single())
        assertEquals(listOf(MenuEvent.Failed(AppError.Forbidden("not offered"))), events)
    }

    @Test
    fun aPluginIsFoundAgainByItsId() = runTest(dispatcher) {
        val menus = FakeMenus().apply { pages["x"] = listOf(menuItem("Inside", MenuItemKind.TEXT)) }
        val vm = MenuViewModel(menus) { "aa:00:00:00:00:01" }
        vm.openPlugin("pluginX", "aa:00:00:00:00:01")
        assertEquals("Inside", vm.navigator.top!!.items.single().text)
        val gone = MenuViewModel(menus) { "aa:00:00:00:00:01" }
        gone.openPlugin("removed", "aa:00:00:00:00:01")
        assertTrue(gone.missing.value)
    }
}

class AppsViewModelTest : MainDispatcherTest() {
    @Test
    fun unusableAppsAreTuckedAwayAndJudgementsRemembered() = runTest(dispatcher) {
        var now = 0L
        val cache = SuitabilityCache({ now })
        val menus = FakeMenus().apply {
            fits["someradio"] = Suitability.Failed("Not available remotely.")
            fits["x"] = Suitability.NeedsSetup("Log in first")
        }
        val vm = AppsViewModel(menus, { "aa:00:00:00:00:01" }, cache)
        val s = vm.state.value
        assertEquals(1, s.hidden)
        assertTrue(s.shown(s.radios).isEmpty())
        assertEquals("Some App", s.shown(s.apps).single().name)
        assertEquals(Suitability.NeedsSetup("Log in first"), s.plugins.single().fit)
        vm.toggleShowAll()
        assertEquals(1, vm.state.value.shown(vm.state.value.radios).size)
        // Asked again within ten minutes: remembered.
        assertEquals(3, menus.probes)
        AppsViewModel(menus, { "aa:00:00:00:00:01" }, cache)
        assertEquals(3, menus.probes)
        now += SuitabilityCache.TTL_MS + 1
        AppsViewModel(menus, { "aa:00:00:00:00:01" }, cache)
        assertEquals(6, menus.probes)
    }

    @Test
    fun withoutAPlayerAppsAreListedButNotProbed() = runTest(dispatcher) {
        val menus = FakeMenus()
        val vm = AppsViewModel(menus, { null }, SuitabilityCache({ 0L }))
        assertTrue(vm.state.value.noPlayer)
        assertEquals(1, vm.state.value.apps.size)
        assertTrue(vm.state.value.plugins.isEmpty())
        assertEquals(0, menus.probes)
    }

    @Test
    fun aFailureListingAppsIsShown() = runTest(dispatcher) {
        val menus = FakeMenus().apply { failApps = AppError.Offline("down") }
        val vm = AppsViewModel(menus, { "aa:00:00:00:00:01" }, SuitabilityCache({ 0L }))
        assertEquals(AppError.Offline("down"), vm.state.value.error)
    }
}

class PlayActionsTest : MainDispatcherTest() {
    @Test
    fun shuffleTurnsShuffleOnThenPlays() = runTest(dispatcher) {
        val players = FakePlayers()
        val vm = PlayerViewModel(players, FakeEvents(), FakeSettings()) { 0L }
        vm.start()
        players.commands.clear()
        vm.playShuffled(Selection.Album(7))
        assertEquals(
            listOf("shuffle ${ShuffleMode.SONGS} aa:00:00:00:00:02", "enqueue ${Selection.Album(7)} ${Placement.PLAY_NOW} null aa:00:00:00:00:02"),
            players.commands,
        )
        vm.playFavourite("f0.0", Placement.PLAY_NEXT)
        assertEquals("favourite f0.0 ${Placement.PLAY_NEXT} aa:00:00:00:00:02", players.commands.last())
        vm.stop()
    }
}
