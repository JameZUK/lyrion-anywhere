package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.LyricLine
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

fun syncedLyrics(vararg lines: Pair<Int, String>) =
    TrackLyrics(false, true, lines.map { (t, text) -> LyricLine(t.toUInt(), text) })

private class ScriptedLyrics(vararg answers: ApiResult<TrackLyrics?>) : LyricsRepository {
    private val queue = ArrayDeque(answers.toList())
    val asked = mutableListOf<Long>()

    override suspend fun track(trackId: Long): ApiResult<TrackLyrics?> {
        asked += trackId
        return queue.removeFirstOrNull() ?: error("unexpected request for $trackId")
    }
}

class LyricsCacheTest {
    private val song = syncedLyrics(1000 to "First", 8000 to "Second")

    @Test
    fun lyricsAndNoneAreKeptForTheSession() = runTest {
        val repo = ScriptedLyrics(ApiResult.Ok(song), ApiResult.Ok(null))
        val c = LyricsCache(repo)
        assertEquals(song, c.load(9))
        assertNull(c.load(7))
        assertEquals(song, c.load(9))
        assertNull(c.load(7))
        assertEquals(listOf(9L, 7L), repo.asked)
    }

    @Test
    fun failuresAreTriedAgainLater() = runTest {
        val repo = ScriptedLyrics(ApiResult.Err(AppError.Offline()), ApiResult.Ok(song))
        val c = LyricsCache(repo)
        assertNull(c.load(9))
        assertEquals(song, c.load(9))
        assertEquals(listOf(9L, 9L), repo.asked)
    }

    @Test
    fun badIdsAreNeverAskedFor() = runTest {
        val repo = ScriptedLyrics()
        val c = LyricsCache(repo)
        for (id in listOf(0L, -1L, Long.MIN_VALUE)) assertNull(c.load(id))
        assertEquals(emptyList<Long>(), repo.asked)
    }

    @Test
    fun theCacheIsBounded() = runTest {
        val repo = ScriptedLyrics(*Array(5) { ApiResult.Ok(null) })
        val c = LyricsCache(repo, maxEntries = 3)
        for (id in 1L..4L) c.load(id)
        c.load(1) // forgotten: asked again
        assertEquals(listOf(1L, 2L, 3L, 4L, 1L), repo.asked)
    }
}

class LyricsMathTest {
    private val song = syncedLyrics(1000 to "First", 8000 to "Second", 20000 to "")

    @Test
    fun theLineAtAPosition() {
        val cases = listOf(-5L to -1, 0L to -1, 999L to -1, 1000L to 0, 7999L to 0, 8000L to 1, 1_000_000L to 2)
        for ((ms, want) in cases) assertEquals("$ms", want, LyricsMath.lineAt(song, ms))
    }

    @Test
    fun plainLyricsAndNoneHaveNoCurrentLine() {
        assertEquals(-1, LyricsMath.lineAt(song.copy(synced = false), 5000))
        assertEquals(-1, LyricsMath.lineAt(null, 5000))
        assertEquals(-1, LyricsMath.lineAt(TrackLyrics(false, true, emptyList()), 5000))
    }
}
