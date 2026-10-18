package io.github.jamezuk.lyrionanywhere.ui

import androidx.lifecycle.ViewModelStore
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.FilterRule
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.GenrePage
import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.FilterProblem
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.FilterMessage
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeedViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.HomeMessage
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.RuleKind
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val KITCHEN = "aa:00:00:00:00:01"

/** A library with many genres (for the 20-genre cap) and an artist search. */
private class GenreLibrary(private val count: Int = 30) : io.github.jamezuk.lyrionanywhere.data.LibraryRepository by FakeLibrary() {
    val genreCalls = mutableListOf<Int>()
    var failGenres: AppError? = null
    val artistSearches = mutableListOf<String?>()

    override suspend fun genres(start: Int, count: Int, libraryId: String?, search: String?): ApiResult<GenrePage> {
        genreCalls += count
        failGenres?.let { return ApiResult.Err(it) }
        return ApiResult.Ok(GenrePage(this.count.toUInt(), (1..this.count).map { Genre(it.toLong(), "Genre $it") } + Genre(0, "Bad")))
    }

    override suspend fun artists(start: Int, count: Int, search: String?, filter: io.github.jamezuk.lyrionanywhere.core.LibraryFilter) =
        ApiResult.Ok(io.github.jamezuk.lyrionanywhere.core.ArtistPage(2u, listOf(Artist(3, "Example Composer"), Artist(-1, "Bad")))).also {
            artistSearches += search
        }

    override suspend fun artistAlbums(artistId: Long, role: ArtistRole?): ApiResult<List<Album>> =
        ApiResult.Ok(listOf(Album(1, "Album", "Someone else", 8, 1999, null, false), Album(2, "Album", "Example Composer", artistId, 2001, null, false)))
}

class FiltersViewModelTest : MainDispatcherTest() {
    @Test
    fun listsPlaysAndDeletes() = runTest(dispatcher) {
        val filters = FakeFilters()
        val players = FakePlayers()
        val store = ViewModelStore()
        val vm = inStore(store) { FiltersViewModel(filters, players) { KITCHEN } }
        try {
            assertEquals(listOf(LATE_JAZZ, COMPOSERS), (vm.state.value.filters as Loadable.Ready).value)
            assertFalse(vm.state.value.full)
            filters.albumCount = 3
            vm.play(LATE_JAZZ, shuffle = true)
            assertEquals(FilterMessage.Playing("Late night jazz", true), vm.messages.first())
            assertEquals(
                listOf(
                    "shuffle SONGS $KITCHEN",
                    "enqueue Album(id=100) PLAY_NOW null $KITCHEN",
                    "enqueue Album(id=101) ADD_TO_QUEUE null $KITCHEN",
                    "enqueue Album(id=102) ADD_TO_QUEUE null $KITCHEN",
                ),
                players.commands,
            )
            assertFalse(vm.state.value.playing)
            vm.delete(LATE_JAZZ)
            assertEquals(FilterMessage.Deleted("Late night jazz"), vm.messages.first())
            assertEquals(listOf(COMPOSERS), (vm.state.value.filters as Loadable.Ready).value)
        } finally {
            store.clear()
        }
    }

    @Test
    fun withoutAPlayerOrAlbumsNothingIsSent() = runTest(dispatcher) {
        val filters = FakeFilters().apply { albumCount = 0 }
        val players = FakePlayers()
        val store = ViewModelStore()
        var player: String? = null
        val vm = inStore(store) { FiltersViewModel(filters, players) { player } }
        try {
            vm.play(LATE_JAZZ, shuffle = false)
            assertEquals(FilterMessage.NoPlayer, vm.messages.first())
            assertTrue("not even the filter is read", filters.pages.isEmpty())
            player = KITCHEN
            vm.play(LATE_JAZZ, shuffle = true)
            assertEquals(FilterMessage.NoAlbums("Late night jazz"), vm.messages.first())
            assertTrue("no shuffle without albums", players.commands.isEmpty())
        } finally {
            store.clear()
        }
    }

    @Test
    fun failuresAreSaidAndTheListIsKept() = runTest(dispatcher) {
        val filters = FakeFilters()
        val store = ViewModelStore()
        val vm = inStore(store) { FiltersViewModel(filters, FakePlayers()) { KITCHEN } }
        try {
            filters.failDelete = AppError.Offline()
            vm.delete(COMPOSERS)
            assertEquals(FilterMessage.Failed(AppError.Offline()), vm.messages.first())
            assertEquals(2, (vm.state.value.filters as Loadable.Ready).value.size)
            filters.failList = AppError.Server(500, "x")
            vm.load()
            assertTrue("a failed reload keeps the list", vm.state.value.filters is Loadable.Ready)
            assertTrue(vm.messages.first() is FilterMessage.Failed)
        } finally {
            store.clear()
        }
        val first = ViewModelStore()
        val failing = inStore(first) { FiltersViewModel(FakeFilters().apply { failList = AppError.Offline() }, FakePlayers()) { KITCHEN } }
        try {
            assertEquals(Loadable.Failed(AppError.Offline()), failing.state.value.filters)
        } finally {
            first.clear()
        }
    }

    @Test
    fun fiftyFiltersIsTheMost() = runTest(dispatcher) {
        val many = (1L..50L).map { LATE_JAZZ.copy(id = it, name = "Filter $it") }
        val store = ViewModelStore()
        val vm = inStore(store) { FiltersViewModel(FakeFilters(many), FakePlayers()) { KITCHEN } }
        try {
            assertTrue(vm.state.value.full)
        } finally {
            store.clear()
        }
    }

    @Test
    fun aSavedFilterIsAnnouncedAndTheListReread() = runTest(dispatcher) {
        val filters = FakeFilters()
        val store = ViewModelStore()
        val vm = inStore(store) { FiltersViewModel(filters, FakePlayers()) { KITCHEN } }
        try {
            val calls = filters.listCalls
            vm.saved(COMPOSERS, added = true)
            assertEquals(FilterMessage.Saved("Example Composer", true), vm.messages.first())
            assertEquals(calls + 1, filters.listCalls)
        } finally {
            store.clear()
        }
    }
}

class FilterEditorViewModelTest : MainDispatcherTest() {
    @Test
    fun aNewFilterIsBuiltCheckedAndSaved() = runTest(dispatcher) {
        val filters = FakeFilters()
        val lib = GenreLibrary()
        val store = ViewModelStore()
        val vm = inStore(store) { FilterEditorViewModel(filters, lib) }
        try {
            vm.save()
            assertEquals(FilterProblem.Refused("A filter needs a name of 1 to 64 characters."), vm.state.value.problem)
            assertTrue("nothing sent", filters.created.isEmpty())
            vm.setName("  Sixties jazz  ")
            assertNull("editing clears the problem", vm.state.value.problem)
            vm.save()
            assertEquals(FilterProblem.Refused("A filter has 1 to 8 rules."), vm.state.value.problem)
            vm.addRule(RuleKind.GENRE)
            assertEquals(listOf(1L, 2L), (vm.state.value.genreChoices as Loadable.Ready).value.take(2).map { it.id })
            assertTrue("a bad genre id from the server is left out", (vm.state.value.genreChoices as Loadable.Ready).value.none { it.id == 0L })
            vm.save()
            assertEquals(FilterProblem.Refused("Choose 1 to 20 genres."), vm.state.value.problem)
            vm.toggleGenre(4)
            vm.toggleGenre(7)
            vm.toggleGenre(4)
            vm.addRule(RuleKind.DECADE)
            vm.save()
            assertEquals(FilterProblem.ChooseDecade, vm.state.value.problem)
            vm.setDecade(1965)
            assertNull("not a decade", vm.state.value.decade)
            vm.setDecade(1960)
            vm.setSort(AlbumSort.RANDOM)
            vm.setLimit("2a5x0")
            assertEquals("250", vm.state.value.limit)
            vm.save()
            assertNull(vm.state.value.problem)
            val sent = filters.created.single()
            assertEquals("Sixties jazz", sent.name)
            assertEquals(listOf(FilterRule.Genre(listOf(7)), FilterRule.Decade(1960u)), sent.rules)
            assertEquals(AlbumSort.RANDOM, sent.sort)
            assertEquals(250, sent.limit)
            assertEquals("Sixties jazz", vm.state.value.saved?.name)
            vm.save()
            assertEquals("saved once only", 1, filters.created.size)
        } finally {
            store.clear()
        }
    }

    @Test
    fun eachRuleOnceAtMostTwentyGenres() = runTest(dispatcher) {
        val store = ViewModelStore()
        val vm = inStore(store) { FilterEditorViewModel(FakeFilters(), GenreLibrary()) }
        try {
            vm.addRule(RuleKind.GENRE)
            vm.addRule(RuleKind.GENRE)
            assertEquals(listOf(RuleKind.GENRE), vm.state.value.rules)
            assertEquals(listOf(RuleKind.DECADE, RuleKind.ARTIST, RuleKind.COMPILATION), vm.state.value.addable)
            for (g in 1L..25L) vm.toggleGenre(g)
            assertEquals(20, vm.state.value.genres.size)
            vm.toggleGenre(0)
            vm.toggleGenre(-3)
            assertEquals("bad ids are never chosen", 20, vm.state.value.genres.size)
            vm.removeRule(RuleKind.GENRE)
            assertTrue(vm.state.value.rules.isEmpty())
            vm.setName("x".repeat(100))
            assertEquals(64, vm.state.value.name.length)
        } finally {
            store.clear()
        }
    }

    @Test
    fun anArtistIsFoundAndChosen() = runTest(dispatcher) {
        val lib = GenreLibrary()
        val filters = FakeFilters()
        val store = ViewModelStore()
        val vm = inStore(store) { FilterEditorViewModel(filters, lib) }
        try {
            vm.setName("By one composer")
            vm.addRule(RuleKind.ARTIST)
            vm.save()
            assertEquals(FilterProblem.ChooseArtist, vm.state.value.problem)
            vm.setArtistQuery("e")
            advanceUntilIdle()
            assertTrue("one letter doesn't search", lib.artistSearches.isEmpty())
            vm.setArtistQuery("ex")
            vm.setArtistQuery("exa")
            advanceTimeBy(FilterEditorViewModel.SEARCH_PAUSE_MS + 1)
            assertEquals("only after the pause, once", listOf<String?>("exa"), lib.artistSearches)
            assertEquals(listOf(Artist(3, "Example Composer")), vm.state.value.artistResults)
            vm.chooseArtist(Artist(-1, "Bad"))
            assertNull(vm.state.value.artistId)
            vm.chooseArtist(vm.state.value.artistResults.single())
            vm.setRole(ArtistRole.COMPOSER)
            assertEquals("", vm.state.value.artistQuery)
            vm.save()
            assertEquals(listOf(FilterRule.Artist(3, ArtistRole.COMPOSER)), filters.created.single().rules)
        } finally {
            store.clear()
        }
    }

    @Test
    fun aSavedFilterIsEditedInPlace() = runTest(dispatcher) {
        val filters = FakeFilters()
        val lib = GenreLibrary()
        val store = ViewModelStore()
        val vm = inStore(store) { FilterEditorViewModel(filters, lib, COMPOSERS) }
        try {
            val s = vm.state.value
            assertEquals("Example Composer", s.name)
            assertEquals(listOf(RuleKind.ARTIST, RuleKind.COMPILATION), s.rules)
            assertEquals(3L, s.artistId)
            assertEquals("the artist's name, from its own album", "Example Composer", s.artistName)
            assertEquals(ArtistRole.COMPOSER, s.role)
            assertFalse(s.compilation)
            assertEquals(AlbumSort.RANDOM, s.sort)
            assertEquals("100", s.limit)
            assertTrue("no genre rule: genres aren't read", lib.genreCalls.isEmpty())
            vm.setCompilation(true)
            vm.save()
            assertEquals(12L, filters.updated.single().first)
            assertEquals(FilterRule.Compilation(true), filters.updated.single().second.rules[1])
            assertTrue(filters.created.isEmpty())
        } finally {
            store.clear()
        }
        val store2 = ViewModelStore()
        val withGenres = inStore(store2) { FilterEditorViewModel(FakeFilters(), lib, LATE_JAZZ) }
        try {
            assertEquals(listOf(4L, 5L), withGenres.state.value.genres)
            assertEquals(1960, withGenres.state.value.decade)
            assertTrue(withGenres.state.value.genreChoices is Loadable.Ready)
        } finally {
            store2.clear()
        }
    }

    @Test
    fun aRefusalFromTheServerIsShown() = runTest(dispatcher) {
        val filters = FakeFilters().apply { failSave = AppError.InvalidInput("too many filters") }
        val lib = GenreLibrary().apply { failGenres = AppError.Offline() }
        val store = ViewModelStore()
        val vm = inStore(store) { FilterEditorViewModel(filters, lib) }
        try {
            vm.setName("One more")
            vm.addRule(RuleKind.COMPILATION)
            vm.save()
            assertEquals(AppError.InvalidInput("too many filters"), vm.state.value.failure)
            assertFalse(vm.state.value.saving)
            assertNull(vm.state.value.saved)
            vm.addRule(RuleKind.GENRE)
            assertEquals(Loadable.Failed(AppError.Offline()), vm.state.value.genreChoices)
            lib.failGenres = null
            vm.retryGenres()
            assertTrue(vm.state.value.genreChoices is Loadable.Ready)
        } finally {
            store.clear()
        }
    }

    @Test
    fun eightRulesIsTheMostAndAFilterOfOtherKindsStillLoads() = runTest(dispatcher) {
        // A filter from a newer gateway, with a rule kind repeated: shown once each.
        val odd = LATE_JAZZ.copy(rules = listOf(FilterRule.Decade(1965u), FilterRule.Decade(1970u), FilterRule.Genre((1L..30L).toList())))
        val store = ViewModelStore()
        val vm = inStore(store) { FilterEditorViewModel(FakeFilters(), GenreLibrary(), odd) }
        try {
            assertEquals(listOf(RuleKind.DECADE, RuleKind.GENRE), vm.state.value.rules)
            assertEquals(1970, vm.state.value.decade)
            assertEquals(20, vm.state.value.genres.size)
        } finally {
            store.clear()
        }
    }

    @Test
    fun problemsAreSentences() {
        assertEquals("Choose a decade.", FilterEditorViewModel.sentence("choose a decade"))
        assertEquals("Done.", FilterEditorViewModel.sentence("Done."))
    }
}

class FilterAlbumsViewModelTest : MainDispatcherTest() {
    @Test
    fun albumsArePagedToTheEnd() = runTest(dispatcher) {
        val filters = FakeFilters()
        val me = FakeMe()
        val store = ViewModelStore()
        val vm = inStore(store) { FilterAlbumsViewModel(filters, me, FakePlayers(), { KITCHEN }, LATE_JAZZ.id) }
        try {
            assertEquals(LATE_JAZZ, (vm.state.value.filter as Loadable.Ready).value)
            assertEquals(50, vm.state.value.albums.items.size)
            assertFalse(vm.state.value.albums.endReached)
            vm.loadMore()
            vm.loadMore()
            vm.loadMore()
            val a = vm.state.value.albums
            assertEquals(120, a.items.size)
            assertTrue(a.endReached)
            assertEquals(listOf(0, 50, 100), filters.pages.map { it.second })
            assertEquals((100L..219L).toList(), a.items.map { it.id })
        } finally {
            store.clear()
        }
    }

    @Test
    fun unknownAlbumsDontEndTheListEarly() = runTest(dispatcher) {
        val filters = FakeFilters()
        // LMS no longer knows every other album.
        val me = FakeMe().apply { albumsById = { ids -> ApiResult.Ok(ids.filter { it % 2 == 0L }.map { sampleAlbum(it) }) } }
        val store = ViewModelStore()
        val vm = inStore(store) { FilterAlbumsViewModel(filters, me, FakePlayers(), { KITCHEN }, LATE_JAZZ.id) }
        try {
            assertFalse(vm.state.value.albums.endReached)
            vm.loadMore()
            vm.loadMore()
            assertTrue(vm.state.value.albums.endReached)
            assertEquals(60, vm.state.value.albums.items.size)
        } finally {
            store.clear()
        }
    }

    @Test
    fun aDeletedFilterSaysSo() = runTest(dispatcher) {
        val store = ViewModelStore()
        val vm = inStore(store) { FilterAlbumsViewModel(FakeFilters(), FakeMe(), FakePlayers(), { KITCHEN }, 77) }
        try {
            assertEquals(Loadable.Ready(null), vm.state.value.filter)
            vm.play(shuffle = false)
            vm.delete()
            assertFalse("nothing to play or delete", vm.state.value.deleted)
        } finally {
            store.clear()
        }
    }

    @Test
    fun playEditAndDelete() = runTest(dispatcher) {
        val filters = FakeFilters()
        val players = FakePlayers()
        val store = ViewModelStore()
        val vm = inStore(store) { FilterAlbumsViewModel(filters, FakeMe(), players, { KITCHEN }, COMPOSERS.id) }
        try {
            vm.play(shuffle = false)
            assertEquals(FilterMessage.Playing("Example Composer", false), vm.messages.first())
            assertEquals("50 albums: one played, 49 added", 50, players.commands.size)
            val renamed = COMPOSERS.copy(name = "Renamed")
            filters.pages.clear()
            vm.saved(renamed)
            assertEquals(renamed, (vm.state.value.filter as Loadable.Ready).value)
            assertEquals(FilterMessage.Saved("Renamed", false), vm.messages.first())
            assertEquals("run afresh from the start", 0, filters.pages.first().second)
            vm.saved(LATE_JAZZ)
            assertEquals("another filter's save is ignored", renamed, (vm.state.value.filter as Loadable.Ready).value)
            vm.delete()
            assertTrue(vm.state.value.deleted)
            assertEquals(listOf(COMPOSERS.id), filters.deleted)
        } finally {
            store.clear()
        }
    }

    @Test
    fun aFailedPageCanBeRetried() = runTest(dispatcher) {
        val filters = FakeFilters().apply { failAlbums = AppError.Offline() }
        val store = ViewModelStore()
        val vm = inStore(store) { FilterAlbumsViewModel(filters, FakeMe(), FakePlayers(), { KITCHEN }, LATE_JAZZ.id) }
        try {
            assertTrue(vm.state.value.albums.failedEmpty)
            filters.failAlbums = null
            vm.loadMore()
            assertEquals(50, vm.state.value.albums.items.size)
        } finally {
            store.clear()
        }
    }
}

/** Home's filter shelves (Wave 2 §5.6), including one whose filter was deleted. */
class HomeFilterShelfTest : MainDispatcherTest() {
    private fun layout(vararg ids: Long) = HomeLayout(ids.map { HomeShelf(ShelfKind.FILTER, it, false) }, false)

    @Test
    fun aFilterShelfShowsItsFirstAlbumsAndPlays() = runTest(dispatcher) {
        val me = FakeMe().apply { layout = layout(LATE_JAZZ.id) }
        val filters = FakeFilters()
        val players = FakePlayers()
        val store = ViewModelStore()
        val vm = inStore(store) { HomeFeedViewModel(FakeLibrary(), me = me, player = { KITCHEN }, filters = filters, players = players) }
        try {
            val feed = vm.state.value
            assertTrue(feed.filtersLoaded)
            assertEquals(20, (feed.filterAlbums[LATE_JAZZ.id] as Loadable.Ready).value.size)
            assertEquals(Triple(LATE_JAZZ.id, 0, 20), filters.pages.single())
            vm.playFilter(LATE_JAZZ.id, LATE_JAZZ.name, shuffle = true)
            assertEquals(HomeMessage.Filter(FilterMessage.Playing("Late night jazz", true)), vm.messages.first())
            assertEquals("shuffle SONGS $KITCHEN", players.commands.first())
        } finally {
            store.clear()
        }
    }

    @Test
    fun aDeletedFiltersShelfIsNeitherAskedForNorShown() = runTest(dispatcher) {
        val me = FakeMe().apply { layout = layout(404, LATE_JAZZ.id) }
        val filters = FakeFilters()
        val store = ViewModelStore()
        val vm = inStore(store) { HomeFeedViewModel(FakeLibrary(), me = me, player = { KITCHEN }, filters = filters, players = FakePlayers()) }
        try {
            assertEquals(listOf(LATE_JAZZ.id), filters.pages.map { it.first })
            assertNull(vm.state.value.filterAlbums[404])
        } finally {
            store.clear()
        }
    }

    @Test
    fun whenTheListCantBeReadTheShelfStillTriesAndSaysWhy() = runTest(dispatcher) {
        val me = FakeMe().apply { layout = layout(LATE_JAZZ.id) }
        val filters = FakeFilters().apply {
            failList = AppError.Offline()
            failAlbums = AppError.Offline()
        }
        val store = ViewModelStore()
        val vm = inStore(store) { HomeFeedViewModel(FakeLibrary(), me = me, player = { KITCHEN }, filters = filters, players = FakePlayers()) }
        try {
            assertFalse(vm.state.value.filtersLoaded)
            assertEquals(Loadable.Failed(AppError.Offline()), vm.state.value.filterAlbums[LATE_JAZZ.id])
        } finally {
            store.clear()
        }
    }

    @Test
    fun aFilterShelfAddedInEditModeLoadsOnSave() = runTest(dispatcher) {
        val me = FakeMe()
        val filters = FakeFilters()
        val store = ViewModelStore()
        val vm = inStore(store) { HomeFeedViewModel(FakeLibrary(), me = me, player = { KITCHEN }, filters = filters, players = FakePlayers()) }
        try {
            assertTrue("the picker has the filters", vm.state.value.filters.isNotEmpty())
            assertTrue(filters.pages.isEmpty())
            vm.edit()
            vm.add(HomeShelf(ShelfKind.FILTER, COMPOSERS.id, false))
            vm.add(HomeShelf(ShelfKind.FILTER, COMPOSERS.id, false))
            vm.add(HomeShelf(ShelfKind.FILTER, null, false))
            assertEquals("once, and only with an id", 1, vm.state.value.draft!!.shelves.count { it.kind == ShelfKind.FILTER })
            vm.save()
            assertTrue(HomeLayouts.has(me.layout, ShelfKind.FILTER, COMPOSERS.id))
            assertTrue(vm.state.value.filterAlbums[COMPOSERS.id] is Loadable.Ready)
        } finally {
            store.clear()
        }
    }

    @Test
    fun withoutAPlayerAFilterShelfSaysSo() = runTest(dispatcher) {
        val me = FakeMe().apply { layout = layout(LATE_JAZZ.id) }
        val players = FakePlayers()
        val store = ViewModelStore()
        val vm = inStore(store) { HomeFeedViewModel(FakeLibrary(), me = me, player = { null }, filters = FakeFilters(), players = players) }
        try {
            vm.playFilter(LATE_JAZZ.id, LATE_JAZZ.name, shuffle = false)
            assertEquals(HomeMessage.Filter(FilterMessage.NoPlayer), vm.messages.first())
            assertTrue(players.commands.isEmpty())
        } finally {
            store.clear()
        }
    }
}
