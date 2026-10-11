package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumPage
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.ArtistPage
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Device
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.DownloadObserver
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.DownloadPolicy
import io.github.jamezuk.lyrionanywhere.core.DownloadRequest
import io.github.jamezuk.lyrionanywhere.core.FavouriteLibraryItems
import io.github.jamezuk.lyrionanywhere.core.FavouritePage
import io.github.jamezuk.lyrionanywhere.core.FolderPage
import io.github.jamezuk.lyrionanywhere.core.FormatChoice
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.GenrePage
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.PlaylistPage
import io.github.jamezuk.lyrionanywhere.core.PlaysSyncOutcome
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.SmartPlan
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.User
import io.github.jamezuk.lyrionanywhere.core.UserRole
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import io.github.jamezuk.lyrionanywhere.core.YearPage
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.ServerInfo
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update

val SERVER = ServerInfo("https://music.example.com/", "music.example.com", true)

fun who(name: String = "alice") = SignedIn(User(name, UserRole.LISTENER), Device(1, "Phone", "02:4c:41:00:00:01"))

fun offlineTrack(n: Int, origin: DownloadOrigin = DownloadOrigin.MANUAL, albumId: Long? = 7) = OfflineTrack(
    key = "%064x".format(n),
    trackId = 100L + n,
    title = "Track $n",
    artist = "Artist",
    album = "Album",
    albumId = albumId,
    trackNumber = n.toUInt(),
    durationSecs = 61.0,
    format = DownloadFormat.ORIGINAL,
    codec = "flc",
    ext = "flac",
    size = 1000uL,
    downloadedAt = 1_800_000_000uL,
    origin = origin,
)

fun libTrack(id: Long, albumId: Long? = 7) = Track(
    id, "Track $id", "Artist", "Album", albumId, 3, 61.0, id.toUInt(), "music/$id/cover", null, null, null, "Jazz", "FLAC", 1999,
)

class MemSettings(initial: AppSettings = AppSettings()) : SettingsStore {
    val state = MutableStateFlow(initial)
    override val settings: Flow<AppSettings> = state

    override suspend fun setServerUrl(url: String?) = state.update { it.copy(serverUrl = url, offlineOwner = null) }

    override suspend fun setSelectedPlayer(playerId: String?) = state.update { it.copy(selectedPlayerId = playerId) }

    override suspend fun setPlayOnPhone(on: Boolean) = state.update { it.copy(playOnPhone = on) }

    override suspend fun setSound(sound: SoundSettings) = state.update { it.copy(sound = sound) }

    override suspend fun setLibrary(id: String?) = state.update { it.copy(libraryId = id) }

    override suspend fun setDefaultPlayer(choice: DefaultPlayer, playerId: String?) =
        state.update { it.copy(defaultPlayer = choice, defaultPlayerId = playerId) }

    override suspend fun setDownloads(downloads: DownloadSettings) = state.update { it.copy(downloads = downloads.sanitised()) }

    override suspend fun setOfflineOwner(username: String?) = state.update { it.copy(offlineOwner = username) }
}

class FakeStore(val owner: Owner, var tracks: MutableList<OfflineTrack> = mutableListOf()) : OfflineStore {
    var closed = false
    var plan = SmartPlan(emptyList(), emptyList())
    val planned = mutableListOf<List<DownloadRequest>>()
    val removed = mutableListOf<String>()
    var removedAll = false
    var pending = 0

    override fun tracks() = tracks.toList()

    override fun albums() = tracks.groupBy { it.albumId }.map { (id, ts) ->
        OfflineAlbum("a$id", id, ts[0].album, ts[0].artist, ts.size.toUInt(), ts.sumOf { it.size }, false, 0uL, DownloadOrigin.MANUAL)
    }

    override fun usedBytes() = tracks.sumOf { it.size.toLong() }

    override fun artwork(albumRef: String): ByteArray? = if (albumRef == "a7") byteArrayOf(1, 2, 3) else null

    override fun remove(keys: List<String>) {
        removed += keys
        tracks.removeAll { it.key in keys }
    }

    override fun removeAlbum(albumRef: String) {
        tracks.removeAll { "a${it.albumId}" == albumRef }
    }

    override fun removeAll() {
        removedAll = true
        tracks.clear()
    }

    override fun pendingPlays() = pending

    override fun planSmart(wanted: List<DownloadRequest>): SmartPlan {
        planned += wanted
        return plan
    }

    override fun close() {
        closed = true
    }
}

class FakeDownloader : DownloaderHandle {
    val calls = mutableListOf<String>()
    val enqueued = mutableListOf<DownloadRequest>()
    var shutDown = false

    override fun enqueue(requests: List<DownloadRequest>, choice: FormatChoice) {
        enqueued += requests
        calls += "enqueue ${requests.size} $choice"
    }

    override fun pause(trackId: Long?) {
        calls += "pause $trackId"
    }

    override fun resume(trackId: Long?) {
        calls += "resume $trackId"
    }

    override fun cancel(trackId: Long?) {
        calls += "cancel $trackId"
    }

    override fun clearFinished() {
        calls += "clear"
    }

    override fun setPolicy(policy: DownloadPolicy) {
        calls += "policy ${policy.wifiOnly} ${policy.capBytes}"
    }

    override fun setNetwork(network: NetworkKind) {
        calls += "network $network"
    }

    override fun shutdown() {
        shutDown = true
    }
}

class FakeLink : OnlineLink {
    val downloaders = mutableListOf<FakeDownloader>()
    var observer: DownloadObserver? = null
    var startedWith: Pair<DownloadPolicy, NetworkKind>? = null
    var sync: () -> PlaysSyncOutcome = { PlaysSyncOutcome(0u, 0u, 0u, false) }
    var syncs = 0

    override fun downloader(
        store: OfflineStore,
        policy: DownloadPolicy,
        network: NetworkKind,
        observer: DownloadObserver,
    ): DownloaderHandle {
        this.observer = observer
        startedWith = policy to network
        return FakeDownloader().also { downloaders += it }
    }

    override suspend fun syncPlays(store: OfflineStore): PlaysSyncOutcome {
        syncs++
        return sync()
    }
}

/** A library with recently added albums 1…5 (tracks id*10+1, id*10+2) and favourites. */
class FakeLibrary : LibraryRepository {
    var failAlbum: Long? = null
    var favourites = FavouriteLibraryItems(emptyList(), emptyList())
    val asked = mutableListOf<String>()

    private fun <T> no(): ApiResult<T> = ApiResult.Err(AppError.Unexpected("not in this fake"))

    override suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String?, filter: LibraryFilter): ApiResult<AlbumPage> {
        asked += "albums $count $sort"
        val items = (1L..5L).map { Album(it, "Album $it", "Artist", 3, 1999, null, false) }.take(count)
        return ApiResult.Ok(AlbumPage(5u, items))
    }

    override suspend fun album(id: Long): ApiResult<Album?> = no()

    override suspend fun albumTracks(albumId: Long): ApiResult<List<Track>> {
        asked += "tracks $albumId"
        if (albumId == failAlbum) return ApiResult.Err(AppError.Offline("down"))
        return ApiResult.Ok(listOf(libTrack(albumId * 10 + 1, albumId), libTrack(albumId * 10 + 2, albumId)))
    }

    override suspend fun artists(start: Int, count: Int, search: String?, filter: LibraryFilter): ApiResult<ArtistPage> = no()

    override suspend fun hasRole(role: ArtistRole, libraryId: String?): ApiResult<Boolean> = no()

    override suspend fun artistAlbums(artistId: Long, role: ArtistRole?): ApiResult<List<Album>> = no()

    override suspend fun genres(start: Int, count: Int, libraryId: String?, search: String?): ApiResult<GenrePage> = no()

    override suspend fun genre(id: Long): ApiResult<Genre?> = no()

    override suspend fun years(start: Int, count: Int, libraryId: String?): ApiResult<YearPage> = no()

    override suspend fun musicFolder(start: Int, count: Int, folderId: Long?): ApiResult<FolderPage> = no()

    override suspend fun libraries(): ApiResult<List<VirtualLibrary>> = no()

    override suspend fun favourites(start: Int, count: Int, itemId: String?): ApiResult<FavouritePage> = no()

    override suspend fun playlists(start: Int, count: Int): ApiResult<PlaylistPage> = no()

    override suspend fun playlistTracks(playlistId: Long): ApiResult<List<Track>> =
        ApiResult.Ok(listOf(libTrack(501, 50), Track(null, "Radio", null, null, null, null, null, null, null, null, null, null, null, null, null)))

    override suspend fun search(term: String, count: Int): ApiResult<SearchResults> = no()

    override suspend fun favouriteLibraryItems(): ApiResult<FavouriteLibraryItems> = ApiResult.Ok(favourites)
}
