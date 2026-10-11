package io.github.jamezuk.lyrionanywhere.playback.auto

import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumPage
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistPage
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.BrowseNode
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.FavouritePage
import io.github.jamezuk.lyrionanywhere.core.FolderPage
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.GenrePage
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.PageWindow
import io.github.jamezuk.lyrionanywhere.core.PlaylistPage
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import io.github.jamezuk.lyrionanywhere.core.YearPage
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.playback.FakePlayers
import io.github.jamezuk.lyrionanywhere.playback.dlTrack

/**
 * A stand-in for the Rust grammar (tested in Rust: `mobile/src/auto.rs`): ids are handed
 * out as opaque tokens, and only tokens it handed out parse back. So every id the tree
 * uses must have come from the codec, and anything else is refused.
 */
class FakeCodec : NodeCodec {
    private val byId = HashMap<String, BrowseNode>()
    private val byNode = HashMap<BrowseNode, String>()

    override fun id(node: BrowseNode): String? {
        if (node is BrowseNode.Album && node.id <= 0) return null
        return byNode.getOrPut(node) { "n${byNode.size}".also { byId[it] = node } }
    }

    override fun parse(id: String): BrowseNode? = byId[id]

    override fun root(online: Boolean, hasDownloads: Boolean) = when {
        !online -> listOf(BrowseNode.Downloads)
        hasDownloads -> listOf(BrowseNode.Home, BrowseNode.Library, BrowseNode.Downloads)
        else -> listOf(BrowseNode.Home, BrowseNode.Library)
    }

    override fun home(hasQueue: Boolean) =
        (if (hasQueue) listOf(BrowseNode.Resume) else emptyList()) +
            listOf(BrowseNode.NewMusic, BrowseNode.Favourites, BrowseNode.Playlists)

    override fun library() =
        listOf(BrowseNode.NewMusic, BrowseNode.Albums, BrowseNode.Artists, BrowseNode.Playlists, BrowseNode.Favourites)

    override fun page(page: Int, pageSize: Int): PageWindow? {
        val size = if (pageSize <= 0) 100 else minOf(pageSize, 100)
        if (page < 0) return null
        return PageWindow((page * size).toUInt(), size.toUInt())
    }

    override fun browsable(node: BrowseNode) = when (node) {
        BrowseNode.Resume, is BrowseNode.Favourite, is BrowseNode.Track, is BrowseNode.AlbumTrack,
        is BrowseNode.PlaylistTrack, is BrowseNode.DownloadedTrack,
        -> false
        else -> true
    }

    override fun playable(node: BrowseNode) = when (node) {
        BrowseNode.Resume, is BrowseNode.Album, is BrowseNode.Playlist, is BrowseNode.Favourite, is BrowseNode.Track,
        is BrowseNode.AlbumTrack, is BrowseNode.PlaylistTrack, is BrowseNode.DownloadedAlbum, is BrowseNode.DownloadedTrack,
        -> true
        else -> false
    }
}

fun libAlbum(id: Long, title: String = "Album $id") = Album(id, title, "Artist", 3, 1999, "music/$id/cover", false)

fun libTrack(id: Long, title: String = "Track $id") =
    Track(id, title, "Artist", "Album", 7, 3, 180.0, 1u, null, null, null, null, null, null, null)

fun dlAlbum(ref: String, title: String) =
    OfflineAlbum(ref, ref.drop(1).toLongOrNull(), title, "Band", 2u, 2000uL, false, 0uL, DownloadOrigin.MANUAL)

/** The library, with canned answers; every call is recorded. */
class FakeLibrary : LibraryRepository {
    val calls = mutableListOf<String>()
    var albums = (1L..30L).map { libAlbum(it) }
    var fail = false

    private fun <T> answer(call: String, value: T): ApiResult<T> {
        calls += call
        return if (fail) ApiResult.Err(AppError.Offline()) else ApiResult.Ok(value)
    }

    override suspend fun albums(start: Int, count: Int, sort: AlbumSort, search: String?, filter: LibraryFilter) =
        answer("albums $start $count $sort", AlbumPage(albums.size.toUInt(), albums.drop(start).take(count)))

    override suspend fun album(id: Long) = answer("album $id", albums.firstOrNull { it.id == id })

    override suspend fun albumTracks(albumId: Long) = answer("albumTracks $albumId", (1L..12L).map { libTrack(albumId * 100 + it) })

    override suspend fun artists(start: Int, count: Int, search: String?, filter: LibraryFilter) =
        answer("artists $start $count", ArtistPage(2u, listOf(Artist(3, "Artist"), Artist(4, "Band")).drop(start).take(count)))

    override suspend fun hasRole(role: ArtistRole, libraryId: String?) = answer("hasRole", false)

    override suspend fun artistAlbums(artistId: Long, role: ArtistRole?) = answer("artistAlbums $artistId", albums.take(3))

    override suspend fun genres(start: Int, count: Int, libraryId: String?, search: String?) =
        answer("genres", GenrePage(0u, emptyList()))

    override suspend fun genre(id: Long) = answer<Genre?>("genre", null)

    override suspend fun years(start: Int, count: Int, libraryId: String?) = answer("years", YearPage(0u, emptyList()))

    override suspend fun musicFolder(start: Int, count: Int, folderId: Long?) =
        answer("folder", FolderPage(0u, emptyList()))

    override suspend fun libraries() = answer("libraries", emptyList<VirtualLibrary>())

    override suspend fun favourites(start: Int, count: Int, itemId: String?) = answer(
        "favourites $start $count $itemId",
        FavouritePage(
            null,
            2u,
            listOf(Favourite("f.1", "Radio One", null, false), Favourite("f.2", "Folder", null, true)),
        ),
    )

    override suspend fun playlists(start: Int, count: Int) =
        answer("playlists $start $count", PlaylistPage(1u, listOf(SavedPlaylist(5, "Road trip"))))

    override suspend fun playlistTracks(playlistId: Long) = answer("playlistTracks $playlistId", listOf(libTrack(1), libTrack(2)))

    override suspend fun search(term: String, count: Int) = answer(
        "search $term",
        SearchResults(listOf(Artist(3, "Artist")), listOf(libAlbum(7, "Kind of Blue")), listOf(libTrack(42, "So What"))),
    )
}

/** The app's side, with the phone's player and downloads recorded. */
class FakeAutoSource(
    override val library: FakeLibrary = FakeLibrary(),
    override val players: FakePlayers = FakePlayers(),
) : AutoSource {
    var isOnline = true
    var queued = false
    var phoneId: String? = "02:4c:41:00:00:01"
    val albumsHere = mutableListOf(dlAlbum("a7", "Record"), dlAlbum("a8", "Another"))
    val tracksHere = mapOf("a7" to listOf(dlTrack(1), dlTrack(2)), "a8" to listOf(dlTrack(3, 8)))
    val played = mutableListOf<Pair<List<OfflineTrack>, Int>>()
    var resumed = 0
    var settled = 0

    override fun online() = isOnline

    override suspend fun settle() {
        settled++
    }

    override fun downloadedAlbums() = albumsHere

    override fun downloadedTracks(albumRef: String) = tracksHere[albumRef].orEmpty()

    override fun allDownloadedTracks() = tracksHere.values.flatten()

    override fun hasQueue() = queued

    override suspend fun onPhone(command: suspend (playerId: String) -> ApiResult<Unit>): ApiResult<Unit> {
        val id = phoneId ?: return ApiResult.Err(AppError.Offline())
        return command(id)
    }

    override fun playDownloads(tracks: List<OfflineTrack>, start: Int) {
        played += tracks to start
    }

    override suspend fun resume(): Boolean {
        resumed++
        return true
    }
}
