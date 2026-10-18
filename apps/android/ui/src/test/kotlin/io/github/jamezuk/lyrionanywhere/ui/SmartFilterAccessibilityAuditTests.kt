package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsActions
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsUi
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditor
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorActions
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorDialog
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorUi
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersActions
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersUi
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeEditActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.PagedState
import io.github.jamezuk.lyrionanywhere.ui.library.RuleKind
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Smart filters (Wave 2 §5.6), audited as TalkBack sees it ([assertAccessible]) in every
 * state: the Filters list (loading, failed, empty, full, its menu and the delete question),
 * the rule builder (new, every rule, searching, genres failing, a problem, a refusal, as a
 * dialog), a filter's page (loading, albums, none, deleted, failed) and Home's filter shelf
 * (with albums, empty, failed, and the shelf picker in edit mode).
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class SmartFilterAccessibilityAuditTests {
    @get:Rule
    val compose = createComposeRule()

    private fun audit(content: @Composable () -> Unit) {
        compose.setContent { LyrionTheme(dynamicColour = false) { content() } }
        compose.assertAccessible()
    }

    @Test fun filtersLoading() = audit { FiltersScreen(FiltersUi(), FiltersActions()) }

    @Test fun filtersFailed() = audit { FiltersScreen(FiltersUi(Loadable.Failed(AppError.Offline())), FiltersActions()) }

    @Test fun filtersNone() = audit { FiltersScreen(FiltersUi(Loadable.Ready(emptyList())), FiltersActions()) }

    @Test fun filtersListed() = audit { FiltersScreen(FilterSamples.list, FiltersActions()) }

    @Test fun filtersWhilePlaying() = audit { FiltersScreen(FilterSamples.list.copy(playing = true), FiltersActions()) }

    @Test fun filtersFull() = audit { FiltersScreen(FilterSamples.full, FiltersActions()) }

    @Test
    fun filtersMenuAndDeleteQuestion() {
        audit { FiltersScreen(FilterSamples.list, FiltersActions()) }
        compose.onNodeWithTag("filter-more-11").performClick()
        compose.assertAccessible()
        compose.onNodeWithTag("filter-delete-11").performClick()
        compose.assertAccessible()
    }

    @Test fun editorNew() = audit { FilterEditor(FilterEditorUi(), FilterEditorActions()) }

    @Test fun editorWithEveryRule() = audit { FilterEditor(FilterSamples.everyRule, FilterEditorActions()) }

    @Test
    fun editorAtTwentyGenres() = audit {
        val g = (1L..24L).map { io.github.jamezuk.lyrionanywhere.core.Genre(it, "Genre $it") }
        FilterEditor(
            FilterSamples.everyRule.copy(genres = (1L..20L).toList(), genreChoices = Loadable.Ready(g)),
            FilterEditorActions(),
        )
    }

    @Test fun editorSearchingForAnArtist() = audit { FilterEditor(FilterSamples.searching.copy(searching = true, artistResults = emptyList()), FilterEditorActions()) }

    @Test fun editorWithArtistResults() = audit { FilterEditor(FilterSamples.searching, FilterEditorActions()) }

    @Test fun editorNoArtistFound() = audit { FilterEditor(FilterSamples.searching.copy(artistResults = emptyList()), FilterEditorActions()) }

    private val genreOnly = FilterEditorUi(name = "x", rules = listOf(RuleKind.GENRE))

    @Test fun editorGenresLoading() = audit { FilterEditor(genreOnly.copy(genreChoices = Loadable.Loading), FilterEditorActions()) }

    @Test fun editorGenresFailed() = audit { FilterEditor(genreOnly.copy(genreChoices = Loadable.Failed(AppError.Offline())), FilterEditorActions()) }

    @Test fun editorNoGenres() = audit { FilterEditor(genreOnly.copy(genreChoices = Loadable.Ready(emptyList())), FilterEditorActions()) }

    @Test fun editorWithAProblem() = audit { FilterEditor(FilterEditorUi(problem = io.github.jamezuk.lyrionanywhere.data.FilterProblem.Refused("A filter has 1 to 8 rules.")), FilterEditorActions()) }

    @Test fun editorRefusedBySaving() = audit { FilterEditor(FilterSamples.everyRule.copy(failure = AppError.InvalidInput("too many filters"), saving = false), FilterEditorActions()) }

    @Test fun editorSaving() = audit { FilterEditor(FilterSamples.everyRule.copy(saving = true), FilterEditorActions()) }

    @Test fun editorAsADialog() = audit { FilterEditorDialog(FilterSamples.everyRule.copy(editing = LATE_JAZZ), FilterEditorActions()) }

    @Test fun filterPageLoading() = audit { FilterAlbumsScreen(FilterAlbumsUi(), FilterAlbumsActions()) }

    @Test fun filterPageWithAlbums() = audit { FilterAlbumsScreen(FilterSamples.albums(), FilterAlbumsActions()) }

    @Test fun filterPageMoreToCome() = audit {
        FilterAlbumsScreen(FilterSamples.albums().copy(albums = PagedState(listOf(sampleAlbum(1)), total = 80, loading = true)), FilterAlbumsActions())
    }

    @Test fun filterPageWithNoAlbums() = audit { FilterAlbumsScreen(FilterSamples.albums(n = 0), FilterAlbumsActions()) }

    @Test fun filterPageDeleted() = audit { FilterAlbumsScreen(FilterSamples.albums(filter = null), FilterAlbumsActions()) }

    @Test fun filterPageFailed() = audit { FilterAlbumsScreen(FilterAlbumsUi(Loadable.Failed(AppError.Offline())), FilterAlbumsActions()) }

    @Test
    fun filterPageDeleteQuestion() {
        audit { FilterAlbumsScreen(FilterSamples.albums(), FilterAlbumsActions()) }
        compose.onNodeWithTag("filter-page-delete").performClick()
        compose.assertAccessible()
    }

    private val noHome = HomeActions({}, {}, {}, {}, {}, {}, {})

    @Test fun homeFilterShelf() = audit { HomeScreen(FilterSamples.homeFeed(), noHome) }

    @Test fun homeFilterShelfLoading() = audit { HomeScreen(FilterSamples.homeFeed(Loadable.Loading), noHome) }

    @Test fun homeFilterShelfEmpty() = audit { HomeScreen(FilterSamples.homeFeed(Loadable.Ready(emptyList())), noHome) }

    @Test fun homeFilterShelfFailed() = audit { HomeScreen(FilterSamples.homeFeed(Loadable.Failed(AppError.Offline())), noHome) }

    @Test
    fun homeEditWithFilterShelvesAndThePicker() {
        val feed = FilterSamples.homeFeed().let { it.copy(draft = it.layout) }
        audit { HomeScreen(feed, noHome, edit = HomeEditActions({ _, _ -> }, { _, _ -> }, {}, {}, {}, {}, {})) }
        compose.onNodeWithText("Add a shelf").performClick()
        compose.assertAccessible()
    }
}
