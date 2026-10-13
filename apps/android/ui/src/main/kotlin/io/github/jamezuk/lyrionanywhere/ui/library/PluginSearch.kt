package io.github.jamezuk.lyrionanywhere.ui.library

import io.github.jamezuk.lyrionanywhere.core.AppEntry
import io.github.jamezuk.lyrionanywhere.core.SearchableApp
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.MenusRepository
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Which apps can be searched ("Search on …", Wave 2 §3.3), as the web app finds them: an app
 * or radio service qualifies when its own top menu has a search box taking the text as its
 * `search:` value (decided in the Rust layer; nothing is hard-coded per app). Found once per
 * session and looked at again only when the apps listed change: installed, removed, or left
 * out for this account by the gateway (plugins reserved for admins), so a listener is never
 * offered one.
 */
class SearchDirectory(private val menus: MenusRepository) {
    private val lock = Mutex()
    private var signature: String? = null
    private var list: List<SearchableApp> = emptyList()

    /** The searchable apps (the apps lists are read each time; their menus only when changed). */
    suspend fun get(player: String): List<SearchableApp> = lock.withLock {
        val apps = menus.apps()
        val radios = menus.radios()
        val listed = ((apps as? ApiResult.Ok)?.value.orEmpty() + (radios as? ApiResult.Ok)?.value.orEmpty())
            .distinctBy { it.cmd }
            .take(MAX_APPS)
        // Not even the apps list: nothing to offer, and nothing remembered.
        if (apps is ApiResult.Err && radios is ApiResult.Err) return@withLock emptyList()
        val sig = listed.joinToString(",") { it.cmd }
        if (sig == signature) return@withLock list
        val found = when (val r = menus.searchable(player, listed)) {
            is ApiResult.Ok -> r.value
            is ApiResult.Err -> return@withLock emptyList()
        }
        signature = sig
        list = found
        found
    }

    /**
     * One app's search box, read again from its top menu now (so it is what LMS offers at
     * this moment, and the gateway has just seen it offered); null if the app isn't listed
     * for this account or no longer has one.
     */
    suspend fun find(player: String, cmd: String): SearchableApp? {
        val known = lock.withLock { list.firstOrNull { it.cmd == cmd } } ?: get(player).firstOrNull { it.cmd == cmd } ?: return null
        val menu = (menus.appMenu(known.cmd) as? ApiResult.Ok)?.value ?: return null
        val again = menus.searchable(player, listOf(AppEntry(known.cmd, known.name, known.icon, menu)))
        return (again as? ApiResult.Ok)?.value?.firstOrNull()
    }

    companion object {
        /** Apps looked at, at most (as the Rust layer). */
        const val MAX_APPS = 64
    }
}
