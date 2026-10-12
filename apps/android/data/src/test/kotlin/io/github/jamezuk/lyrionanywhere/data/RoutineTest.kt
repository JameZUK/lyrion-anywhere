package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private class ScriptedRatings(var answer: ApiResult<List<TrackRating>> = ApiResult.Ok(emptyList())) : RatingsRepository {
    val asked = mutableListOf<List<Long>>()
    val rated = mutableListOf<Pair<Long, Int>>()
    var rateAnswer: ApiResult<TrackRating?> = ApiResult.Ok(null)

    override suspend fun style() = ApiResult.Ok(RatingStyle.STARS)
    override suspend fun setStyle(style: RatingStyle) = ApiResult.Ok(Unit)
    override suspend fun ratings(trackIds: List<Long>): ApiResult<List<TrackRating>> {
        asked += trackIds
        return answer
    }
    override suspend fun rate(trackId: Long, value: Int): ApiResult<TrackRating?> {
        rated += trackId to value
        return rateAnswer
    }
    override suspend fun clear() = ApiResult.Ok(0L)
}

class RatingsCacheTest {
    @Test
    fun ratingsAreKeptAndARatingUpdatesTheCache() = runTest {
        val repo = ScriptedRatings(ApiResult.Ok(listOf(TrackRating(9, null, 60u, 60u))))
        val c = RatingsCache(repo)
        assertEquals(60u, c.get(9)?.shared)
        c.get(9)
        assertEquals(listOf(listOf(9L)), repo.asked)
        repo.rateAnswer = ApiResult.Ok(TrackRating(9, 80u, 60u, 75u))
        c.rate(9, 80)
        assertEquals(80u, c.get(9)?.mine)
        assertEquals("the rating answer is kept", 1, repo.asked.size)
        assertNull(c.get(0))
        assertNull(c.get(-5))
    }

    @Test
    fun failuresAreNotKept() = runTest {
        val repo = ScriptedRatings(ApiResult.Err(AppError.Offline()))
        val c = RatingsCache(repo)
        assertNull(c.get(9))
        assertNull(c.get(9))
        assertEquals(2, repo.asked.size)
    }
}

class RatingsOfflineTest {
    private val noConnection = object : ConnectionSource {
        override val connection: StateFlow<Connection?> = MutableStateFlow(null)
    }

    @Test
    fun aDownloadedTrackRatedOfflineIsQueued() = runTest {
        val queued = mutableListOf<Pair<Long, Int>>()
        val repo = RustRatingsRepository(noConnection) { id, v ->
            queued += id to v
            id == 9L // only track 9 is downloaded
        }
        val r = repo.rate(9, 84)
        assertTrue(r is ApiResult.Ok)
        assertEquals(80u, (r as ApiResult.Ok).value?.mine)
        assertEquals(listOf(9L to 84), queued)
        // Not downloaded: the failure stands.
        assertTrue(repo.rate(7, 50) is ApiResult.Err)
        // Clearing offline works too.
        assertNull((repo.rate(9, 0) as ApiResult.Ok).value?.mine)
    }
}
