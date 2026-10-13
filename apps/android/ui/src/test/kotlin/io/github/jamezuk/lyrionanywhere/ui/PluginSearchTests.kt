package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.runtime.Composable
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.hasTestTag
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.AppEntry
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.MenuParam
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.SearchableApp
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.ui.library.PagedState
import io.github.jamezuk.lyrionanywhere.ui.library.SearchActions
import io.github.jamezuk.lyrionanywhere.ui.library.SearchDirectory
import io.github.jamezuk.lyrionanywhere.ui.library.SearchScreen
import io.github.jamezuk.lyrionanywhere.ui.library.SearchType
import io.github.jamezuk.lyrionanywhere.ui.library.SearchUiState
import io.github.jamezuk.lyrionanywhere.ui.library.SearchViewModel
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuNavigator
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val P = "02:00:00:00:00:01"
private const val HOSTILE = "<b>x</b>‮evil"

/** An app's top menu: a search box for `search:` text, and something to play. */
private fun searchableMenu(cmd: String) = listOf(
    menuItem("Search", MenuItemKind.SEARCH, go = action(cmd, "items", input = true)),
    menuItem("A track", MenuItemKind.PLAYABLE, play = action(cmd, "playlist", "play", opens = false)),
)

private fun fakeMenus() = FakeMenus().apply {
    apps = listOf(
        AppEntry("examplemusic", "Example Music", null, action("examplemusic", "items")),
        AppEntry("exampleplain", "Example Plain", null, action("exampleplain", "items")),
    )
    radios = listOf(AppEntry("exampleradio", "Example Radio", null, action("exampleradio", "items")))
    pages["examplemusic"] = searchableMenu("examplemusic")
    pages["exampleradio"] = searchableMenu("exampleradio")
    // A search box whose text goes elsewhere than `search:`: not offered.
    pages["exampleplain"] = listOf(
        menuItem(
            "Find",
            MenuItemKind.SEARCH,
            go = action("exampleplain", "items").copy(params = listOf(MenuParam("q", "__TAGGEDINPUT__")), needsInput = true),
        ),
    )
}

/** "Search on <app>" (Wave 2 §3.3): the directory, the Search view model and the results menu. */
@OptIn(ExperimentalCoroutinesApi::class)
class PluginSearchViewModelTest : MainDispatcherTest() {
    private fun TestScope.search(vm: SearchViewModel, q: String) {
        vm.onQueryChange(q)
        advanceTimeBy(SearchViewModel.DEBOUNCE_MS + 1)
    }

    @Test
    fun theDirectoryFindsAppsWithASearchBoxOncePerSession() = runTest(dispatcher) {
        val menus = fakeMenus()
        val dir = SearchDirectory(menus)
        assertEquals(listOf("examplemusic", "exampleradio"), dir.get(P).map { it.cmd })
        assertEquals(listOf(P to listOf("examplemusic", "exampleplain", "exampleradio")), menus.searchableCalls)
        dir.get(P)
        assertEquals("cached while the apps listed are the same", 1, menus.searchableCalls.size)
        // An app left out for this account (the gateway hides admin-only plugins): looked at again.
        menus.apps = menus.apps.filter { it.cmd != "examplemusic" }
        assertEquals(listOf("exampleradio"), dir.get(P).map { it.cmd })
        assertEquals(2, menus.searchableCalls.size)
        // Without the apps lists, nothing is offered and nothing remembered.
        menus.failApps = io.github.jamezuk.lyrionanywhere.data.AppError.Server(502, "down")
        assertTrue(dir.get(P).isEmpty())
        menus.failApps = null
        assertEquals(listOf("exampleradio"), dir.get(P).map { it.cmd })
    }

    @Test
    fun findReadsTheSearchBoxAgainAndRefusesAppsNotListed() = runTest(dispatcher) {
        val menus = fakeMenus()
        val dir = SearchDirectory(menus)
        val app = dir.find(P, "examplemusic")
        assertEquals("Example Music", app?.name)
        assertEquals(listOf(MenuParam("search", "__TAGGEDINPUT__")), app?.search?.params)
        // Discovery, then the one app read again.
        assertEquals(listOf("examplemusic"), menus.searchableCalls.last().second)
        assertNull(dir.find(P, "notlisted"))
        assertNull(dir.find(P, "exampleplain"))
        assertNull(dir.find(P, "../x"))
    }

    @Test
    fun searchOffersAppsOnlyOnceTypedAndWithAPlayer() = runTest(dispatcher) {
        val menus = fakeMenus()
        val dir = SearchDirectory(menus)
        var player: String? = null
        val vm = SearchViewModel(FakeLibrary(), FakeSettings(), dir) { player }
        search(vm, "blue")
        assertTrue("no player: nothing offered", vm.state.value.searchOn.isEmpty())
        assertTrue(menus.searchableCalls.isEmpty())
        player = P
        search(vm, "blue moon")
        assertEquals(listOf("Example Music", "Example Radio"), vm.state.value.searchOn.map { it.name })
        // Typing more doesn't ask again.
        search(vm, "blue moon over")
        assertEquals(1, menus.searchableCalls.size)
        // A new Search screen in the same session uses what was found.
        val again = SearchViewModel(FakeLibrary(), FakeSettings(), dir) { P }
        search(again, "red")
        assertEquals(2, again.state.value.searchOn.size)
        assertEquals(1, menus.searchableCalls.size)
        // Without a directory, never offered.
        val none = SearchViewModel(FakeLibrary(), FakeSettings())
        search(none, "red")
        assertTrue(none.state.value.searchOn.isEmpty())
    }

    @Test
    fun anAppsResultsOpenThroughItsSearchBoxWithTheTextAsTypedText() = runTest(dispatcher) {
        val menus = fakeMenus()
        val vm = MenuViewModel(menus, null) { P }
        val hostile = "x item_id:9 menu:1 url:file:///etc/passwd"
        vm.openSearch(SearchDirectory(menus), "examplemusic", "Example Music: “x”", "  $hostile  ", P)
        val frame = vm.navigator.top!!
        assertEquals("Example Music: “x”", frame.title)
        assertEquals(listOf("examplemusic", "items"), frame.action?.cmd)
        assertEquals(listOf(MenuParam("search", "__TAGGEDINPUT__")), frame.action?.params)
        // The text goes only as typed text (filled into `search:` by the Rust layer).
        assertEquals(hostile, menus.inputs.last())
        assertEquals(Triple(P, listOf("examplemusic", "items"), 0), menus.opened.last())
        assertFalse(vm.missing.value)
        // Results items play and add as any app item; searches aren't kept as stations.
        assertEquals(2, frame.items.size)
        assertNull(MenuNavigator.stepsTo(vm.navigator.stack.value, 0))
    }

    @Test
    fun noSearchIsSentForAppsNotListedOrEmptyText() = runTest(dispatcher) {
        for ((cmd, q) in listOf("notlisted" to "abc", "exampleplain" to "abc", "examplemusic" to "   ", "examplemusic" to "")) {
            val menus = fakeMenus()
            val vm = MenuViewModel(menus, null) { P }
            vm.openSearch(SearchDirectory(menus), cmd, "t", q, P)
            assertTrue("$cmd/$q", vm.missing.value)
            assertTrue("$cmd/$q", menus.inputs.isEmpty())
        }
        // No player: nothing at all.
        val menus = fakeMenus()
        val vm = MenuViewModel(menus, null) { null }
        vm.openSearch(SearchDirectory(menus), "examplemusic", "t", "abc", null)
        assertTrue(menus.searchableCalls.isEmpty())
    }

    @Test
    fun theNavigatorRefusesASearchWithoutTextOrABox() = runTest(dispatcher) {
        val menus = fakeMenus()
        val nav = MenuNavigator(this, menus) { P }
        nav.startSearch("t", action("examplemusic", "items", input = true), "  ")
        nav.startSearch("t", action("examplemusic", "items"), "abc")
        nav.startSearch("t", action("examplemusic", "playlist", "play", opens = false, input = true), "abc")
        assertTrue(nav.stack.value.isEmpty())
        assertTrue(menus.opened.isEmpty())
    }
}

private val apps = listOf(
    SearchableApp("examplemusic", "Example Music", null, action("examplemusic", "items", input = true)),
    SearchableApp("examplevideo", HOSTILE, null, action("examplevideo", "items", input = true)),
)

private fun actions(log: MutableList<String>) = SearchActions(
    {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, { _, _ -> }, {},
    onSearchOn = { log += "on ${it.cmd}" },
)

private val noResults = SearchResults(emptyList(), emptyList(), emptyList())

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class PluginSearchScreenTest {
    @get:Rule
    val compose = createComposeRule()

    private fun show(content: @Composable () -> Unit) = compose.setContent { LyrionTheme(dynamicColour = false) { content() } }

    @Test
    fun rowsBelowTheLibraryResultsOpenTheAppsSearch() {
        val log = mutableListOf<String>()
        val results = SearchResults(listOf(Artist(3, "Blue Band")), emptyList(), emptyList(), 1u, 0u, 0u, null)
        show { SearchScreen(SearchUiState(query = " blue moon ", results = results, searchOn = apps), PagedState(), actions(log)) }
        compose.onNodeWithText("Blue Band").assertIsDisplayed()
        compose.onNodeWithTag("search-all").performScrollToNode(hasTestTag("search-on-examplemusic"))
        compose.onNodeWithText("Search on…").assertIsDisplayed()
        compose.onNodeWithText("Search Example Music for “blue moon”").assertIsDisplayed()
        // A hostile name is plain text.
        compose.onNodeWithText("Search $HOSTILE for “blue moon”").assertIsDisplayed()
        compose.onNodeWithTag("search-on-examplemusic")
            .assert(SemanticsMatcher("labelled for TalkBack") { it.config[SemanticsActions.OnClick].label == "Open results" })
            .performClick()
        assertEquals(listOf("on examplemusic"), log)
    }

    @Test
    fun offeredEvenWhenTheLibraryHasNothing() {
        val log = mutableListOf<String>()
        show { SearchScreen(SearchUiState(query = "blue", results = noResults, searchOn = apps), PagedState(), actions(log)) }
        compose.onNodeWithText("Nothing found for “blue”.").assertIsDisplayed()
        compose.onNodeWithTag("search-on-examplevideo").performClick()
        assertEquals(listOf("on examplevideo"), log)
    }

    @Test
    fun notOfferedForOneTypeOrBeforeASearch() {
        show {
            SearchScreen(
                SearchUiState(query = "blue", results = noResults, searchOn = apps, type = SearchType.ALBUMS),
                PagedState(total = 0),
                actions(mutableListOf()),
            )
        }
        compose.onNodeWithText("Search on…").assertDoesNotExist()
    }

    @Test
    fun notOfferedWhileTheBoxIsEmpty() {
        show { SearchScreen(SearchUiState(query = "", searchOn = apps), PagedState(), actions(mutableListOf())) }
        compose.onNodeWithText("Search on…").assertDoesNotExist()
    }

    @Test
    fun accessible() {
        show { SearchScreen(SearchUiState(query = "blue", results = noResults, searchOn = apps), PagedState(), actions(mutableListOf())) }
        compose.assertAccessible()
    }
}
