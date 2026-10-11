package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumPage
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.AppEntry
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.FavouriteLibraryItems
import io.github.jamezuk.lyrionanywhere.core.FavouritePage
import io.github.jamezuk.lyrionanywhere.core.FolderPage
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.GenrePage
import io.github.jamezuk.lyrionanywhere.core.InfoKind
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.MenuAction
import io.github.jamezuk.lyrionanywhere.core.MenuOutcome
import io.github.jamezuk.lyrionanywhere.core.MenuPage
import io.github.jamezuk.lyrionanywhere.core.PlaylistPage
import io.github.jamezuk.lyrionanywhere.core.PluginEntry
import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import io.github.jamezuk.lyrionanywhere.core.YearPage
import io.github.jamezuk.lyrionanywhere.core.ArtImage
import io.github.jamezuk.lyrionanywhere.core.ArtistPage
import io.github.jamezuk.lyrionanywhere.core.EventListener
import io.github.jamezuk.lyrionanywhere.core.EventsState
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ServerEvent
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.TransferOutcome
import io.github.jamezuk.lyrionanywhere.core.TransferProgress
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.ExperimentalCoroutinesApi

/** Library browsing. Pages are at most 500 items. */
interface LibraryRepository {
    suspend fun albums(
        start: Int,
        count: Int,
        sort: AlbumSort,
        search: String? = null,
        filter: LibraryFilter = LibraryFilter(),
    ): ApiResult<AlbumPage>

    suspend fun album(id: Long): ApiResult<Album?>

    suspend fun albumTracks(albumId: Long): ApiResult<List<Track>>

    suspend fun artists(
        start: Int,
        count: Int,
        search: String? = null,
        filter: LibraryFilter = LibraryFilter(),
    ): ApiResult<ArtistPage>

    /** Whether the library has any artists in this role. */
    suspend fun hasRole(role: ArtistRole, libraryId: String?): ApiResult<Boolean>

    suspend fun artistAlbums(artistId: Long, role: ArtistRole? = null): ApiResult<List<Album>>

    suspend fun genres(start: Int, count: Int, libraryId: String?, search: String? = null): ApiResult<GenrePage>

    suspend fun genre(id: Long): ApiResult<Genre?>

    suspend fun years(start: Int, count: Int, libraryId: String?): ApiResult<YearPage>

    suspend fun musicFolder(start: Int, count: Int, folderId: Long?): ApiResult<FolderPage>

    suspend fun libraries(): ApiResult<List<VirtualLibrary>>

    suspend fun favourites(start: Int, count: Int, itemId: String? = null): ApiResult<FavouritePage>

    suspend fun playlists(start: Int, count: Int): ApiResult<PlaylistPage>

    suspend fun playlistTracks(playlistId: Long): ApiResult<List<Track>>

    suspend fun search(term: String, count: Int = 20): ApiResult<SearchResults>

    /** Library tracks and albums among the LMS favourites ("Keep favourites offline"). */
    suspend fun favouriteLibraryItems(): ApiResult<FavouriteLibraryItems> =
        ApiResult.Ok(FavouriteLibraryItems(emptyList(), emptyList()))
}

/** LMS menus (apps, radio, plugins and "More…" menus), browsed for a player. */
interface MenusRepository {
    suspend fun apps(): ApiResult<List<AppEntry>>

    suspend fun radios(): ApiResult<List<AppEntry>>

    suspend fun plugins(player: String, shownApps: List<String>): ApiResult<List<PluginEntry>>

    suspend fun open(player: String, action: MenuAction, start: Int, count: Int, input: String? = null): ApiResult<MenuPage>

    suspend fun run(player: String, action: MenuAction, input: String? = null): ApiResult<MenuOutcome>

    suspend fun probe(player: String, action: MenuAction): Suitability

    /** An app's top menu. */
    fun appMenu(cmd: String): ApiResult<MenuAction>

    /** The "More…" menu for a library item. */
    fun infoMenu(kind: InfoKind, id: Long): ApiResult<MenuAction>
}

/** Remote control of LMS players. */
interface PlayersRepository {
    suspend fun list(): ApiResult<List<Player>>

    /** Sync groups among the players this account may control. */
    suspend fun syncGroups(): ApiResult<List<SyncGroup>>

    suspend fun status(player: String): ApiResult<PlayerStatus>

    suspend fun queue(player: String, start: Int, count: Int): ApiResult<List<Track>>

    suspend fun play(player: String): ApiResult<Unit>

    suspend fun setPaused(player: String, paused: Boolean): ApiResult<Unit>

    suspend fun next(player: String): ApiResult<Unit>

    suspend fun previous(player: String): ApiResult<Unit>

    suspend fun jumpTo(player: String, index: Int): ApiResult<Unit>

    suspend fun seek(player: String, seconds: Double): ApiResult<Unit>

    suspend fun setVolume(player: String, volume: Int): ApiResult<Unit>

    suspend fun setShuffle(player: String, mode: ShuffleMode): ApiResult<Unit>

    suspend fun setRepeat(player: String, mode: RepeatMode): ApiResult<Unit>

    suspend fun setPower(player: String, on: Boolean): ApiResult<Unit>

    suspend fun enqueue(
        player: String,
        selection: Selection,
        placement: Placement,
        playIndex: Int? = null,
    ): ApiResult<Unit>

    /** Play a favourite (a station, track or app item). */
    suspend fun playFavourite(player: String, itemId: String, placement: Placement): ApiResult<Unit>

    /** Take the queue entry at [index] out. */
    suspend fun removeFromQueue(player: String, index: Int): ApiResult<Unit>

    /** Move the queue entry at [from] so that it ends up at [to]. */
    suspend fun moveInQueue(player: String, from: Int, to: Int): ApiResult<Unit>

    suspend fun clearQueue(player: String): ApiResult<Unit>

    /** Undo a removal: put library track [trackId] back at [index]. */
    suspend fun restoreToQueue(player: String, trackId: Long, index: Int): ApiResult<Unit>

    /** Put [other] in [current]'s sync group, or take [other] out of its group. */
    suspend fun setSynced(current: String, other: String, on: Boolean): ApiResult<Unit>

    /**
     * Move playback from [source] to [target] (docs/PROTOCOL.md › Transfer), reporting each
     * stage to [onStage] (on a background thread).
     */
    suspend fun transfer(source: String, target: String, onStage: (TransferStage) -> Unit): ApiResult<TransferOutcome>
}

/** Artwork bytes by key (from a record), for the image loader. */
interface ArtRepository {
    suspend fun image(key: String, sizePx: Int): ApiResult<ArtImage>
}

/** What the event stream delivers. */
sealed interface StreamUpdate {
    data class Event(val event: ServerEvent) : StreamUpdate

    data class State(val state: EventsState) : StreamUpdate
}

interface EventsRepository {
    /** Events and connection states while collected; the socket closes when collection stops. */
    fun updates(): Flow<StreamUpdate>
}

private fun Int.u(): UInt = coerceAtLeast(0).toUInt()

class RustLibraryRepository(private val source: ConnectionSource) : LibraryRepository {
    private suspend fun <T> withLibrary(block: suspend (io.github.jamezuk.lyrionanywhere.core.MusicLibrary) -> T): ApiResult<T> {
        val lib = source.connection.value?.library ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(lib) }
    }

    override suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String?, filter: LibraryFilter) =
        withLibrary { it.albums(start.u(), count.u(), sort, search, filter) }

    override suspend fun album(id: Long) = withLibrary { it.album(id) }

    override suspend fun albumTracks(albumId: Long) = withLibrary { it.albumTracks(albumId) }

    override suspend fun artists(start: Int, count: Int, search: String?, filter: LibraryFilter) =
        withLibrary { it.artists(start.u(), count.u(), search, filter) }

    override suspend fun hasRole(role: ArtistRole, libraryId: String?) = withLibrary { it.hasRole(role, libraryId) }

    override suspend fun artistAlbums(artistId: Long, role: ArtistRole?) = withLibrary { it.artistAlbums(artistId, role) }

    override suspend fun genres(start: Int, count: Int, libraryId: String?, search: String?) =
        withLibrary { it.genres(start.u(), count.u(), libraryId, search) }

    override suspend fun genre(id: Long) = withLibrary { it.genre(id) }

    override suspend fun years(start: Int, count: Int, libraryId: String?) =
        withLibrary { it.years(start.u(), count.u(), libraryId) }

    override suspend fun musicFolder(start: Int, count: Int, folderId: Long?) =
        withLibrary { it.musicFolder(start.u(), count.u(), folderId) }

    override suspend fun libraries() = withLibrary { it.libraries() }

    override suspend fun favourites(start: Int, count: Int, itemId: String?) =
        withLibrary { it.favourites(start.u(), count.u(), itemId) }

    override suspend fun playlists(start: Int, count: Int) = withLibrary { it.playlists(start.u(), count.u()) }

    override suspend fun playlistTracks(playlistId: Long) = withLibrary { it.playlistTracks(playlistId) }

    override suspend fun search(term: String, count: Int) = withLibrary { it.search(term, count.u()) }

    override suspend fun favouriteLibraryItems() = withLibrary { it.favouriteLibraryItems() }
}

class RustMenusRepository(private val source: ConnectionSource) : MenusRepository {
    private suspend fun <T> withMenus(block: suspend (io.github.jamezuk.lyrionanywhere.core.Menus) -> T): ApiResult<T> {
        val m = source.connection.value?.menus ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(m) }
    }

    override suspend fun apps() = withMenus { it.apps() }

    override suspend fun radios() = withMenus { it.radios() }

    override suspend fun plugins(player: String, shownApps: List<String>) = withMenus { it.plugins(player, shownApps) }

    override suspend fun open(player: String, action: MenuAction, start: Int, count: Int, input: String?) =
        withMenus { it.open(player, action, start.u(), count.u(), input) }

    override suspend fun run(player: String, action: MenuAction, input: String?) =
        withMenus { it.run(player, action, input) }

    override suspend fun probe(player: String, action: MenuAction): Suitability =
        when (val r = withMenus { it.probe(player, action) }) {
            is ApiResult.Ok -> r.value
            is ApiResult.Err -> Suitability.Failed("The music server could not open it.")
        }

    override fun appMenu(cmd: String): ApiResult<MenuAction> = try {
        ApiResult.Ok(io.github.jamezuk.lyrionanywhere.core.appMenu(cmd))
    } catch (e: Exception) {
        ApiResult.Err(e.toAppError())
    }

    override fun infoMenu(kind: InfoKind, id: Long): ApiResult<MenuAction> = try {
        ApiResult.Ok(io.github.jamezuk.lyrionanywhere.core.infoMenu(kind, id))
    } catch (e: Exception) {
        ApiResult.Err(e.toAppError())
    }
}

class RustPlayersRepository(private val source: ConnectionSource) : PlayersRepository {
    private suspend fun <T> withPlayers(block: suspend (io.github.jamezuk.lyrionanywhere.core.Players) -> T): ApiResult<T> {
        val p = source.connection.value?.players ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(p) }
    }

    override suspend fun list() = withPlayers { it.list() }

    override suspend fun syncGroups() = withPlayers { it.syncGroups() }

    override suspend fun status(player: String) = withPlayers { it.status(player) }

    override suspend fun queue(player: String, start: Int, count: Int) =
        withPlayers { it.queue(player, start.u(), count.u()) }

    override suspend fun play(player: String) = withPlayers { it.play(player) }

    override suspend fun setPaused(player: String, paused: Boolean) = withPlayers { it.setPaused(player, paused) }

    override suspend fun next(player: String) = withPlayers { it.next(player) }

    override suspend fun previous(player: String) = withPlayers { it.previous(player) }

    override suspend fun jumpTo(player: String, index: Int) = withPlayers { it.jumpTo(player, index.u()) }

    override suspend fun seek(player: String, seconds: Double) = withPlayers { it.seek(player, seconds) }

    override suspend fun setVolume(player: String, volume: Int) =
        withPlayers { it.setVolume(player, volume.coerceIn(0, 100).toUInt()) }

    override suspend fun setShuffle(player: String, mode: ShuffleMode) = withPlayers { it.setShuffle(player, mode) }

    override suspend fun setRepeat(player: String, mode: RepeatMode) = withPlayers { it.setRepeat(player, mode) }

    override suspend fun setPower(player: String, on: Boolean) = withPlayers { it.setPower(player, on) }

    override suspend fun enqueue(player: String, selection: Selection, placement: Placement, playIndex: Int?) =
        withPlayers { it.enqueue(player, selection, placement, playIndex?.u()) }

    override suspend fun playFavourite(player: String, itemId: String, placement: Placement) =
        withPlayers { it.playFavourite(player, itemId, placement) }

    override suspend fun removeFromQueue(player: String, index: Int) =
        withPlayers { it.removeFromQueue(player, index.u()) }

    override suspend fun moveInQueue(player: String, from: Int, to: Int) =
        withPlayers { it.moveInQueue(player, from.u(), to.u()) }

    override suspend fun clearQueue(player: String) = withPlayers { it.clearQueue(player) }

    override suspend fun restoreToQueue(player: String, trackId: Long, index: Int) =
        withPlayers { it.restoreToQueue(player, trackId, index.u()) }

    override suspend fun setSynced(current: String, other: String, on: Boolean) =
        withPlayers { it.setSynced(current, other, on) }

    override suspend fun transfer(source: String, target: String, onStage: (TransferStage) -> Unit) =
        withPlayers {
            it.transfer(
                source,
                target,
                object : TransferProgress {
                    override fun onStage(stage: TransferStage) = onStage(stage)
                },
            )
        }
}

class RustArtRepository(private val source: ConnectionSource) : ArtRepository {
    override suspend fun image(key: String, sizePx: Int): ApiResult<ArtImage> {
        val art = source.connection.value?.art ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { art.fetch(key, sizePx.u()) }
    }
}

class RustEventsRepository(private val source: ConnectionSource) : EventsRepository {
    @OptIn(ExperimentalCoroutinesApi::class)
    override fun updates(): Flow<StreamUpdate> = source.connection.flatMapLatest { conn ->
        if (conn == null) {
            emptyFlow()
        } else {
            callbackFlow {
                val listener = object : EventListener {
                    override fun onEvent(event: ServerEvent) {
                        trySend(StreamUpdate.Event(event))
                    }

                    override fun onState(state: EventsState) {
                        trySend(StreamUpdate.State(state))
                    }
                }
                val subscription = conn.events.subscribe(listener)
                awaitClose {
                    subscription.cancel()
                    subscription.close()
                }
            }
        }
    }
}
