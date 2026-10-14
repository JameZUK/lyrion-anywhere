package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.SmartFilter
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsActions
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsUi
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditor
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorActions
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorDialog
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorUi
import io.github.jamezuk.lyrionanywhere.ui.library.FilterMessage
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersActions
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersUi
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeEditActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeed
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.PagedState
import io.github.jamezuk.lyrionanywhere.ui.library.RuleKind
import io.github.jamezuk.lyrionanywhere.ui.library.filterMessageText
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

/** Sample states of the smart filter screens (shared with the accessibility audit). */
internal object FilterSamples {
    val list = FiltersUi(Loadable.Ready(listOf(LATE_JAZZ, COMPOSERS)))
    val full = FiltersUi(Loadable.Ready((1L..50L).map { LATE_JAZZ.copy(id = it, name = "Filter $it") }))
    val genres = (1L..12L).map { Genre(it, "Genre $it") }
    val everyRule = FilterEditorUi(
        name = "Sixties jazz",
        rules = listOf(RuleKind.GENRE, RuleKind.DECADE, RuleKind.ARTIST, RuleKind.COMPILATION),
        genres = listOf(2L, 5L),
        genreChoices = Loadable.Ready(genres),
        decade = 1960,
        artistId = 3,
        artistName = "Example Composer",
        role = ArtistRole.COMPOSER,
        compilation = false,
        sort = AlbumSort.YEAR,
        limit = "40",
    )
    val searching = FilterEditorUi(
        name = "One composer",
        rules = listOf(RuleKind.ARTIST),
        artistQuery = "exa",
        artistResults = listOf(Artist(3, "Example Composer"), Artist(4, "Example Band")),
    )
    fun albums(filter: SmartFilter? = LATE_JAZZ, n: Int = 6) =
        FilterAlbumsUi(Loadable.Ready(filter), PagedState((1L..n.toLong()).map { sampleAlbum(it) }, total = n))

    fun homeFeed(filterAlbums: Loadable<List<io.github.jamezuk.lyrionanywhere.core.Album>> = Loadable.Ready((1L..4L).map { sampleAlbum(it) })) = HomeFeed(
        recent = Loadable.Ready(listOf(sampleAlbum(1))),
        layout = HomeLayout(listOf(HomeShelf(ShelfKind.FILTER, LATE_JAZZ.id, false), HomeShelf(ShelfKind.FILTER, 404, false)), false),
        layoutLoaded = true,
        stations = Loadable.Ready(emptyList()),
        filters = listOf(LATE_JAZZ, COMPOSERS),
        filtersLoaded = true,
        filterAlbums = mapOf(LATE_JAZZ.id to filterAlbums),
    )
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class SmartFilterScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    @Test
    fun theListPlaysShufflesOpensAndEdits() {
        val played = mutableListOf<Pair<Long, Boolean>>()
        val opened = mutableListOf<Long>()
        val edited = mutableListOf<Long>()
        show {
            FiltersScreen(
                FilterSamples.list,
                FiltersActions(
                    onOpen = { opened += it.id },
                    onEdit = { edited += it.id },
                    onPlay = { f, s -> played += f.id to s },
                ),
            )
        }
        compose.onNodeWithText("Late night jazz").assertIsDisplayed()
        compose.onNodeWithText("2 genres · 1960s").assertExists()
        compose.onNodeWithText("An artist (composer) · No compilations").assertExists()
        // Play and Shuffle are in each row's ⋮ (Wave 3.5 rule 3).
        compose.onNodeWithContentDescription("More for Late night jazz").performClick()
        compose.onNodeWithTag("filter-play-11").performClick()
        compose.onNodeWithContentDescription("More for Example Composer").performClick()
        compose.onNodeWithTag("filter-shuffle-12").performClick()
        assertEquals(listOf(11L to false, 12L to true), played)
        compose.onNodeWithText("Late night jazz").performClick()
        assertEquals(listOf(11L), opened)
        compose.onNodeWithContentDescription("More for Example Composer").performClick()
        compose.onNodeWithText("Edit").performClick()
        assertEquals(listOf(12L), edited)
    }

    @Test
    fun deletingAsksFirst() {
        val deleted = mutableListOf<Long>()
        show { FiltersScreen(FilterSamples.list, FiltersActions(onDelete = { deleted += it.id })) }
        compose.onNodeWithTag("filter-more-11").performClick()
        compose.onNodeWithTag("filter-delete-11").performClick()
        compose.onNodeWithText("Delete “Late night jazz”?").assertExists()
        compose.onNodeWithText("Cancel").performClick()
        assertTrue(deleted.isEmpty())
        compose.onNodeWithTag("filter-more-11").performClick()
        compose.onNodeWithTag("filter-delete-11").performClick()
        compose.onNodeWithTag("filter-delete-confirm").performClick()
        assertEquals(listOf(11L), deleted)
    }

    @Test
    fun newIsOfferedUntilFifty() {
        var news = 0
        show { FiltersScreen(FilterSamples.list, FiltersActions(onNew = { news++ })) }
        compose.onNodeWithTag("filter-new").assertIsEnabled().performClick()
        assertEquals(1, news)
    }

    @Test
    fun atFiftyNewIsOff() {
        show { FiltersScreen(FilterSamples.full, FiltersActions()) }
        compose.onNodeWithTag("filter-new").assertIsNotEnabled()
        compose.onNodeWithText("You have the most allowed (50)", substring = true).assertExists()
    }

    @Test
    fun noFiltersYet() {
        show { FiltersScreen(FiltersUi(Loadable.Ready(emptyList())), FiltersActions()) }
        compose.onNodeWithTag("filters-none").assertExists()
    }

    @Test
    fun theRuleBuilderReachesItsActions() {
        val added = mutableListOf<RuleKind>()
        val genres = mutableListOf<Long>()
        val decades = mutableListOf<Int>()
        val sorts = mutableListOf<AlbumSort>()
        var saves = 0
        show {
            FilterEditor(
                FilterSamples.everyRule.copy(rules = listOf(RuleKind.GENRE, RuleKind.DECADE)),
                FilterEditorActions(
                    onAddRule = { added += it },
                    onGenre = { genres += it },
                    onDecade = { decades += it },
                    onSort = { sorts += it },
                    onSave = { saves++ },
                ),
            )
        }
        compose.onNodeWithText("2 of up to 20 chosen").assertExists()
        compose.onNodeWithTag("filter-genre-7").performScrollTo().performClick()
        compose.onNodeWithTag("filter-decade-1990").performScrollTo().performClick()
        compose.onNodeWithContentDescription("Add rule: Artist").performScrollTo().performClick()
        compose.onNodeWithTag("filter-sort-random").performScrollTo().performClick()
        compose.onNodeWithTag("filter-save").performClick()
        assertEquals(listOf(7L), genres)
        assertEquals(listOf(1990), decades)
        assertEquals(listOf(RuleKind.ARTIST), added)
        assertEquals(listOf(AlbumSort.RANDOM), sorts)
        assertEquals(1, saves)
    }

    @Test
    fun aProblemIsShownAndAnnounced() {
        show { FilterEditor(FilterEditorUi(problem = "A filter has 1 to 8 rules."), FilterEditorActions()) }
        compose.onNodeWithTag("filter-error").assertExists()
        compose.onNodeWithText("A filter has 1 to 8 rules.").assertExists()
        compose.onNodeWithText("Add at least one rule.").assertExists()
    }

    @Test
    fun aServerRefusalIsShown() {
        show { FilterEditor(FilterEditorUi(failure = AppError.InvalidInput("too many filters")), FilterEditorActions()) }
        compose.onNodeWithText("Too many filters").assertExists()
    }

    @Test
    fun anArtistIsPickedFromTheResults() {
        val picked = mutableListOf<Artist>()
        val roles = mutableListOf<ArtistRole?>()
        var removed: RuleKind? = null
        show {
            FilterEditor(
                FilterSamples.searching,
                FilterEditorActions(onArtist = { picked += it }, onRole = { roles += it }, onRemoveRule = { removed = it }),
            )
        }
        compose.onNodeWithText("Example Band").performClick()
        compose.onNodeWithTag("filter-role-conductor").performScrollTo().performClick()
        compose.onNodeWithTag("filter-role-any").performClick()
        compose.onNodeWithContentDescription("Remove the artist rule").performClick()
        assertEquals(listOf(Artist(4, "Example Band")), picked)
        assertEquals(listOf(ArtistRole.CONDUCTOR, null), roles)
        assertEquals(RuleKind.ARTIST, removed)
    }

    @Test
    fun theChosenArtistCanBeChanged() {
        var changes = 0
        val compilations = mutableListOf<Boolean>()
        show { FilterEditor(FilterSamples.everyRule, FilterEditorActions(onChangeArtist = { changes++ }, onCompilation = { compilations += it })) }
        compose.onNodeWithText("Artist is Example Composer").performScrollTo().assertExists()
        compose.onNodeWithTag("filter-artist-change").performClick()
        compose.onNodeWithTag("filter-compilation-true").performScrollTo().performClick()
        assertEquals(1, changes)
        assertEquals(listOf(true), compilations)
    }

    @Test
    fun theNameAndLimitAreTyped() {
        val names = mutableListOf<String>()
        val limits = mutableListOf<String>()
        show { FilterEditor(FilterEditorUi(limit = ""), FilterEditorActions(onName = { names += it }, onLimit = { limits += it })) }
        compose.onNodeWithTag("filter-name").performTextInput("Mine")
        compose.onNodeWithTag("filter-limit").performTextInput("2")
        assertTrue(names.toString(), "Mine" in names)
        assertTrue(limits.toString(), "2" in limits)
    }

    @Test
    fun theDialogClosesWithoutSaving() {
        var cancels = 0
        show { FilterEditorDialog(FilterEditorUi(editing = LATE_JAZZ, name = "Late night jazz"), FilterEditorActions(onCancel = { cancels++ })) }
        compose.onNodeWithText("Edit smart filter").assertExists()
        compose.onNodeWithContentDescription("Close without saving").performClick()
        assertEquals(1, cancels)
    }

    @Test
    fun aFiltersPagePlaysEditsAndDeletes() {
        val plays = mutableListOf<Boolean>()
        var edits = 0
        var deletes = 0
        show {
            FilterAlbumsScreen(
                FilterSamples.albums(),
                FilterAlbumsActions(onPlay = { plays += it }, onEdit = { edits++ }, onDelete = { deletes++ }),
            )
        }
        compose.onNodeWithText("2 genres · 1960s").assertExists()
        compose.onNodeWithTag("filter-page-play").performClick()
        compose.onNodeWithTag("filter-page-shuffle").performClick()
        compose.onNodeWithTag("filter-page-edit").performClick()
        compose.onNodeWithTag("filter-page-delete").performClick()
        compose.onNodeWithTag("filter-delete-confirm").performClick()
        assertEquals(listOf(false, true), plays)
        assertEquals(1, edits)
        assertEquals(1, deletes)
    }

    @Test
    fun aFilterWithNoAlbumsCantBePlayed() {
        show { FilterAlbumsScreen(FilterSamples.albums(n = 0), FilterAlbumsActions()) }
        compose.onNodeWithText("No albums match this filter.").assertExists()
        compose.onNodeWithTag("filter-page-play").assertIsNotEnabled()
    }

    @Test
    fun aDeletedFiltersPageSaysSo() {
        show { FilterAlbumsScreen(FilterSamples.albums(filter = null), FilterAlbumsActions()) }
        compose.onNodeWithText("This filter has been deleted, or isn’t yours.").assertExists()
    }

    @Test
    fun aFilterShelfPlaysAndADeletedOneIsLeftOut() {
        val plays = mutableListOf<Triple<Long, String, Boolean>>()
        val opened = mutableListOf<Long>()
        show {
            HomeScreen(
                FilterSamples.homeFeed(),
                HomeActions({}, {}, {}, {}, {}, {}, {}, onPlayFilter = { id, n, s -> plays += Triple(id, n, s) }, onFilter = { id, _ -> opened += id }),
            )
        }
        compose.onNodeWithTag("shelf-filter-11").assertExists()
        compose.onNodeWithTag("shelf-filter-404").assertDoesNotExist()
        compose.onNodeWithContentDescription("Play Late night jazz").performClick()
        compose.onNodeWithContentDescription("Shuffle Late night jazz").performClick()
        compose.onNodeWithContentDescription("See all: Late night jazz", substring = true).performClick()
        assertEquals(listOf(Triple(11L, "Late night jazz", false), Triple(11L, "Late night jazz", true)), plays)
        assertEquals(listOf(11L), opened)
    }

    @Test
    fun anEmptyFilterShelfSaysSoWithoutPlay() {
        show { HomeScreen(FilterSamples.homeFeed(Loadable.Ready(emptyList())), HomeActions({}, {}, {}, {}, {}, {}, {})) }
        compose.onNodeWithText("No albums match this filter.").assertExists()
        compose.onNodeWithContentDescription("Play Late night jazz").assertDoesNotExist()
    }

    @Test
    fun filtersAreOfferedAsShelvesInEditMode() {
        val added = mutableListOf<HomeShelf>()
        val feed = FilterSamples.homeFeed().let { it.copy(draft = it.layout) }
        show {
            HomeScreen(
                feed,
                HomeActions({}, {}, {}, {}, {}, {}, {}),
                edit = HomeEditActions({ _, _ -> }, { _, _ -> }, {}, { added += it }, {}, {}, {}),
            )
        }
        // The shelves are named after their filters; a deleted one is still there to remove.
        compose.onNodeWithContentDescription("Remove: Smart filter: Late night jazz").assertExists()
        compose.onNodeWithContentDescription("Remove: Smart filter").assertExists()
        compose.onNodeWithText("Add a shelf").performClick()
        compose.onNodeWithTag("home-add-filter-11").assertDoesNotExist()
        compose.onNodeWithTag("home-add-filter-12").performScrollTo().performClick()
        assertEquals(listOf(HomeShelf(ShelfKind.FILTER, 12, false)), added)
        assertTrue(HomeLayouts.takesId(ShelfKind.FILTER))
    }

    @Test
    fun libraryHasAFiltersChip() {
        val toggled = mutableListOf<LibraryChip>()
        show {
            LibraryScreen(
                emptyList(), null, {}, onNewMusic = {}, onOpen = {},
                chip = LibraryChip.FILTERS,
                onChip = { toggled += it },
                chipContent = { c -> androidx.compose.material3.Text("Showing $c") },
            )
        }
        compose.onNodeWithText("Showing FILTERS").assertExists()
        compose.onNodeWithTag("chip-filters").performClick()
        assertEquals(listOf(LibraryChip.FILTERS), toggled)
    }

    @Test
    fun messagesReadWell() {
        val res = RuntimeEnvironment.getApplication().resources
        assertEquals("Shuffling “Late night jazz”", filterMessageText(res, FilterMessage.Playing("Late night jazz", true)))
        assertEquals("No albums match “Late night jazz”", filterMessageText(res, FilterMessage.NoAlbums("Late night jazz")))
        assertEquals("Added “Late night jazz”", filterMessageText(res, FilterMessage.Saved("Late night jazz", true)))
        assertEquals("Deleted “Late night jazz”", filterMessageText(res, FilterMessage.Deleted("Late night jazz")))
        assertEquals("Too many filters", filterMessageText(res, FilterMessage.Failed(AppError.InvalidInput("too many filters"))))
    }
}
