package io.github.jamezuk.lyrionanywhere.playback.auto

import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.BrowseNode
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.PageWindow
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.browseHome
import io.github.jamezuk.lyrionanywhere.core.browseLibrary
import io.github.jamezuk.lyrionanywhere.core.browseRoot
import io.github.jamezuk.lyrionanywhere.core.nodeBrowsable
import io.github.jamezuk.lyrionanywhere.core.nodeId
import io.github.jamezuk.lyrionanywhere.core.nodePlayable
import io.github.jamezuk.lyrionanywhere.core.pageWindow
import io.github.jamezuk.lyrionanywhere.core.parseNode
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository

/** The browse tree's grammar and layout: the Rust browse model (`mobile/src/auto.rs`),
 *  behind a seam so the tree can be tested on the JVM. */
interface NodeCodec {
    fun id(node: BrowseNode): String?

    fun parse(id: String): BrowseNode?

    fun root(online: Boolean, hasDownloads: Boolean): List<BrowseNode>

    fun home(hasQueue: Boolean): List<BrowseNode>

    fun library(): List<BrowseNode>

    fun page(page: Int, pageSize: Int): PageWindow?

    fun browsable(node: BrowseNode): Boolean

    fun playable(node: BrowseNode): Boolean
}

/** The real grammar and layout, from the Rust layer. */
object RustNodeCodec : NodeCodec {
    override fun id(node: BrowseNode) = nodeId(node)

    override fun parse(id: String) = parseNode(id)

    override fun root(online: Boolean, hasDownloads: Boolean) = browseRoot(online, hasDownloads)

    override fun home(hasQueue: Boolean) = browseHome(hasQueue)

    override fun library() = browseLibrary()

    override fun page(page: Int, pageSize: Int) = pageWindow(page, pageSize)

    override fun browsable(node: BrowseNode) = nodeBrowsable(node)

    override fun playable(node: BrowseNode) = nodePlayable(node)
}

/** What the car's tree reads and plays: the app's own repositories (a seam for tests). */
interface AutoSource {
    /** Signed in, and the server reachable. */
    fun online(): Boolean

    /** The car may start the app from cold: wait (briefly) for the saved sign-in to settle. */
    suspend fun settle() {}

    val library: LibraryRepository
    val players: PlayersRepository

    /** Downloaded albums, by title. */
    fun downloadedAlbums(): List<OfflineAlbum>

    /** A downloaded album's tracks, in album order. */
    fun downloadedTracks(albumRef: String): List<OfflineTrack>

    fun allDownloadedTracks(): List<OfflineTrack>

    /** Something is queued on this phone, to carry on with. */
    fun hasQueue(): Boolean

    /** Run [command] on this phone's LMS player (D20), switching it on first if needed. */
    suspend fun onPhone(command: suspend (playerId: String) -> ApiResult<Unit>): ApiResult<Unit>

    /** Play downloads from the app's own queue. */
    fun playDownloads(tracks: List<OfflineTrack>, start: Int)

    /** Carry on with what is queued (downloads if they have the session, else the phone's
     *  LMS player). */
    suspend fun resume(): Boolean
}

/** Titles for the tree's fixed nodes (from the app's resources). */
fun interface AutoLabels {
    fun title(node: BrowseNode): String
}

/**
 * The car's browse tree (Android Auto now, the same shape for CarPlay later): what each node
 * holds, search, and playing what the car chose, always on this phone (SPEC D20), or from
 * the downloads when there's no server.
 *
 * Every media id that comes back from the car is parsed by the Rust grammar first; anything
 * else is refused. Items carry no URI (so no file path, stream or server address): playing
 * one goes through [play], by id.
 */
class AutoCatalogue(
    private val source: AutoSource,
    private val labels: AutoLabels,
    private val codec: NodeCodec = RustNodeCodec,
    /** Album covers as URIs of the app's own provider (none: no art). */
    private val art: AutoArtUris? = null,
) {
    /** The tree's root (its children are the tabs). */
    fun root(): MediaItem = folder(BrowseNode.Root, labels.title(BrowseNode.Root))!!

    /** A node's children, one page (`null`: no such node, or not browsable). */
    suspend fun children(parentId: String, page: Int, pageSize: Int): List<MediaItem>? {
        val node = codec.parse(parentId) ?: return null
        if (!codec.browsable(node)) return null
        source.settle()
        val window = codec.page(page, pageSize) ?: return emptyList()
        val online = source.online()
        if (needsServer(node) && !online) return emptyList()
        val start = window.start.toInt()
        val count = window.count.toInt()
        val lib = source.library
        return when (node) {
            BrowseNode.Root -> codec.root(online, source.downloadedAlbums().isNotEmpty())
                .slice(start, count).mapNotNull(::fixed)
            BrowseNode.Home -> codec.home(source.hasQueue()).slice(start, count).mapNotNull(::fixed)
            BrowseNode.Library -> codec.library().slice(start, count).mapNotNull(::fixed)
            BrowseNode.Downloads -> downloadedAlbums(source.downloadedAlbums().slice(start, count))
            BrowseNode.NewMusic -> ok(lib.albums(start, count, AlbumSort.NEWEST))?.items?.let { albums(it) }
            BrowseNode.Albums -> ok(lib.albums(start, count, AlbumSort.TITLE))?.items?.let { albums(it) }
            BrowseNode.Artists -> ok(lib.artists(start, count))?.items?.mapNotNull { a ->
                folder(BrowseNode.Artist(a.id), a.name, type = MediaMetadata.MEDIA_TYPE_ARTIST)
            }
            BrowseNode.Playlists -> ok(lib.playlists(start, count))?.items?.mapNotNull { p ->
                item(BrowseNode.Playlist(p.id), p.name, null, browsable = true, playable = true, MediaMetadata.MEDIA_TYPE_PLAYLIST)
            }
            BrowseNode.Favourites -> favourites(null, start, count)
            is BrowseNode.FavouriteFolder -> favourites(node.itemId, start, count)
            is BrowseNode.Album -> ok(lib.albumTracks(node.id))?.let { tracks ->
                tracks.withIndex().toList().slice(start, count).mapNotNull { (i, t) ->
                    item(
                        BrowseNode.AlbumTrack(node.id, i.toUInt()),
                        t.title,
                        t.artist,
                        browsable = false,
                        playable = true,
                        MediaMetadata.MEDIA_TYPE_MUSIC,
                    )
                }
            }
            is BrowseNode.Artist -> ok(lib.artistAlbums(node.id))?.slice(start, count)?.let { albums(it) }
            is BrowseNode.Playlist -> ok(lib.playlistTracks(node.id))?.let { tracks ->
                tracks.withIndex().toList().slice(start, count).mapNotNull { (i, t) ->
                    item(
                        BrowseNode.PlaylistTrack(node.id, i.toUInt()),
                        t.title,
                        t.artist,
                        browsable = false,
                        playable = true,
                        MediaMetadata.MEDIA_TYPE_MUSIC,
                    )
                }
            }
            is BrowseNode.DownloadedAlbum ->
                source.downloadedTracks(node.albumRef).withIndex().toList().slice(start, count).mapNotNull { (i, t) ->
                    item(
                        BrowseNode.DownloadedTrack(node.albumRef, i.toUInt()),
                        t.title,
                        t.artist,
                        browsable = false,
                        playable = true,
                        MediaMetadata.MEDIA_TYPE_MUSIC,
                    )
                }
            else -> null
        } ?: emptyList()
    }

    /** One item by id (`null`: unknown). */
    suspend fun item(id: String): MediaItem? {
        val node = codec.parse(id) ?: return null
        fixed(node)?.let { return it }
        return when (node) {
            is BrowseNode.DownloadedAlbum -> source.downloadedAlbums().firstOrNull { it.albumRef == node.albumRef }
                ?.let { downloadedAlbums(listOf(it)).firstOrNull() }
            is BrowseNode.Album -> if (source.online()) ok(source.library.album(node.id))?.let { albums(listOf(it)).firstOrNull() } else null
            else -> null
        }
    }

    /** Would [play] take this request? An id of the grammar that can be played, or a spoken
     *  query with no id. (Checked before Media3 hands the request to the session player.) */
    fun acceptsPlayRequest(m: MediaItem): Boolean = if (m.mediaId.isEmpty()) {
        !m.requestMetadata.searchQuery.isNullOrBlank()
    } else {
        codec.parse(m.mediaId)?.let(codec::playable) == true
    }

    /** Search the library (online) and the downloads, most relevant first. */
    suspend fun search(query: String): List<MediaItem> {
        val q = query.trim().take(MAX_QUERY)
        if (q.isEmpty()) return emptyList()
        source.settle()
        val out = mutableListOf<MediaItem>()
        if (source.online()) {
            ok(source.library.search(q, SEARCH_COUNT))?.let { r ->
                r.tracks.forEach { t ->
                    val id = t.id ?: return@forEach
                    item(BrowseNode.Track(id), t.title, t.artist, browsable = false, playable = true, MediaMetadata.MEDIA_TYPE_MUSIC)
                        ?.let(out::add)
                }
                out += albums(r.albums)
                r.artists.forEach { a ->
                    folder(BrowseNode.Artist(a.id), a.name, type = MediaMetadata.MEDIA_TYPE_ARTIST)?.let(out::add)
                }
            }
        }
        out += downloadedMatches(q)
        return out.take(MAX_RESULTS)
    }

    /**
     * Play what the car chose ([mediaId]), or the best match for a spoken [query] ("play
     * …" with no id), on this phone. False if refused or it failed.
     */
    suspend fun play(mediaId: String?, query: String?): Boolean {
        source.settle()
        if (mediaId.isNullOrEmpty()) {
            val q = query?.trim()?.take(MAX_QUERY).orEmpty()
            if (q.isEmpty()) return source.resume()
            val best = search(q).firstOrNull { it.mediaMetadata.isPlayable == true } ?: return false
            return play(best.mediaId, null)
        }
        val node = codec.parse(mediaId) ?: return false
        if (!codec.playable(node)) return false
        if (needsServer(node) && !source.online() && node !is BrowseNode.Resume) return false
        suspend fun phone(sel: Selection, index: Int?): Boolean =
            source.onPhone { p -> source.players.enqueue(p, sel, Placement.PLAY_NOW, index) } is ApiResult.Ok
        return when (node) {
            BrowseNode.Resume -> source.resume()
            is BrowseNode.Album -> phone(Selection.Album(node.id), null)
            is BrowseNode.AlbumTrack -> phone(Selection.Album(node.albumId), node.index.toInt())
            is BrowseNode.Playlist -> phone(Selection.Playlist(node.id), null)
            is BrowseNode.PlaylistTrack -> phone(Selection.Playlist(node.playlistId), node.index.toInt())
            is BrowseNode.Track -> phone(Selection.Track(node.id), null)
            is BrowseNode.Favourite ->
                source.onPhone { p -> source.players.playFavourite(p, node.itemId, Placement.PLAY_NOW) } is ApiResult.Ok
            is BrowseNode.DownloadedAlbum -> playDownloads(node.albumRef, 0)
            is BrowseNode.DownloadedTrack -> playDownloads(node.albumRef, node.index.toInt())
            else -> false
        }
    }

    private fun playDownloads(ref: String, start: Int): Boolean {
        val tracks = source.downloadedTracks(ref)
        if (start !in tracks.indices) return false
        source.playDownloads(tracks, start)
        return true
    }

    private fun downloadedMatches(q: String): List<MediaItem> {
        val needle = q.lowercase()
        val byRef = source.downloadedAlbums().associateBy { it.albumRef }
        return byRef.keys.flatMap { ref ->
            source.downloadedTracks(ref).withIndex()
                .filter { (_, t) -> listOf(t.title, t.artist, t.album).any { it.lowercase().contains(needle) } }
                .mapNotNull { (i, t) ->
                    item(BrowseNode.DownloadedTrack(ref, i.toUInt()), t.title, t.artist, browsable = false, playable = true, MediaMetadata.MEDIA_TYPE_MUSIC)
                }
        }.take(MAX_RESULTS)
    }

    private suspend fun favourites(itemId: String?, start: Int, count: Int): List<MediaItem>? =
        ok(source.library.favourites(start, count, itemId))?.items?.mapNotNull { f ->
            if (f.isFolder) {
                folder(BrowseNode.FavouriteFolder(f.id), f.name)
            } else {
                item(BrowseNode.Favourite(f.id), f.name, null, browsable = false, playable = true, MediaMetadata.MEDIA_TYPE_MUSIC)
            }
        }

    private fun needsServer(node: BrowseNode) = when (node) {
        BrowseNode.Root, BrowseNode.Downloads, is BrowseNode.DownloadedAlbum, is BrowseNode.DownloadedTrack -> false
        else -> true
    }

    private fun fixed(node: BrowseNode): MediaItem? = when (node) {
        BrowseNode.Root, BrowseNode.Home, BrowseNode.Library, BrowseNode.Downloads, BrowseNode.NewMusic,
        BrowseNode.Albums, BrowseNode.Artists, BrowseNode.Playlists, BrowseNode.Favourites,
        -> folder(node, labels.title(node))
        BrowseNode.Resume -> item(node, labels.title(node), null, browsable = false, playable = true, MediaMetadata.MEDIA_TYPE_MUSIC)
        else -> null
    }

    /** Albums as items, with their covers where there is art. */
    private suspend fun albums(list: List<io.github.jamezuk.lyrionanywhere.core.Album>): List<MediaItem> =
        withArt(list.mapNotNull { a -> album(a)?.let { it to a.artwork } })

    private suspend fun downloadedAlbums(list: List<OfflineAlbum>): List<MediaItem> =
        withArt(list.mapNotNull { a -> downloadedAlbum(a)?.let { it to AutoArtUris.OFFLINE_PREFIX + a.albumRef } })

    /** Each item with its cover's URI (from the app's provider), where one could be had. */
    private suspend fun withArt(items: List<Pair<MediaItem, String?>>): List<MediaItem> {
        val a = art ?: return items.map { it.first }
        val uris = runCatching { a.urisFor(items.map { it.second }) }.getOrNull()
        if (uris == null || uris.size != items.size) return items.map { it.first }
        return items.mapIndexed { i, (item, _) ->
            val uri = uris[i] ?: return@mapIndexed item
            item.buildUpon().setMediaMetadata(item.mediaMetadata.buildUpon().setArtworkUri(uri).build()).build()
        }
    }

    private fun album(a: io.github.jamezuk.lyrionanywhere.core.Album): MediaItem? =
        item(BrowseNode.Album(a.id), a.title, a.artist, browsable = true, playable = true, MediaMetadata.MEDIA_TYPE_ALBUM)

    private fun downloadedAlbum(a: OfflineAlbum): MediaItem? =
        item(BrowseNode.DownloadedAlbum(a.albumRef), a.title, a.artist, browsable = true, playable = true, MediaMetadata.MEDIA_TYPE_ALBUM)

    private fun folder(node: BrowseNode, title: String, type: Int = MediaMetadata.MEDIA_TYPE_FOLDER_MIXED): MediaItem? =
        item(node, title, null, browsable = true, playable = codec.playable(node), type)

    /** A media item: an id, text and flags only. No URI or extras (an album's artwork URI,
     *  added after, is only ever one of the app's own provider: [withArt]). */
    private fun item(
        node: BrowseNode,
        title: String,
        subtitle: String?,
        browsable: Boolean,
        playable: Boolean,
        type: Int,
    ): MediaItem? {
        val id = codec.id(node) ?: return null
        val meta = MediaMetadata.Builder()
            .setTitle(clean(title))
            .setSubtitle(subtitle?.let(::clean))
            .setArtist(subtitle?.let(::clean))
            .setIsBrowsable(browsable)
            .setIsPlayable(playable)
            .setMediaType(type)
            .build()
        return MediaItem.Builder().setMediaId(id).setMediaMetadata(meta).build()
    }

    companion object {
        const val MAX_QUERY = 200
        const val SEARCH_COUNT = 20
        const val MAX_RESULTS = 50
        private const val MAX_TEXT = 200

        /** Text for the car: printable, bounded. */
        fun clean(s: String): String = s.filter { !it.isISOControl() }.trim().take(MAX_TEXT)

        private fun <T> ok(r: ApiResult<T>): T? = (r as? ApiResult.Ok)?.value

        private fun <T> List<T>.slice(start: Int, count: Int): List<T> =
            if (start >= size) emptyList() else subList(start, minOf(size, start + count))
    }
}
