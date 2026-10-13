package io.github.jamezuk.lyrionanywhere.ui

import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.HistoryEntry
import io.github.jamezuk.lyrionanywhere.core.HistoryPage
import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.MenuAction
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.MenuPage
import io.github.jamezuk.lyrionanywhere.core.MenuStep
import io.github.jamezuk.lyrionanywhere.core.PinnedMenu
import io.github.jamezuk.lyrionanywhere.core.RunnerUpdate
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationBatch
import io.github.jamezuk.lyrionanywhere.core.StationSpec
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeEditActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeed
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeedViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.HomeMessage
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuEvent
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuFrame
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuNavigator
import io.github.jamezuk.lyrionanywhere.ui.menus.SaveKind
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.HistorySettings
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryUiState
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryViewModel
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val PLAYER = "aa:00:00:00:00:01"

private fun shelf(kind: ShelfKind, id: Long? = null, hidden: Boolean = false) = HomeShelf(kind, id, hidden)

/** The gateway's `/v1/me/…`, in memory. */
class FakeMe : MeRepository {
    var layout: HomeLayout = HomeLayouts.DEFAULT
    var saveResult: ApiResult<HomeLayout>? = null
    val saved = mutableListOf<HomeLayout>()
    var stations = listOf(
        Station(1, "Forgotten favourites", StationSpec.Forgotten, null),
        Station(2, "Fresh", StationSpec.Fresh, null),
        Station(3, "More like One", StationSpec.Menu, "this menu item is no longer offered"),
    )
    var menus = listOf(PinnedMenu(9, "Some Plugin", null))
    val created = mutableListOf<Pair<String, StationSpec>>()
    val started = mutableListOf<Pair<Long, String>>()
    var startResult: ApiResult<StationBatch> = ApiResult.Ok(StationBatch.Tracks(listOf(1, 2)))
    val savedStations = mutableListOf<Triple<List<MenuStep>, MenuAction, MenuAction?>>()
    val pinned = mutableListOf<List<MenuStep>>()
    var saveStationResult: ApiResult<Long> = ApiResult.Ok(40)
    var historyEnabled = true
    var history = listOf(
        HistoryEntry(2, "a".repeat(64), 7, null, 1_790_000_100, 200.0, false),
        HistoryEntry(1, "b".repeat(64), 7, null, 1_790_000_000, 61.0, true),
    )
    var enableResult: ApiResult<Boolean>? = null
    var cleared = 0
    private val active = MutableStateFlow<Long?>(null)
    override val activeStation: StateFlow<Long?> = active

    override suspend fun history(limit: Int, before: String?) =
        ApiResult.Ok(HistoryPage(historyEnabled, 396u, history, null))

    override suspend fun clearHistory(): ApiResult<Long> {
        cleared++
        return ApiResult.Ok(history.size.toLong())
    }

    override suspend fun setHistoryEnabled(enabled: Boolean): ApiResult<Boolean> =
        enableResult ?: ApiResult.Ok(enabled).also { historyEnabled = enabled }

    override suspend fun shelfAlbums(kind: ShelfKind, limit: Int): ApiResult<List<Long>> = when (kind) {
        ShelfKind.RECENT -> ApiResult.Ok(listOf(7, 3))
        ShelfKind.MOST -> ApiResult.Ok(listOf(3))
        else -> ApiResult.Ok(emptyList())
    }

    override suspend fun albums(ids: List<Long>): ApiResult<List<Album>> = ApiResult.Ok(ids.map { sampleAlbum(it) })

    override suspend fun tracks(ids: List<Long>): ApiResult<List<Track>> = ApiResult.Ok(ids.map { sampleTrack(it) })

    override suspend fun home() = ApiResult.Ok(layout)

    override suspend fun saveHome(layout: HomeLayout): ApiResult<HomeLayout> {
        saved += layout
        return saveResult ?: ApiResult.Ok(layout.copy(isDefault = false)).also { this.layout = it.value }
    }

    override suspend fun resetHome(): ApiResult<HomeLayout> {
        layout = HomeLayouts.DEFAULT
        return ApiResult.Ok(layout)
    }

    override suspend fun stations() = ApiResult.Ok(stations)

    override suspend fun createStation(name: String, spec: StationSpec): ApiResult<Long> {
        created += name to spec
        stations = stations + Station(50, name, spec, null)
        return ApiResult.Ok(50)
    }

    override suspend fun deleteStation(id: Long): ApiResult<Unit> {
        stations = stations.filterNot { it.id == id }
        return ApiResult.Ok(Unit)
    }

    override suspend fun saveMenuStation(name: String, steps: List<MenuStep>, play: MenuAction, add: MenuAction?, player: String): ApiResult<Long> {
        savedStations += Triple(steps, play, add)
        return saveStationResult
    }

    override suspend fun menus() = ApiResult.Ok(menus)

    override suspend fun pinMenu(name: String, steps: List<MenuStep>, player: String): ApiResult<Long> {
        pinned += steps
        return ApiResult.Ok(9)
    }

    override suspend fun deleteMenu(id: Long) = ApiResult.Ok(Unit)

    override suspend fun openMenu(id: Long, player: String) = ApiResult.Ok(MenuPage("Some Plugin", 0u, emptyList()))

    override suspend fun startStation(id: Long, player: String): ApiResult<StationBatch> {
        started += id to player
        if (startResult is ApiResult.Ok && (startResult as ApiResult.Ok).value !is StationBatch.Broken) active.value = id
        return startResult
    }

    val statuses = mutableListOf<Pair<String, List<Long>>>()
    var statusUpdate: RunnerUpdate = RunnerUpdate.Idle

    override suspend fun onStatus(player: String, tracks: Int, current: Int, playing: Long?, queued: List<Long>): RunnerUpdate {
        statuses += player to queued
        return statusUpdate
    }

    override fun stopStation() {
        active.value = null
    }
}

class HomeLayoutsTest {
    @Test
    fun theDefaultIsTheAgreedOrder() {
        assertEquals(
            listOf(ShelfKind.RECENT, ShelfKind.STATIONS, ShelfKind.NEW, ShelfKind.MOST, ShelfKind.ON_THIS_DAY),
            HomeLayouts.DEFAULT.shelves.map { it.kind },
        )
        assertNull(HomeLayouts.problem(HomeLayouts.DEFAULT))
    }

    @Test
    fun shelvesMoveHideAndGo() {
        val l = HomeLayouts.DEFAULT
        val moved = HomeLayouts.move(l, 0, 4)
        assertEquals(ShelfKind.RECENT, moved.shelves.last().kind)
        assertFalse(moved.isDefault)
        assertEquals(l, HomeLayouts.move(l, 0, 9))
        assertEquals(l, HomeLayouts.move(l, -1, 0))
        assertTrue(HomeLayouts.setHidden(l, 2, true).shelves[2].hidden)
        assertEquals(l, HomeLayouts.setHidden(l, 7, true))
        assertEquals(4, HomeLayouts.remove(l, 0).shelves.size)
        assertEquals(l, HomeLayouts.remove(l, 5))
    }

    @Test
    fun onlyWhatTheGatewayTakesCanBeAdded() {
        val l = HomeLayouts.DEFAULT
        val withStation = HomeLayouts.add(l, shelf(ShelfKind.STATION, 4))!!
        assertEquals(6, withStation.shelves.size)
        assertNull("twice", HomeLayouts.add(withStation, shelf(ShelfKind.STATION, 4)))
        assertNull("already there", HomeLayouts.add(l, shelf(ShelfKind.RECENT)))
        assertNull("needs an id", HomeLayouts.add(l, shelf(ShelfKind.GENRE)))
        assertNull("takes no id", HomeLayouts.add(l, shelf(ShelfKind.RANDOM, 3)))
        assertNull("bad id", HomeLayouts.add(l, shelf(ShelfKind.MENU, 0)))
        assertNull("bad id", HomeLayouts.add(l, shelf(ShelfKind.MENU, 1_000_000_000_000)))
        // Hidden shelves are added visible.
        assertFalse(HomeLayouts.add(l, shelf(ShelfKind.RANDOM, hidden = true))!!.shelves.last().hidden)
        var full = HomeLayout(emptyList(), false)
        for (i in 1..HomeLayouts.MAX_SHELVES) full = HomeLayouts.add(full, shelf(ShelfKind.GENRE, i.toLong()))!!
        assertNull("full", HomeLayouts.add(full, shelf(ShelfKind.GENRE, 99)))
        assertEquals(
            listOf(ShelfKind.RANDOM, ShelfKind.FAVOURITES, ShelfKind.PLAYLISTS),
            HomeLayouts.addableSingles(l),
        )
    }

}

class HomeViewModelTest : MainDispatcherTest() {
    @Test
    fun loadsTheLayoutHistoryShelvesAndStations() = runTest(dispatcher) {
        val me = FakeMe()
        val vm = HomeFeedViewModel(FakeLibrary(), me = me, player = { PLAYER })
        val feed = vm.state.value
        assertEquals(HomeLayouts.DEFAULT, feed.layout)
        assertEquals(listOf(7L, 3L), (feed.history[ShelfKind.RECENT] as Loadable.Ready).value.map { it.id })
        assertEquals(listOf(3L), (feed.history[ShelfKind.MOST] as Loadable.Ready).value.map { it.id })
        assertEquals(3, (feed.stations as Loadable.Ready).value.size)
        assertEquals(listOf(PinnedMenu(9, "Some Plugin", null)), feed.menus)
    }

    @Test
    fun editModeReordersHidesAddsAndSaves() = runTest(dispatcher) {
        val me = FakeMe()
        val vm = HomeFeedViewModel(FakeLibrary(), me = me, player = { PLAYER })
        vm.edit()
        vm.moveDown(0)
        vm.moveUp(4)
        vm.setHidden(0, true)
        vm.add(shelf(ShelfKind.STATION, 2))
        vm.add(shelf(ShelfKind.STATION, 2))
        vm.remove(99)
        val draft = vm.state.value.draft!!
        assertEquals(
            listOf(ShelfKind.STATIONS, ShelfKind.RECENT, ShelfKind.NEW, ShelfKind.ON_THIS_DAY, ShelfKind.MOST, ShelfKind.STATION),
            draft.shelves.map { it.kind },
        )
        assertTrue(draft.shelves[0].hidden)
        vm.save()
        assertEquals(draft, me.saved.single())
        assertNull(vm.state.value.draft)
        assertEquals(draft.shelves, vm.state.value.layout!!.shelves)
    }

    @Test
    fun aFailedSaveKeepsTheEditAndSaysWhy() = runTest(dispatcher) {
        val me = FakeMe().apply { saveResult = ApiResult.Err(AppError.InvalidInput("no such station or pinned menu")) }
        val vm = HomeFeedViewModel(FakeLibrary(), me = me, player = { PLAYER })
        val messages = mutableListOf<HomeMessage>()
        backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.edit()
        vm.moveDown(0)
        vm.save()
        assertTrue(vm.state.value.draft != null)
        assertFalse(vm.state.value.saving)
        assertEquals(HomeLayouts.DEFAULT, vm.state.value.layout)
        assertTrue(messages.single() is HomeMessage.Failed)
        vm.cancelEdit()
        assertNull(vm.state.value.draft)
    }

    @Test
    fun resettingSavesTheDefault() = runTest(dispatcher) {
        val me = FakeMe().apply { layout = HomeLayouts.move(HomeLayouts.DEFAULT, 0, 1) }
        val vm = HomeFeedViewModel(FakeLibrary(), me = me, player = { PLAYER })
        vm.edit()
        vm.resetDraft()
        vm.save()
        assertTrue("a reset is a DELETE, not a PUT", me.saved.isEmpty())
        assertEquals(HomeLayouts.DEFAULT, vm.state.value.layout)
    }

    @Test
    fun stationsStartOnThePlayerAndBrokenOnesSaySo() = runTest(dispatcher) {
        val me = FakeMe()
        var player: String? = null
        val vm = HomeFeedViewModel(FakeLibrary(), me = me, player = { player })
        val messages = mutableListOf<HomeMessage>()
        backgroundScope.launch { vm.messages.collect { messages += it } }
        val fresh = me.stations[1]
        vm.startStation(fresh)
        assertEquals(HomeMessage.NoPlayer, messages.last())
        assertTrue(me.started.isEmpty())
        player = PLAYER
        vm.startStation(fresh)
        assertEquals(listOf(2L to PLAYER), me.started)
        assertEquals(2L, vm.state.value.activeStation)
        assertEquals(HomeMessage.StationStarted("Fresh"), messages.last())
        me.startResult = ApiResult.Ok(StationBatch.Broken("this menu item is no longer offered"))
        vm.startStation(me.stations[2])
        assertEquals(HomeMessage.StationBroken("this menu item is no longer offered"), messages.last())
        vm.createStation("The 1980s", StationSpec.Decade(1980u, true))
        assertEquals("The 1980s" to StationSpec.Decade(1980u, true), me.created.single())
        assertTrue((vm.state.value.stations as Loadable.Ready).value.any { it.id == 50L })
        vm.deleteStation(me.stations.first())
        assertFalse((vm.state.value.stations as Loadable.Ready).value.any { it.id == 1L })
    }

    @Test
    fun withoutTheGatewayHomeKeepsItsFixedCarousels() = runTest(dispatcher) {
        val vm = HomeFeedViewModel(FakeLibrary())
        assertNull(vm.state.value.layout)
        vm.edit()
        vm.save()
        assertTrue(vm.state.value.recent is Loadable.Ready)
    }
}

class MenuSavingTest : MainDispatcherTest() {
    private val info = action("trackinfo", "items")

    @Test
    fun stepsFollowTheStackAtThePageHoldingTheItem() {
        val root = MenuFrame(1, "Track", info)
        val sub = MenuFrame(2, "Mixes", action("someplugin", "items"))
        assertEquals(
            listOf(MenuStep.Opened(info, 0u, 500u), MenuStep.Opened(sub.action!!, 200u, 100u)),
            MenuNavigator.stepsTo(listOf(root, sub), 250),
        )
        assertEquals(listOf(MenuStep.Opened(info, 0u, 100u)), MenuNavigator.stepsTo(listOf(root), 3))
        // A menu taken from the home menu is the home menu again.
        val home = MenuFrame(3, "Random Mix", null, fromHomeMenu = true)
        assertEquals(listOf(MenuStep.HomeMenu), MenuNavigator.stepsTo(listOf(home), 0))
        // A menu an action answered with, a search, or too deep: can't be replayed.
        assertNull(MenuNavigator.stepsTo(listOf(root, MenuFrame(4, "Choices", null)), 0))
        assertNull(MenuNavigator.stepsTo(listOf(MenuFrame(5, "Search", info, input = "x")), 0))
        assertNull(MenuNavigator.stepsTo(List(5) { root }, 0))
        assertNull(MenuNavigator.stepsTo(emptyList(), 0))
        // A plugin's own menu from its home-menu entry: replayed from the home menu.
        val plugin = MenuFrame(6, "Discover Mix", action("discovermix", "menu", opens = true))
        assertEquals(
            listOf(MenuStep.HomeMenu, MenuStep.Opened(plugin.action!!, 0u, 100u)),
            MenuNavigator.stepsTo(listOf(plugin), 0, MenuStep.HomeMenu),
        )
        assertEquals(listOf(MenuStep.HomeMenu), MenuNavigator.stepsTo(listOf(home), 0, MenuStep.HomeMenu))
        assertNull("five steps", MenuNavigator.stepsTo(List(4) { plugin }, 0, MenuStep.HomeMenu))
    }

    @Test
    fun mixesSaveAsStationsAndMenusPinToHome() = runTest(dispatcher) {
        val mix = action("someplugin", "mix", opens = false, next = "nowPlaying")
        val menus = FakeMenus().apply {
            pages["trackinfo"] = listOf(
                menuItem("More like this", MenuItemKind.ACTION, go = mix),
                menuItem("Mixes", MenuItemKind.FOLDER, go = action("someplugin", "items")),
                menuItem("Search", MenuItemKind.SEARCH, go = action("someplugin", "items", input = true)),
                menuItem("Track", MenuItemKind.PLAYABLE, play = action("someplugin", "playlist", "play", opens = false)),
            )
        }
        val me = FakeMe()
        val n = MenuNavigator(backgroundScope, menus) { PLAYER }
        n.start("Track 1", info)
        val items = n.top!!.items
        assertNull("nowhere to save to", n.canSave(items[0]))
        n.saver = me
        assertEquals(SaveKind.STATION, n.canSave(items[0]))
        assertEquals(SaveKind.PIN, n.canSave(items[1]))
        assertNull("typed text is never saved", n.canSave(items[2]))
        assertNull("a track that offers no add isn't a station", n.canSave(items[3]))
        val events = mutableListOf<MenuEvent>()
        backgroundScope.launch { n.events.collect { events += it } }
        n.save(items[0], 0, SaveKind.STATION)
        assertEquals(Triple(listOf<MenuStep>(MenuStep.Opened(info, 0u, 100u)), mix, null), me.savedStations.single())
        assertEquals(MenuEvent.Saved(station = true), events.last())
        n.save(items[1], 1, SaveKind.PIN)
        assertEquals(
            listOf(MenuStep.Opened(info, 0u, 100u), MenuStep.Opened(items[1].go!!, 0u, 100u)),
            me.pinned.single(),
        )
        assertTrue("pinned menus go on Home", HomeLayouts.has(me.layout, ShelfKind.MENU, 9))
        me.saveStationResult = ApiResult.Err(AppError.InvalidInput("this menu item is no longer offered"))
        n.save(items[0], 0, SaveKind.STATION)
        assertEquals(MenuEvent.CantSave(AppError.InvalidInput("this menu item is no longer offered")), events.last())
    }
}

class HistoryViewModelTest : MainDispatcherTest() {
    @Test
    fun showsPlaysWithTheirTracksAndSwitchesOff() = runTest(dispatcher) {
        val me = FakeMe()
        val vm = HistoryViewModel(me)
        val s = vm.state.value
        assertEquals(true, s.enabled)
        assertEquals(listOf(2L, 1L), s.rows.map { it.track?.id })
        assertTrue(s.rows[1].entry.offline)
        vm.setEnabled(false)
        assertEquals(false, vm.state.value.enabled)
        assertFalse(me.historyEnabled)
        // A failure puts the switch back.
        me.enableResult = ApiResult.Err(AppError.Offline("down"))
        vm.setEnabled(true)
        assertEquals(false, vm.state.value.enabled)
        val messages = mutableListOf<HistoryMessage>()
        backgroundScope.launch { vm.messages.collect { messages += it } }
        vm.clear()
        assertEquals(1, me.cleared)
        assertTrue(vm.state.value.rows.isEmpty())
        assertEquals(HistoryMessage.Cleared, messages.last())
    }
}

@RunWith(AndroidJUnit4::class)
@Config(sdk = [35], qualifiers = "w420dp-h1800dp")
class HomeScreenTests {
    @get:Rule
    val compose = createComposeRule()

    private val noActions = HomeActions({}, {}, {}, {}, {}, {}, {})

    private fun feed(draft: HomeLayout? = null) = HomeFeed(
        recent = Loadable.Ready(listOf(sampleAlbum(1))),
        layout = HomeLayouts.DEFAULT,
        history = mapOf(ShelfKind.RECENT to Loadable.Ready(listOf(sampleAlbum(7))), ShelfKind.MOST to Loadable.Ready(listOf(sampleAlbum(3)))),
        // The broken one first: a row of cards shows only the first two at this width.
        stations = Loadable.Ready(FakeMe().stations.reversed()),
        draft = draft,
    )

    @Test
    fun shelvesFollowTheLayoutAndStationsStart() {
        val started = mutableListOf<Station>()
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                HomeScreen(feed(), noActions.copy(onStation = { started += it }), edit = editActions())
            }
        }
        compose.onNodeWithText("Recently played").assertExists()
        compose.onNodeWithText("Stations").assertExists()
        compose.onNodeWithText("Fresh").performClick()
        assertEquals("Fresh", started.single().name)
        // A broken station says why.
        compose.onNodeWithText("this menu item is no longer offered").assertExists()
        compose.onNodeWithText("Edit Home").performScrollTo().assertExists()
    }

    private val moves = mutableListOf<Pair<Int, Int>>()
    private val hides = mutableListOf<Pair<Int, Boolean>>()
    private var saves = 0

    private fun editActions() = HomeEditActions(
        onMove = { a, b -> moves += a to b },
        onHidden = { i, h -> hides += i to h },
        onRemove = {},
        onAdd = {},
        onReset = {},
        onSave = { saves++ },
        onCancel = {},
    )

    @Test
    fun editModeMovesWithButtonsAndAccessibilityActions() {
        val draft = HomeLayouts.add(HomeLayouts.DEFAULT, shelf(ShelfKind.STATION, 2))!!
        compose.setContent {
            LyrionTheme(dynamicColour = false) { HomeScreen(feed(draft), noActions, edit = editActions()) }
        }
        compose.onNodeWithContentDescription("Move up: Recently played").assertIsNotEnabled()
        compose.onNodeWithContentDescription("Move down: Recently played").performClick()
        compose.onNodeWithContentDescription("Move up: Most played").performClick()
        assertEquals(listOf(0 to 1, 3 to 2), moves)
        compose.onNodeWithContentDescription("Hide: Stations").performClick()
        assertEquals(listOf(1 to true), hides)
        // Station shelves are named after the station, and can be removed.
        compose.onNodeWithContentDescription("Remove: Station: Fresh").assertExists()
        // The same moves as accessibility actions (TalkBack), without dragging.
        compose.onNodeWithContentDescription("Drag to reorder: Recently added").assertExists()
        compose.onNode(
            SemanticsMatcher("has Move down for Stations") { node ->
                SemanticsActions.CustomActions in node.config &&
                    node.config[SemanticsActions.CustomActions].any { it.label == "Move down: Stations" }
            },
        ).assertExists()
        compose.onNodeWithText("Save").performClick()
        assertEquals(1, saves)
    }

    @Test
    fun theHomeAndHistoryAreAccessible() {
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                androidx.compose.foundation.layout.Column {
                    HomeScreen(feed(HomeLayouts.DEFAULT), noActions, edit = editActions())
                }
            }
        }
        compose.assertAccessible()
    }

    @Test
    fun historySettingsAndListAreAccessible() {
        val state = HistoryUiState(
            enabled = true,
            rows = FakeMe().history.map { io.github.jamezuk.lyrionanywhere.ui.settings.HistoryRow(it, sampleTrack(it.trackId)) },
            loaded = true,
        )
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                androidx.compose.foundation.layout.Column {
                    HistorySettings(state, onEnabled = {}, onView = {}, onClear = {})
                    HistoryScreen(state, onMore = {}, onRetry = {})
                }
            }
        }
        compose.assertAccessible()
        compose.onNodeWithText("Clear history").performClick()
        compose.onNodeWithText("Clear your listening history?").assertExists()
    }
}
