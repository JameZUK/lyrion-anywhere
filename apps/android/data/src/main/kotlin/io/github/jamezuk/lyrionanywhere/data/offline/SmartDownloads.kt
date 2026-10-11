package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.DownloadRequest
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository

/**
 * What the smart downloads should hold now: the tracks of the most recently added albums,
 * and of the user's LMS favourites that are library tracks or albums.
 *
 * All or nothing: if any part of the library can't be read, nothing is planned, so a
 * passing failure never removes smart downloads that are still wanted.
 */
class SmartDownloads(private val library: LibraryRepository) {
    suspend fun wanted(settings: DownloadSettings, libraryId: String? = null): ApiResult<List<DownloadRequest>> {
        val out = LinkedHashMap<Long, DownloadRequest>()
        fun add(tracks: List<Track>, albumId: Long?, origin: DownloadOrigin) {
            for (t in tracks) {
                val id = t.id ?: continue
                if (id <= 0 || out.size >= MAX_TRACKS) continue
                out.putIfAbsent(id, DownloadRequest(id, t.albumId ?: albumId, origin))
            }
        }
        if (settings.smartRecentAlbums > 0) {
            val n = settings.smartRecentAlbums.coerceAtMost(DownloadSettings.MAX_RECENT)
            val page = when (val r = library.albums(0, n, AlbumSort.NEWEST, filter = LibraryFilter(libraryId = libraryId))) {
                is ApiResult.Ok -> r.value
                is ApiResult.Err -> return r
            }
            for (album in page.items.take(n)) {
                when (val t = library.albumTracks(album.id)) {
                    is ApiResult.Ok -> add(t.value, album.id, DownloadOrigin.RECENT)
                    is ApiResult.Err -> return t
                }
            }
        }
        if (settings.smartFavourites) {
            val fav = when (val r = library.favouriteLibraryItems()) {
                is ApiResult.Ok -> r.value
                is ApiResult.Err -> return r
            }
            for (id in fav.trackIds.take(MAX_TRACKS)) {
                if (id > 0 && out.size < MAX_TRACKS) out.putIfAbsent(id, DownloadRequest(id, null, DownloadOrigin.FAVOURITE))
            }
            for (albumId in fav.albumIds.take(MAX_FAVOURITE_ALBUMS)) {
                when (val t = library.albumTracks(albumId)) {
                    is ApiResult.Ok -> add(t.value, albumId, DownloadOrigin.FAVOURITE)
                    is ApiResult.Err -> return t
                }
            }
        }
        return ApiResult.Ok(out.values.toList())
    }

    companion object {
        /** Most tracks smart downloads take on. */
        const val MAX_TRACKS = 5000

        /** Most favourite albums expanded. */
        const val MAX_FAVOURITE_ALBUMS = 100
    }
}
