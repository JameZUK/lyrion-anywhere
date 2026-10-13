package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.FilterAlbums
import io.github.jamezuk.lyrionanywhere.core.FilterRule
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.SmartFilter
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

private val signedOut = object : ConnectionSource {
    override val connection: StateFlow<Connection?> = MutableStateFlow(null)
}

private val good = FilterDraft("Late night jazz", listOf(FilterRule.Genre(listOf(4)), FilterRule.Decade(1990u)), AlbumSort.TITLE, 100)

/**
 * The repository refuses what the gateway would refuse before anything is sent: a refusal is
 * InvalidInput, never SignedOut (which would mean it got as far as the connection).
 */
class RustFiltersRepositoryTest {
    private val checked = mutableListOf<Triple<String, List<FilterRule>, UInt>>()
    private var answer: String? = null
    private val repo = RustFiltersRepository(signedOut) { n, r, l ->
        checked += Triple(n, r, l)
        answer
    }

    @Test
    fun aRefusedFilterIsNeverSent() = runTest {
        answer = "a filter needs a name of 1 to 64 characters"
        val created = repo.create(good.copy(name = ""))
        assertEquals(ApiResult.Err(AppError.InvalidInput("a filter needs a name of 1 to 64 characters")), created)
        val updated = repo.update(7, good.copy(name = ""))
        assertEquals(ApiResult.Err(AppError.InvalidInput("a filter needs a name of 1 to 64 characters")), updated)
        assertEquals("the Rust check saw exactly what would be sent", Triple("", good.rules, 100u), checked.first())
    }

    @Test
    fun aGoodFilterGoesOnToTheServer() = runTest {
        assertEquals(ApiResult.Err(AppError.SignedOut), repo.create(good))
        assertEquals(ApiResult.Err(AppError.SignedOut), repo.update(7, good))
        assertEquals(2, checked.size)
    }

    @Test
    fun limitsOutsideOneToFiveHundredAreRefusedHere() = runTest {
        for (limit in listOf(0, -1, 501, Int.MIN_VALUE, Int.MAX_VALUE)) {
            val r = repo.create(good.copy(limit = limit))
            assertTrue("limit $limit", (r as ApiResult.Err).error is AppError.InvalidInput)
        }
        // Never wrapped into a huge unsigned number for the check.
        assertTrue(checked.isEmpty())
    }

    @Test
    fun badIdsAreRefusedHere() = runTest {
        for (id in listOf(0L, -1L, 1_000_000_000_000L, Long.MIN_VALUE, Long.MAX_VALUE)) {
            assertTrue("update $id", (repo.update(id, good) as ApiResult.Err).error is AppError.InvalidInput)
            assertTrue("delete $id", (repo.delete(id) as ApiResult.Err).error is AppError.InvalidInput)
            assertTrue("albums $id", (repo.albums(id, 0, 50) as ApiResult.Err).error is AppError.InvalidInput)
        }
        assertEquals(ApiResult.Err(AppError.SignedOut), repo.delete(7))
    }

    @Test
    fun pagesAreCheckedAsTheGatewayChecksThem() = runTest {
        for ((from, count) in listOf(-1 to 50, 501 to 50, 0 to 0, 0 to 101, 0 to -5, Int.MAX_VALUE to 1)) {
            val r = repo.albums(7, from, count)
            assertTrue("from $from count $count", (r as ApiResult.Err).error is AppError.InvalidInput)
        }
        assertEquals(ApiResult.Err(AppError.SignedOut), repo.albums(7, 0, 100))
        assertEquals(ApiResult.Err(AppError.SignedOut), repo.albums(7, 500, 1))
    }

    @Test
    fun theDecadesAreTheContractsOwn() {
        assertEquals(20, FilterLimits.DECADES.size)
        assertEquals(1900, FilterLimits.DECADES.first())
        assertEquals(2090, FilterLimits.DECADES.last())
        assertTrue(FilterLimits.DECADES.all { it % 10 == 0 })
    }
}

/** A filter repository that gives one page of album ids. */
private class PagedFilters(var page: ApiResult<FilterAlbums>) : FiltersRepository {
    val asked = mutableListOf<Triple<Long, Int, Int>>()
    override suspend fun filters() = ApiResult.Ok(emptyList<SmartFilter>())
    override suspend fun create(draft: FilterDraft): ApiResult<SmartFilter> = ApiResult.Err(AppError.Unexpected("x"))
    override suspend fun update(id: Long, draft: FilterDraft): ApiResult<SmartFilter> = ApiResult.Err(AppError.Unexpected("x"))
    override suspend fun delete(id: Long) = ApiResult.Ok(Unit)
    override suspend fun albums(id: Long, from: Int, count: Int): ApiResult<FilterAlbums> {
        asked += Triple(id, from, count)
        return page
    }
    override fun problem(draft: FilterDraft): String? = null
}

class FilterPlayTest {
    private val sent = mutableListOf<String>()
    private var failOn: String? = null

    private suspend fun play(filters: FiltersRepository, shuffle: Boolean) = FilterPlay.play(
        filters,
        7,
        shuffle,
        shuffleOn = {
            sent += "shuffle"
            if (failOn == "shuffle") ApiResult.Err(AppError.Offline()) else ApiResult.Ok(Unit)
        },
        enqueue = { album, placement ->
            sent += "$album $placement"
            if (failOn == "$album") ApiResult.Err(AppError.Server(500, "x")) else ApiResult.Ok(Unit)
        },
    )

    @Test
    fun theFirstAlbumPlaysAndTheRestAreAdded() = runTest {
        val f = PagedFilters(ApiResult.Ok(FilterAlbums(listOf(3, 9, 4), 3u)))
        assertEquals(ApiResult.Ok(3), play(f, shuffle = false))
        assertEquals(listOf("3 ${Placement.PLAY_NOW}", "9 ${Placement.ADD_TO_QUEUE}", "4 ${Placement.ADD_TO_QUEUE}"), sent)
        assertEquals("the first 50, from the start", listOf(Triple(7L, 0, 50)), f.asked)
    }

    @Test
    fun shuffleTurnsOnTrackShuffleFirst() = runTest {
        play(PagedFilters(ApiResult.Ok(FilterAlbums(listOf(3, 9), 2u))), shuffle = true)
        assertEquals(listOf("shuffle", "3 ${Placement.PLAY_NOW}", "9 ${Placement.ADD_TO_QUEUE}"), sent)
    }

    @Test
    fun nothingMatchingSendsNothing() = runTest {
        assertEquals(ApiResult.Ok(0), play(PagedFilters(ApiResult.Ok(FilterAlbums(emptyList(), 0u))), shuffle = true))
        assertTrue(sent.isEmpty())
    }

    @Test
    fun aFailureStopsThePlay() = runTest {
        failOn = "9"
        val r = play(PagedFilters(ApiResult.Ok(FilterAlbums(listOf(3, 9, 4), 3u))), shuffle = false)
        assertEquals(ApiResult.Err(AppError.Server(500, "x")), r)
        assertEquals("nothing after the failure", 2, sent.size)
        sent.clear()
        failOn = "shuffle"
        assertTrue(play(PagedFilters(ApiResult.Ok(FilterAlbums(listOf(3), 1u))), shuffle = true) is ApiResult.Err)
        assertEquals("no albums once shuffle failed", listOf("shuffle"), sent)
    }

    @Test
    fun aFilterThatCantBeReadSendsNothing() = runTest {
        val r = play(PagedFilters(ApiResult.Err(AppError.Server(404, "gone"))), shuffle = true)
        assertEquals(ApiResult.Err(AppError.Server(404, "gone")), r)
        assertTrue(sent.isEmpty())
    }

    @Test
    fun onlyValidAlbumsOnceEachAndAtMostFifty() {
        assertEquals(listOf(3L, 9L), FilterPlay.albumsToPlay(listOf(3, 0, -4, 9, 3, 1_000_000_000_000)))
        assertEquals(50, FilterPlay.albumsToPlay((1L..120L).toList()).size)
        assertEquals((1L..50L).toList(), FilterPlay.albumsToPlay((1L..120L).toList()))
    }

    /** Whatever ids a confused or hostile server sends, the play is bounded and well formed. */
    @Test
    fun arbitraryIdsNeverOverflowThePlay() = runTest {
        val rnd = java.util.Random(5)
        repeat(200) {
            sent.clear()
            val ids = List(rnd.nextInt(300)) { rnd.nextLong() % 2_000_000_000_000 }
            val r = play(PagedFilters(ApiResult.Ok(FilterAlbums(ids, rnd.nextInt().toUInt()))), shuffle = rnd.nextBoolean())
            val n = (r as ApiResult.Ok).value
            assertTrue(n in 0..50)
            assertEquals(n, sent.count { it != "shuffle" })
            assertTrue(sent.filter { it != "shuffle" }.drop(1).none { it.endsWith("${Placement.PLAY_NOW}") })
        }
    }
}
