package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.AppEntry
import io.github.jamezuk.lyrionanywhere.core.InfoKind
import io.github.jamezuk.lyrionanywhere.core.MenuAction
import io.github.jamezuk.lyrionanywhere.core.MenuItem
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.MenuOutcome
import io.github.jamezuk.lyrionanywhere.core.MenuPage
import io.github.jamezuk.lyrionanywhere.core.MenuParam
import io.github.jamezuk.lyrionanywhere.core.PluginEntry
import io.github.jamezuk.lyrionanywhere.core.SearchableApp
import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.MenusRepository

fun action(vararg cmd: String, opens: Boolean = cmd.last() == "items", next: String? = null, input: Boolean = false) =
    MenuAction(
        cmd.toList(),
        if (input) listOf(MenuParam("search", "__TAGGEDINPUT__")) else emptyList(),
        next,
        opens,
        input,
    )

fun menuItem(
    text: String,
    kind: MenuItemKind,
    go: MenuAction? = null,
    play: MenuAction? = null,
    add: MenuAction? = null,
    next: MenuAction? = null,
) = MenuItem(text, kind, null, go, play, add, next, null)

/** Menus: each opened action answers [pages] (by its first word), a page at a time. */
class FakeMenus : MenusRepository {
    val opened = mutableListOf<Triple<String, List<String>, Int>>()
    val inputs = mutableListOf<String?>()
    val ran = mutableListOf<List<String>>()
    var pages = mutableMapOf<String, List<MenuItem>>()
    var runResult: ApiResult<MenuOutcome> = ApiResult.Ok(MenuOutcome.Done(null))
    var apps = listOf(AppEntry("someapp", "Some App", null, action("someapp", "items")))
    var radios = listOf(AppEntry("someradio", "Some Radio", null, action("someradio", "items")))
    var plugins = listOf(PluginEntry("pluginX", "Plugin X", null, 100, action("x", "items"), emptyList()))
    var fits = mutableMapOf<String, Suitability>()
    var probes = 0
    var failApps: AppError? = null

    override suspend fun apps() = failApps?.let { ApiResult.Err(it) } ?: ApiResult.Ok(apps)

    override suspend fun radios() = failApps?.let { ApiResult.Err(it) } ?: ApiResult.Ok(radios)

    override suspend fun plugins(player: String, shownApps: List<String>) =
        ApiResult.Ok(plugins.filter { p -> p.go?.cmd?.first() !in shownApps })

    override suspend fun open(player: String, action: MenuAction, start: Int, count: Int, input: String?): ApiResult<MenuPage> {
        opened += Triple(player, action.cmd, start)
        inputs += input
        val all = pages[action.cmd.first()].orEmpty()
        val page = all.drop(start).take(count)
        return ApiResult.Ok(MenuPage(null, all.size.toUInt(), page))
    }

    override suspend fun run(player: String, action: MenuAction, input: String?): ApiResult<MenuOutcome> {
        ran += action.cmd
        return runResult
    }

    override suspend fun probe(player: String, action: MenuAction): Suitability {
        probes++
        return fits[action.cmd.first()] ?: Suitability.Usable
    }

    /** Each `searchable` call: the player and the apps asked about. */
    val searchableCalls = mutableListOf<Pair<String, List<String>>>()

    /** As the Rust layer: an app whose top menu has a search box for `search:` text. */
    override suspend fun searchable(player: String, apps: List<AppEntry>): ApiResult<List<SearchableApp>> {
        searchableCalls += player to apps.map { it.cmd }
        return ApiResult.Ok(
            apps.mapNotNull { a ->
                val box = pages[a.cmd].orEmpty().firstOrNull { i ->
                    i.kind == MenuItemKind.SEARCH && i.go?.params?.singleOrNull { p -> p.value == "__TAGGEDINPUT__" }?.key == "search"
                }?.go ?: return@mapNotNull null
                SearchableApp(a.cmd, a.name, a.icon, box)
            },
        )
    }

    override fun appMenu(cmd: String): ApiResult<MenuAction> = ApiResult.Ok(action(cmd, "items"))

    override fun infoMenu(kind: InfoKind, id: Long): ApiResult<MenuAction> =
        if (id > 0) ApiResult.Ok(action("trackinfo", "items")) else ApiResult.Err(AppError.InvalidInput("bad id"))
}
