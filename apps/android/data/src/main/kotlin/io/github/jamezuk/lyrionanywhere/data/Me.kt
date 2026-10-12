package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.HistoryPage
import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.MenuAction
import io.github.jamezuk.lyrionanywhere.core.MenuPage
import io.github.jamezuk.lyrionanywhere.core.MenuStep
import io.github.jamezuk.lyrionanywhere.core.PinnedMenu
import io.github.jamezuk.lyrionanywhere.core.RunnerUpdate
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationBatch
import io.github.jamezuk.lyrionanywhere.core.StationSpec
import io.github.jamezuk.lyrionanywhere.core.Track
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The signed-in user's own things on the gateway (docs/plans/wave1-design.md §6–8): listening
 * history, the customisable Home, stations and pinned menus, and keeping a station going.
 * Everything is per user and the same on every device.
 */
interface MeRepository {
    suspend fun history(limit: Int = 50, before: String? = null): ApiResult<HistoryPage>

    suspend fun clearHistory(): ApiResult<Long>

    suspend fun setHistoryEnabled(enabled: Boolean): ApiResult<Boolean>

    /** Album ids for [kind] (recently played, most played, on this day), best first. */
    suspend fun shelfAlbums(kind: ShelfKind, limit: Int = 20): ApiResult<List<Long>>

    /** Albums and tracks by id, in the order given (shelves, history). */
    suspend fun albums(ids: List<Long>): ApiResult<List<Album>>

    suspend fun tracks(ids: List<Long>): ApiResult<List<Track>>

    suspend fun home(): ApiResult<HomeLayout>

    suspend fun saveHome(layout: HomeLayout): ApiResult<HomeLayout>

    suspend fun resetHome(): ApiResult<HomeLayout>

    suspend fun stations(): ApiResult<List<Station>>

    suspend fun createStation(name: String, spec: StationSpec): ApiResult<Long>

    suspend fun deleteStation(id: Long): ApiResult<Unit>

    /** "Save as station" on a menu item: the menus that led to it, and its actions. */
    suspend fun saveMenuStation(
        name: String,
        steps: List<MenuStep>,
        play: MenuAction,
        add: MenuAction?,
        player: String,
    ): ApiResult<Long>

    suspend fun menus(): ApiResult<List<PinnedMenu>>

    /** "Pin to Home" for a menu: the menus that lead to it (the last is the menu itself). */
    suspend fun pinMenu(name: String, steps: List<MenuStep>, player: String): ApiResult<Long>

    suspend fun deleteMenu(id: Long): ApiResult<Unit>

    suspend fun openMenu(id: Long, player: String): ApiResult<MenuPage>

    /** The station kept going, if any. */
    val activeStation: StateFlow<Long?>

    /** Start a station on [player] (replacing its queue). */
    suspend fun startStation(id: Long, player: String): ApiResult<StationBatch>

    /**
     * A status update for [player]: [tracks] queued, [current] playing ([playing] its id),
     * [queued] their ids. Adds a batch when the station runs low; ends the station when
     * something else plays.
     */
    suspend fun onStatus(player: String, tracks: Int, current: Int, playing: Long?, queued: List<Long>): RunnerUpdate

    /** Stop keeping the station going (the user chose something else). */
    fun stopStation()
}

private fun Int.u(): UInt = coerceAtLeast(0).toUInt()

/** The Rust [io.github.jamezuk.lyrionanywhere.core.Me] and station runner for the signed-in server. */
class RustMeRepository(private val source: ConnectionSource) : MeRepository {
    private suspend fun <T> withMe(block: suspend (Connection) -> T): ApiResult<T> {
        val c = source.connection.value ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(c) }
    }

    private val _active = MutableStateFlow<Long?>(null)
    override val activeStation: StateFlow<Long?> = _active.asStateFlow()

    override suspend fun history(limit: Int, before: String?) = withMe { it.me.history(limit.u(), before) }

    override suspend fun clearHistory() = withMe { it.me.clearHistory().toLong() }

    override suspend fun setHistoryEnabled(enabled: Boolean) = withMe { it.me.setHistoryEnabled(enabled) }

    override suspend fun shelfAlbums(kind: ShelfKind, limit: Int) = withMe { it.me.shelfAlbums(kind, limit.u()) }

    override suspend fun albums(ids: List<Long>) =
        if (ids.isEmpty()) ApiResult.Ok(emptyList()) else withMe { it.library.albumsById(ids) }

    override suspend fun tracks(ids: List<Long>) =
        if (ids.isEmpty()) ApiResult.Ok(emptyList()) else withMe { it.library.tracksById(ids) }

    override suspend fun home() = withMe { it.me.home() }

    override suspend fun saveHome(layout: HomeLayout) = withMe { it.me.saveHome(layout) }

    override suspend fun resetHome() = withMe { it.me.resetHome() }

    override suspend fun stations() = withMe { it.me.stations() }

    override suspend fun createStation(name: String, spec: StationSpec) = withMe { it.me.createStation(name, spec) }

    override suspend fun deleteStation(id: Long) = withMe { it.me.deleteStation(id) }

    override suspend fun saveMenuStation(name: String, steps: List<MenuStep>, play: MenuAction, add: MenuAction?, player: String) =
        withMe { it.me.saveMenuStation(name, steps, play, add, player) }

    override suspend fun menus() = withMe { it.me.menus() }

    override suspend fun pinMenu(name: String, steps: List<MenuStep>, player: String) =
        withMe { it.me.pinMenu(name, steps, player) }

    override suspend fun deleteMenu(id: Long) = withMe { it.me.deleteMenu(id) }

    override suspend fun openMenu(id: Long, player: String) = withMe { it.me.openMenu(id, player) }

    override suspend fun startStation(id: Long, player: String): ApiResult<StationBatch> {
        val r = withMe { it.stations.start(id, player) }
        _active.value = when ((r as? ApiResult.Ok)?.value) {
            null, is StationBatch.Broken -> null
            else -> id
        }
        return r
    }

    override suspend fun onStatus(player: String, tracks: Int, current: Int, playing: Long?, queued: List<Long>): RunnerUpdate {
        val c = source.connection.value ?: return RunnerUpdate.Idle
        if (_active.value == null) return RunnerUpdate.Idle
        val update = try {
            c.stations.onStatus(player, tracks.u(), current.u(), playing, queued)
        } catch (e: Exception) {
            e.toAppError()
            RunnerUpdate.Idle
        }
        if (update is RunnerUpdate.Stopped || update is RunnerUpdate.Ended) _active.value = null
        return update
    }

    override fun stopStation() {
        source.connection.value?.stations?.stop()
        _active.value = null
    }
}
