package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumPage
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
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
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.flow.flatMapLatest
import kotlinx.coroutines.ExperimentalCoroutinesApi

/** Library browsing. Pages are at most 500 items. */
interface LibraryRepository {
    suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String? = null): ApiResult<AlbumPage>

    suspend fun album(id: Long): ApiResult<Album?>

    suspend fun albumTracks(albumId: Long): ApiResult<List<Track>>

    suspend fun artists(start: Int, count: Int, search: String? = null): ApiResult<ArtistPage>

    suspend fun artistAlbums(artistId: Long): ApiResult<List<Album>>

    suspend fun search(term: String, count: Int = 20): ApiResult<SearchResults>
}

/** Remote control of LMS players. */
interface PlayersRepository {
    suspend fun list(): ApiResult<List<Player>>

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

    override suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String?) =
        withLibrary { it.albums(start.u(), count.u(), sort, search) }

    override suspend fun album(id: Long) = withLibrary { it.album(id) }

    override suspend fun albumTracks(albumId: Long) = withLibrary { it.albumTracks(albumId) }

    override suspend fun artists(start: Int, count: Int, search: String?) =
        withLibrary { it.artists(start.u(), count.u(), search) }

    override suspend fun artistAlbums(artistId: Long) = withLibrary { it.artistAlbums(artistId) }

    override suspend fun search(term: String, count: Int) = withLibrary { it.search(term, count.u()) }
}

class RustPlayersRepository(private val source: ConnectionSource) : PlayersRepository {
    private suspend fun <T> withPlayers(block: suspend (io.github.jamezuk.lyrionanywhere.core.Players) -> T): ApiResult<T> {
        val p = source.connection.value?.players ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(p) }
    }

    override suspend fun list() = withPlayers { it.list() }

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
