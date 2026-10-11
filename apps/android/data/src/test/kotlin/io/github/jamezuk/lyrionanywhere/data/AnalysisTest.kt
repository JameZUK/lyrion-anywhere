package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AnalysisOutcome
import io.github.jamezuk.lyrionanywhere.core.TrackAnalysisInfo
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

fun analysisInfo(peaks: ByteArray = byteArrayOf(0, 64, -128, -1), duration: Double = 61.0) = TrackAnalysisInfo(
    durationS = duration,
    sampleRate = 44_100u,
    integratedLufs = -14.0,
    loudnessRangeLu = 6.0,
    truePeakDbtp = -0.5,
    peak = peaks,
    rms = peaks,
)

private class ScriptedAnalysis(vararg answers: ApiResult<AnalysisOutcome>) : AnalysisRepository {
    private val queue = ArrayDeque(answers.toList())
    val asked = mutableListOf<Long>()

    override suspend fun track(trackId: Long): ApiResult<AnalysisOutcome> {
        asked += trackId
        return queue.removeFirstOrNull() ?: error("unexpected request for $trackId")
    }
}

class WaveformCacheTest {
    private val sleeps = mutableListOf<Long>()
    private fun cache(repo: AnalysisRepository, maxEntries: Int = 200, maxPolls: Int = 20) =
        WaveformCache(repo, maxEntries, maxPolls) { sleeps += it }

    @Test
    fun aReadyAnalysisIsAWaveformKeptForTheSession() = runTest {
        val repo = ScriptedAnalysis(ApiResult.Ok(AnalysisOutcome.Ready(analysisInfo())))
        val c = cache(repo)
        val w = c.load(7)!!
        assertEquals(7L, w.trackId)
        assertEquals(4, w.size)
        // Bytes are unsigned 0–255.
        assertEquals(listOf(0, 64, 128, 255), (0 until w.size).map { w[it] })
        assertEquals(61.0, w.durationSecs, 0.0)
        assertEquals(w, c.load(7))
        assertEquals(listOf(7L), repo.asked)
    }

    @Test
    fun pendingIsAskedAgainAfterTheSuggestedWaitWithinBounds() = runTest {
        val repo = ScriptedAnalysis(
            ApiResult.Ok(AnalysisOutcome.Pending(2_000uL)),
            ApiResult.Ok(AnalysisOutcome.Pending(10uL)),
            ApiResult.Ok(AnalysisOutcome.Pending(999_999uL)),
            ApiResult.Ok(AnalysisOutcome.Ready(analysisInfo())),
        )
        assertNotNull(cache(repo).load(3))
        assertEquals(listOf(2_000L, 1_000L, 30_000L), sleeps)
        // A limit on polling: still pending → no waveform, and nothing kept.
        val stuck = ScriptedAnalysis(*Array(3) { ApiResult.Ok(AnalysisOutcome.Pending(1_000uL)) })
        val c = cache(stuck, maxPolls = 2)
        assertNull(c.load(3))
        assertEquals(3, stuck.asked.size)
    }

    @Test
    fun refusalsAreKeptAndFailuresTriedAgain() = runTest {
        val repo = ScriptedAnalysis(
            ApiResult.Ok(AnalysisOutcome.Unavailable("not analysable")),
            ApiResult.Err(AppError.Offline()),
            ApiResult.Ok(AnalysisOutcome.Ready(analysisInfo())),
            ApiResult.Ok(AnalysisOutcome.Ready(analysisInfo(peaks = ByteArray(0)))),
        )
        val c = cache(repo)
        assertNull(c.load(1))
        assertNull(c.load(1)) // kept: not asked again
        assertNull(c.load(2)) // offline: not kept
        assertNotNull(c.load(2))
        assertNull(c.load(5)) // an empty waveform draws nothing: the plain bar
        assertEquals(listOf(1L, 2L, 2L, 5L), repo.asked)
    }

    @Test
    fun badIdsAreNeverAskedForAndTheCacheIsBounded() = runTest {
        val repo = ScriptedAnalysis(*Array(3) { ApiResult.Ok(AnalysisOutcome.Ready(analysisInfo())) })
        val c = cache(repo, maxEntries = 2)
        assertNull(c.load(0))
        assertNull(c.load(-4))
        assertTrue(repo.asked.isEmpty())
        c.load(1)
        c.load(2)
        c.load(3)
        assertEquals(listOf(1L, 2L, 3L), repo.asked)
        // 1 was dropped: asking again fetches (and fails here: the script is empty).
        val r = runCatching { c.load(1) }
        assertTrue(r.isFailure)
    }
}
