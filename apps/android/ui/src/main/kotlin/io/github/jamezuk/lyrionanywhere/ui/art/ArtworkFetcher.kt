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
import okio.Buffer

/** What to load: an artwork key from a record, at a size in pixels. */
data class Artwork(val key: String, val sizePx: Int)

class ArtworkException(val error: AppError) : Exception("artwork unavailable")

/**
 * Coil fetcher for [Artwork]: the bytes come from the Rust Art API, which adds the bearer
 * token and enforces the size cap. Images are cached in memory by Coil (keyed by
 * [ArtworkKeyer]); nothing touches the disk.
 */
class ArtworkFetcher(
    private val data: Artwork,
    private val options: Options,
    private val art: ArtRepository,
) : Fetcher {
    override suspend fun fetch(): FetchResult = when (val r = art.image(data.key, data.sizePx)) {
        is ApiResult.Ok -> SourceFetchResult(
            source = ImageSource(Buffer().write(r.value.bytes), options.fileSystem),
            mimeType = r.value.mimeType,
            dataSource = DataSource.NETWORK,
        )
        is ApiResult.Err -> throw ArtworkException(r.error)
    }

    class Factory(private val art: ArtRepository) : Fetcher.Factory<Artwork> {
        override fun create(data: Artwork, options: Options, imageLoader: ImageLoader): Fetcher =
            ArtworkFetcher(data, options, art)
    }
}

/** Memory-cache key: the artwork key and size (never anything secret). */
class ArtworkKeyer : Keyer<Artwork> {
    override fun key(data: Artwork, options: Options): String = "${data.key}@${data.sizePx}"
}
