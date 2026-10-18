package io.github.jamezuk.lyrionanywhere.ui.menus

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.AppEntry
import io.github.jamezuk.lyrionanywhere.core.MenuAction
import io.github.jamezuk.lyrionanywhere.core.MenuItem
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.MenuStep
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.MenuOutcome
import io.github.jamezuk.lyrionanywhere.core.PluginEntry
import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import io.github.jamezuk.lyrionanywhere.ui.library.SearchDirectory
import io.github.jamezuk.lyrionanywhere.data.MenusRepository
import io.github.jamezuk.lyrionanywhere.data.SuitabilityCache
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit

/** What the app calls a menu action whose item has no text of its own (worded by the screen). */
enum class MenuVerb { PLAY, OPEN, SEARCH, PLAY_NEXT, ADD, MORE }

/** What a menu action is called: its item's own text, or a [MenuVerb] the screen words. */
sealed interface MenuLabel {
    data class Named(val text: String) : MenuLabel

    data class Verb(val verb: MenuVerb) : MenuLabel
}

/** One menu on the navigator's stack. */
data class MenuFrame(
    val id: Int,
    /** LMS's title, or the item's text; empty when neither has one (see [titleVerb]). */
    val title: String,
    /** The action (and typed text) that pages this menu, or null for a fixed one. */
    val action: MenuAction?,
    val input: String? = null,
    val items: List<MenuItem> = emptyList(),
    val total: Int = 0,
    val loading: Boolean = false,
    val error: AppError? = null,
    /** A fixed menu taken from the player's home menu (plugin entries), which can be read again. */
    val fromHomeMenu: Boolean = false,
    /** What to call it when [title] is empty (the action that opened it). */
    val titleVerb: MenuVerb? = null,
) {
    val canLoadMore: Boolean get() = action != null && !loading && error == null && items.size < total
}

/** What happened, for a snackbar or navigation. */
sealed interface MenuEvent {
    /** An action ran ("Play next: done"). */
    data class Done(val label: MenuLabel) : MenuEvent

    /** LMS suggests showing what's playing. */
    data object NowPlaying : MenuEvent

    data class Failed(val error: AppError) : MenuEvent

    /** Menus browse and play for a player; none is chosen. */
    data object NoPlayer : MenuEvent

    /** Backed out of the first menu. */
    data object Closed : MenuEvent

    /** Saved as a station ([station]) or pinned to Home. */
    data class Saved(val station: Boolean) : MenuEvent

    /** The server wouldn't save it (the reason is fit to show). */
    data class CantSave(val error: AppError) : MenuEvent
}

/** What can be kept from a menu item. */
enum class SaveKind {
    /** A mix or generator: "Save as station". */
    STATION,

    /** A menu: "Pin to Home". */
    PIN,
}

/**
 * Browses LMS menus (apps, radio, plugins, "More…"), with its own back stack, as the web
 * app's MenuView does. It runs only actions the menus offered; the gateway enforces the
 * same. [player] is whichever player commands go to (D20: decided elsewhere).
 */
class MenuNavigator(
    private val scope: CoroutineScope,
    private val menus: MenusRepository,
    private val player: () -> String?,
) {
    private val _stack = MutableStateFlow<List<MenuFrame>>(emptyList())
    val stack: StateFlow<List<MenuFrame>> = _stack.asStateFlow()
    private val _events = Channel<MenuEvent>(Channel.BUFFERED)
    val events: Flow<MenuEvent> = _events.receiveAsFlow()
    private var nextId = 0

    val top: MenuFrame? get() = _stack.value.lastOrNull()

    /** Start over with a menu opened by [action]. */
    fun start(title: String, action: MenuAction) {
        _stack.value = emptyList()
        push(title, action, null)
    }

    /**
     * Start over with an app's results for [text], from its search box [action] ("Search on").
     * The text is sent as typed text only: the action fills it into its one `search:` value.
     */
    fun startSearch(title: String, action: MenuAction, text: String) {
        val t = text.trim()
        if (t.isEmpty() || !action.opensMenu || !action.needsInput) return
        _stack.value = emptyList()
        push(title, action, t)
    }

    /**
     * Start over with a menu already in hand (e.g. a group of plugin entries); [fromHomeMenu]
     * when it was taken from the player's home menu (so it can be saved from).
     */
    fun startWith(title: String, items: List<MenuItem>, fromHomeMenu: Boolean = false) {
        _stack.value = listOf(MenuFrame(nextId++, title, null, items = items, total = items.size, fromHomeMenu = fromHomeMenu))
    }

    /** Where stations and pinned menus are saved (none: the options aren't offered). */
    var saver: MeRepository? = null

    /**
     * The menu the first one was taken from, when it isn't one the gateway takes as a start
     * by itself (a plugin's entry in the home menu: [MenuStep.HomeMenu]).
     */
    var root: MenuStep? = null

    /** What [item] (on the top menu) can be kept as, if anything. */
    fun canSave(item: MenuItem): SaveKind? {
        if (saver == null || stepsTo(_stack.value, 0, root) == null) return null
        val go = item.go
        val play = item.play
        return when {
            item.kind == MenuItemKind.FOLDER && go != null && go.opensMenu && !go.needsInput -> SaveKind.PIN
            item.kind == MenuItemKind.ACTION && go != null && !go.opensMenu && !go.needsInput -> SaveKind.STATION
            item.kind == MenuItemKind.PLAYABLE && play != null && !play.opensMenu && !play.needsInput &&
                item.add != null -> SaveKind.STATION
            else -> null
        }
    }

    /** Save [item] (at [index] on the top menu) as [kind]: the server checks LMS offers it now. */
    fun save(item: MenuItem, index: Int, kind: SaveKind) {
        val me = saver ?: return
        val p = player()
        if (p == null) {
            _events.trySend(MenuEvent.NoPlayer)
            return
        }
        val steps = stepsTo(_stack.value, index, root) ?: return
        val name = item.text.lineSequence().firstOrNull().orEmpty().trim().ifEmpty { top?.title.orEmpty() }.take(64)
        scope.launch {
            val r = when (kind) {
                SaveKind.STATION -> {
                    val play = (if (item.kind == MenuItemKind.ACTION) item.go else item.play) ?: return@launch
                    me.saveMenuStation(name, steps, play, item.add?.takeIf { !it.opensMenu && !it.needsInput }, p)
                }
                SaveKind.PIN -> {
                    val go = item.go ?: return@launch
                    if (steps.size >= MAX_STEPS) {
                        _events.trySend(MenuEvent.CantSave(AppError.Reason.MENU_TOO_DEEP.error()))
                        return@launch
                    }
                    val pinned = me.pinMenu(name, steps + MenuStep.Opened(go, 0u, PAGE.toUInt()), p)
                    // And put it on Home (best effort: it is listed under Add a shelf anyway).
                    if (pinned is ApiResult.Ok) {
                        (me.home() as? ApiResult.Ok)?.value?.let { home ->
                            HomeLayouts.add(home, HomeShelf(ShelfKind.MENU, pinned.value, false))?.let { me.saveHome(it) }
                        }
                    }
                    pinned
                }
            }
            when (r) {
                is ApiResult.Ok -> _events.trySend(MenuEvent.Saved(kind == SaveKind.STATION))
                is ApiResult.Err -> _events.trySend(MenuEvent.CantSave(r.error))
            }
        }
    }

    private fun push(title: String, action: MenuAction?, input: String?, items: List<MenuItem>? = null, titleVerb: MenuVerb? = null) {
        val f = MenuFrame(
            nextId++,
            title,
            action,
            input,
            items = items.orEmpty(),
            total = items?.size ?: 0,
            loading = items == null,
            titleVerb = titleVerb.takeIf { title.isEmpty() },
        )
        _stack.update { it + f }
        if (items == null) load(f.id, 0)
    }

    private fun updateFrame(id: Int, f: (MenuFrame) -> MenuFrame) =
        _stack.update { list -> list.map { if (it.id == id) f(it) else it } }

    private fun load(id: Int, from: Int) {
        val frame = _stack.value.firstOrNull { it.id == id } ?: return
        val action = frame.action ?: return
        val p = player()
        if (p == null) {
            updateFrame(id) { it.copy(loading = false) }
            _events.trySend(MenuEvent.NoPlayer)
            return
        }
        updateFrame(id) { it.copy(loading = true, error = null) }
        scope.launch {
            when (val r = menus.open(p, action, from, PAGE, frame.input)) {
                is ApiResult.Ok -> updateFrame(id) {
                    val items = if (from == 0) r.value.items else it.items + r.value.items
                    // A page with nothing new ends the menu.
                    val total = if (r.value.items.isEmpty()) items.size else maxOf(r.value.total.toInt(), items.size)
                    it.copy(
                        items = items,
                        total = total,
                        loading = false,
                        title = it.title.ifEmpty { r.value.title.orEmpty() },
                    )
                }
                is ApiResult.Err -> updateFrame(id) { it.copy(loading = false, error = r.error) }
            }
        }
    }

    fun loadMore() {
        val f = top ?: return
        if (f.canLoadMore) load(f.id, f.items.size)
    }

    fun retry() {
        val f = top ?: return
        load(f.id, f.items.size)
    }

    /** Back one menu; false (and [MenuEvent.Closed]) at the first. */
    fun back(): Boolean {
        if (_stack.value.size > 1) {
            _stack.update { it.dropLast(1) }
            return true
        }
        _events.trySend(MenuEvent.Closed)
        return false
    }

    /** The item's main action: play what plays, open what opens. */
    fun choose(item: MenuItem) {
        val play = item.play
        val go = item.go
        when {
            item.kind == MenuItemKind.PLAYABLE && play != null -> run(item, play, MenuLabel.Verb(MenuVerb.PLAY), null)
            go != null -> run(item, go, labelFor(item, MenuVerb.OPEN), null)
        }
    }

    fun act(item: MenuItem, action: MenuAction, verb: MenuVerb) = run(item, action, MenuLabel.Verb(verb), null)

    private fun labelFor(item: MenuItem, verb: MenuVerb): MenuLabel =
        if (item.text.isEmpty()) MenuLabel.Verb(verb) else MenuLabel.Named(item.text)

    /** Search with [text] in a search item. */
    fun search(item: MenuItem, text: String) {
        val go = item.go ?: return
        val t = text.trim()
        if (t.isNotEmpty()) run(item, go, labelFor(item, MenuVerb.SEARCH), t)
    }

    private fun run(item: MenuItem, action: MenuAction, label: MenuLabel, input: String?) {
        if (action.opensMenu) {
            push(item.text, action, input, titleVerb = (label as? MenuLabel.Verb)?.verb)
            return
        }
        val p = player()
        if (p == null) {
            _events.trySend(MenuEvent.NoPlayer)
            return
        }
        scope.launch {
            when (val r = menus.run(p, action, input)) {
                is ApiResult.Ok -> when (val o = r.value) {
                    // An action that answers with a menu (a list of choices).
                    is MenuOutcome.Menu -> push(o.page.title ?: item.text, null, null, o.page.items, (label as? MenuLabel.Verb)?.verb)
                    is MenuOutcome.Done -> {
                        _events.trySend(MenuEvent.Done(label))
                        when (o.nextWindow) {
                            "nowPlaying" -> _events.trySend(MenuEvent.NowPlaying)
                            "parent", "grandparent" -> back()
                            "refresh" -> top?.let { load(it.id, 0) }
                        }
                    }
                }
                is ApiResult.Err -> _events.trySend(MenuEvent.Failed(r.error))
            }
        }
    }

    companion object {
        const val PAGE = 100

        /** Menu requests from the first menu to an item the gateway takes, at most. */
        const val MAX_STEPS = 4

        /** Menus above the top one are read in one go (an item opened from them must be in it). */
        private const val WHOLE = 500

        /**
         * The menu requests from the first menu to the top one, as the gateway replays them
         * (`/v1/me/stations`, `/v1/me/menus`): each paged menu at the page holding what was
         * chosen in it ([index] on the top one), a menu from the home menu as the home menu.
         * Null if one can't be read again (a menu an action answered with), or too deep.
         */
        fun stepsTo(stack: List<MenuFrame>, index: Int, root: MenuStep? = null): List<MenuStep>? {
            if (stack.isEmpty()) return null
            val out = mutableListOf<MenuStep>()
            root?.let { out += it }
            stack.forEachIndexed { i, f ->
                val action = f.action
                when {
                    action != null && f.input == null -> {
                        val last = i == stack.size - 1
                        val start = if (last) (index.coerceAtLeast(0) / PAGE) * PAGE else 0
                        out += MenuStep.Opened(action, start.toUInt(), (if (last) PAGE else WHOLE).toUInt())
                    }
                    // A menu taken from the home menu: the root (or the home menu itself).
                    i == 0 && (root != null || f.fromHomeMenu) -> if (root == null) out += MenuStep.HomeMenu
                    else -> return null
                }
            }
            return out.takeIf { it.size <= MAX_STEPS }
        }
    }
}

/** A menu screen (an app, a plugin): a [MenuNavigator] that survives configuration changes. */
class MenuViewModel(
    private val menus: MenusRepository,
    /** Where "Save as station" and "Pin to Home" save (none: not offered). */
    private val me: MeRepository? = null,
    player: () -> String?,
) : ViewModel() {
    val navigator = MenuNavigator(viewModelScope, menus, player).also { it.saver = me }
    private val _missing = MutableStateFlow(false)

    /** The plugin isn't in the music server's menu any more. */
    val missing: StateFlow<Boolean> = _missing.asStateFlow()
    private var started = false

    fun openApp(cmd: String, name: String) {
        if (started) return
        started = true
        when (val a = menus.appMenu(cmd)) {
            is ApiResult.Ok -> navigator.start(name, a.value)
            is ApiResult.Err -> _missing.value = true
        }
    }

    /** A menu pinned to Home, as LMS offers it now. */
    fun openPinned(id: Long, name: String, player: String?) {
        val m = me ?: return
        if (started || player == null) return
        started = true
        viewModelScope.launch {
            when (val r = m.openMenu(id, player)) {
                is ApiResult.Ok -> navigator.startWith(name, r.value.items)
                is ApiResult.Err -> _missing.value = true
            }
        }
    }

    /**
     * An app's own results for [query] ("Search on" from Search): its search box is read
     * again from its top menu first, so the search runs through what LMS offers now.
     */
    fun openSearch(directory: SearchDirectory, cmd: String, title: String, query: String, player: String?) {
        if (started || player == null) return
        started = true
        val q = query.trim()
        viewModelScope.launch {
            val app = if (q.isEmpty()) null else directory.find(player, cmd)
            if (app == null) _missing.value = true else navigator.startSearch(title, app.search, q)
        }
    }

    /** A plugin entry from the home menu, found again by its id (as the web app does). */
    fun openPlugin(id: String, player: String?) {
        if (started || player == null) return
        started = true
        viewModelScope.launch {
            val entry = (menus.plugins(player, emptyList()) as? ApiResult.Ok)?.value?.firstOrNull { it.id == id }
            // Saved from here, the gateway replays it from the home menu it came from.
            navigator.root = MenuStep.HomeMenu
            when {
                entry == null -> _missing.value = true
                entry.go != null -> navigator.start(entry.text, entry.go!!)
                else -> navigator.startWith(entry.text, entry.children, fromHomeMenu = true)
            }
        }
    }
}

/** An app, radio service or plugin on the Apps & Radio page. */
data class AppTile(
    /** "app:<cmd>" or "plugin:<id>". */
    val key: String,
    val name: String,
    val icon: String?,
    val fit: Suitability? = null,
) {
    /** Hidden unless "Show all": it shows nothing here, or couldn't be opened. */
    val unusable: Boolean get() = fit is Suitability.Empty || fit is Suitability.Failed
}

data class AppsUiState(
    val radios: List<AppTile> = emptyList(),
    val apps: List<AppTile> = emptyList(),
    val plugins: List<AppTile> = emptyList(),
    val loading: Boolean = true,
    val error: AppError? = null,
    val showAll: Boolean = false,
    val noPlayer: Boolean = false,
) {
    fun shown(list: List<AppTile>): List<AppTile> = if (showAll) list else list.filterNot { it.unusable }

    val hidden: Int get() = (radios + apps + plugins).count { it.unusable }

    val empty: Boolean get() = !loading && error == null && radios.isEmpty() && apps.isEmpty() && plugins.isEmpty()
}

/**
 * Apps & Radio: LMS's apps and radio services, and plugins from the player's home menu.
 * Each is judged from its own top menu (a few at a time, remembered for a while), so ones
 * that are broken or empty here are tucked away and ones needing setup in LMS say so.
 */
class AppsViewModel(
    private val menus: MenusRepository,
    private val player: () -> String?,
    private val cache: SuitabilityCache,
) : ViewModel() {
    private val _state = MutableStateFlow(AppsUiState())
    val state: StateFlow<AppsUiState> = _state.asStateFlow()
    private val actions = mutableMapOf<String, MenuAction>()

    init {
        load()
    }

    fun load() {
        _state.update { it.copy(loading = true, error = null) }
        viewModelScope.launch {
            val apps = menus.apps()
            val radios = menus.radios()
            val err = (apps as? ApiResult.Err)?.error ?: (radios as? ApiResult.Err)?.error
            val a = (apps as? ApiResult.Ok)?.value.orEmpty()
            val r = (radios as? ApiResult.Ok)?.value.orEmpty()
            val p = player()
            // Plugins need a player; not essential (apps and radio still work).
            val plugins = if (p == null) emptyList() else (menus.plugins(p, (a + r).map { it.cmd }) as? ApiResult.Ok)?.value.orEmpty()
            actions.clear()
            fun tile(e: AppEntry): AppTile {
                actions["app:${e.cmd}"] = e.menu
                return AppTile("app:${e.cmd}", e.name, e.icon)
            }
            fun tile(e: PluginEntry): AppTile {
                e.go?.let { actions["plugin:${e.id}"] = it }
                return AppTile("plugin:${e.id}", e.text, e.icon)
            }
            // One tile per command: LMS may list the same one under radio and apps (the core
            // removes repeats only within each list), and a repeated key crashes the grid.
            val radioTiles = r.map(::tile).distinctBy { it.key }
            val appTiles = a.map(::tile).distinctBy { it.key }.filterNot { t -> radioTiles.any { it.key == t.key } }
            val shownKeys = (radioTiles + appTiles).map { it.key }.toSet()
            _state.value = AppsUiState(
                radios = radioTiles,
                apps = appTiles,
                plugins = plugins.map(::tile).distinctBy { it.key }.filterNot { it.key in shownKeys },
                loading = false,
                error = if (a.isEmpty() && r.isEmpty()) err else null,
                showAll = _state.value.showAll,
                noPlayer = p == null,
            )
            if (p != null) probe(p)
        }
    }

    private fun probe(p: String) {
        val gate = Semaphore(CONCURRENCY)
        for ((key, action) in actions.toMap()) {
            viewModelScope.launch {
                val cacheKey = "$p|$key"
                val fit = cache.get(cacheKey) ?: gate.withPermit { menus.probe(p, action) }.also { cache.put(cacheKey, it) }
                _state.update { s ->
                    fun List<AppTile>.with() = map { if (it.key == key) it.copy(fit = fit) else it }
                    s.copy(radios = s.radios.with(), apps = s.apps.with(), plugins = s.plugins.with())
                }
            }
        }
    }

    fun toggleShowAll() = _state.update { it.copy(showAll = !it.showAll) }

    companion object {
        /** Menus probed at once (LMS is single-threaded). */
        const val CONCURRENCY = 3
    }
}
