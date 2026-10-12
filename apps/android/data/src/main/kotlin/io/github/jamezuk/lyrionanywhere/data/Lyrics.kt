package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Lyrics from the gateway (docs/plans/wave1-design.md §9): the file's tags, or the Music &
 * Artist Information plugin on the server. Checked by the Rust layer; null when a track has
 * none.
 */
interface LyricsRepository {
    suspend fun track(trackId: Long): ApiResult<TrackLyrics?>
}

class RustLyricsRepository(private val source: ConnectionSource) : LyricsRepository {
    override suspend fun track(trackId: Long): ApiResult<TrackLyrics?> {
        val c = source.connection.value ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { c.me.lyrics(trackId) }
    }
}

/**
 * Lyrics for one sign-in: the most recent [maxEntries] tracks, "none" included (so a track
 * without lyrics isn't asked about again and again). Failures (offline, busy) aren't kept,
 * so they are tried again next time.
 */
class LyricsCache(private val repo: LyricsRepository, private val maxEntries: Int = 50) {
    private val mutex = Mutex()

    /** Track id → lyrics, or null for "none" (access order: least recently used first). */
    private val results = LinkedHashMap<Long, TrackLyrics?>(16, 0.75f, true)

    suspend fun load(trackId: Long): TrackLyrics? {
        if (trackId <= 0) return null
        mutex.withLock {
            if (results.containsKey(trackId)) return results[trackId]
        }
        val found = when (val r = repo.track(trackId)) {
            is ApiResult.Err -> return null
            is ApiResult.Ok -> r.value
        }
        return mutex.withLock {
            results[trackId] = found
            while (results.size > maxEntries) results.remove(results.keys.first())
            found
        }
    }

    suspend fun clear() = mutex.withLock { results.clear() }
}

/** Looking up the playing line (pure, so it is unit-tested). */
object LyricsMath {
    /** The line playing at [posMs]: the last starting at or before it; -1 before the first or for plain lyrics. */
    fun lineAt(lyrics: TrackLyrics?, posMs: Long): Int {
        if (lyrics == null || !lyrics.synced || posMs < 0) return -1
        var lo = 0
        var hi = lyrics.lines.size
        while (lo < hi) {
            val mid = (lo + hi) ushr 1
            if (lyrics.lines[mid].timeMs.toLong() <= posMs) lo = mid + 1 else hi = mid
        }
        return lo - 1
    }
}
