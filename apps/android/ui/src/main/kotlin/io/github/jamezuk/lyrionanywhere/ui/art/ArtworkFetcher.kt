package io.github.jamezuk.lyrionanywhere.ui.art

import coil3.ImageLoader
import coil3.decode.DataSource
import coil3.decode.ImageSource
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.SourceFetchResult
import coil3.key.Keyer
import coil3.request.Options
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.AuthState
import okio.Buffer

/** What to load: an artwork key from a record, at a size in pixels. */
data class Artwork(val key: String, val sizePx: Int)

class ArtworkException(val error: AppError) : Exception("artwork unavailable")

/** The artwork key of a downloaded album's cached cover. */
fun offlineArtKey(albumRef: String) = "$OFFLINE_PREFIX$albumRef"

const val OFFLINE_PREFIX = "offline:"

/**
 * Coil fetcher for [Artwork]: the bytes come from the Rust Art API, which adds the bearer
 * token and enforces the size cap. Images are cached in memory by Coil (keyed by
 * [ArtworkKeyer]); nothing touches the disk.
 *
 * Keys made by [offlineArtKey] are downloaded albums' covers, read from the download store
 * (they work offline); the store checks the bytes are an image.
 */
class ArtworkFetcher(
    private val data: Artwork,
    private val options: Options,
    private val art: ArtRepository,
    private val offline: (suspend (String) -> ByteArray?)? = null,
) : Fetcher {
    override suspend fun fetch(): FetchResult {
        if (data.key.startsWith(OFFLINE_PREFIX)) {
            val bytes = offline?.invoke(data.key.removePrefix(OFFLINE_PREFIX))
                ?: throw ArtworkException(AppError.InvalidInput("no cached cover"))
            return SourceFetchResult(
                source = ImageSource(Buffer().write(bytes), options.fileSystem),
                mimeType = null,
                dataSource = DataSource.DISK,
            )
        }
        return when (val r = art.image(data.key, data.sizePx)) {
            is ApiResult.Ok -> SourceFetchResult(
                source = ImageSource(Buffer().write(r.value.bytes), options.fileSystem),
                mimeType = r.value.mimeType,
                dataSource = DataSource.NETWORK,
            )
            is ApiResult.Err -> throw ArtworkException(r.error)
        }
    }

    class Factory(
        private val art: ArtRepository,
        private val offline: (suspend (String) -> ByteArray?)? = null,
    ) : Fetcher.Factory<Artwork> {
        override fun create(data: Artwork, options: Options, imageLoader: ImageLoader): Fetcher =
            ArtworkFetcher(data, options, art, offline)
    }
}

/**
 * Memory-cache key: the server, the artwork key and the size (never anything secret). An
 * artwork key (`music/123/cover`, a downloaded album's reference) names an image only on its
 * own server, so after a change of server the same key must not find the old server's cover.
 * [scope]: the server in use now (its address; empty: none).
 */
class ArtworkKeyer(private val scope: () -> String = { "" }) : Keyer<Artwork> {
    override fun key(data: Artwork, options: Options): String = "${scope()}|${data.key}@${data.sizePx}"
}

/** [ArtworkKeyer]'s scope for an account state: the server in use (empty: none yet). */
fun artScopeOf(state: AuthState): String = when (state) {
    is AuthState.SignedInState -> state.server.url
    is AuthState.SignedOut -> state.server.url
    is AuthState.Restoring -> state.server.url
    is AuthState.Unreachable -> state.server.url
    AuthState.NoServer, AuthState.Starting -> ""
}
