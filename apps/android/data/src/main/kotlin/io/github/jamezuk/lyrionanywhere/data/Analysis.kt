package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AnalysisOutcome
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/** Track analyses from the gateway (waveform, loudness), decoded and checked by the Rust layer. */
interface AnalysisRepository {
    suspend fun track(trackId: Long): ApiResult<AnalysisOutcome>
}

class RustAnalysisRepository(private val source: ConnectionSource) : AnalysisRepository {
    override suspend fun track(trackId: Long): ApiResult<AnalysisOutcome> {
        val analysis = source.connection.value?.analysis ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { analysis.track(trackId) }
    }
}

/** A track's waveform for the seek bar: bucket peaks, 0 (silence) to 255 (full scale). */
class Waveform(val trackId: Long, private val peaks: ByteArray, val durationSecs: Double) {
    val size: Int get() = peaks.size

    /** Bucket `i` as 0–255. */
    operator fun get(i: Int): Int = peaks[i].toInt() and 0xff

    override fun equals(other: Any?): Boolean =
        other is Waveform && other.trackId == trackId && other.peaks.contentEquals(peaks) &&
            other.durationSecs == durationSecs

    override fun hashCode(): Int = 31 * trackId.hashCode() + peaks.contentHashCode()
}

/**
 * Waveforms for one sign-in: cached (the most recent [maxEntries]), a pending analysis asked
 * for again after the gateway's suggested wait (at most [maxPolls] times), refusals kept
 * ("no waveform for this track"), and transient failures (offline) tried again next time.
 */
class WaveformCache(
    private val repo: AnalysisRepository,
    private val maxEntries: Int = 200,
    private val maxPolls: Int = 20,
    private val sleep: suspend (Long) -> Unit = { delay(it) },
) {
    private val mutex = Mutex()

    /** Track id → waveform, or null for "none" (access order: least recently used first). */
    private val results = LinkedHashMap<Long, Waveform?>(16, 0.75f, true)

    /** The waveform of a track, fetching (and waiting while it is analysed) if need be. */
    suspend fun load(trackId: Long): Waveform? {
        if (trackId <= 0) return null
        mutex.withLock {
            if (results.containsKey(trackId)) return results[trackId]
        }
        for (attempt in 0..maxPolls) {
            when (val r = repo.track(trackId)) {
                is ApiResult.Err -> return null
                is ApiResult.Ok -> when (val o = r.value) {
                    is AnalysisOutcome.Ready -> {
                        val a = o.analysis
                        val w = if (a.peak.isEmpty()) null else Waveform(trackId, a.peak, a.durationS)
                        return keep(trackId, w)
                    }
                    is AnalysisOutcome.Unavailable -> return keep(trackId, null)
                    is AnalysisOutcome.Pending -> {
                        if (attempt == maxPolls) return null
                        sleep(o.retryAfterMs.toLong().coerceIn(1_000L, 30_000L))
                    }
                }
            }
        }
        return null
    }

    suspend fun clear() = mutex.withLock { results.clear() }

    private suspend fun keep(trackId: Long, w: Waveform?): Waveform? = mutex.withLock {
        results[trackId] = w
        while (results.size > maxEntries) {
            results.remove(results.keys.first())
        }
        w
    }
}
