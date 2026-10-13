package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.FilterAlbums
import io.github.jamezuk.lyrionanywhere.core.FilterRule
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.SmartFilter
import io.github.jamezuk.lyrionanywhere.core.filterProblem

/** A smart filter as built in the rule builder, before it is saved (it has no id yet). */
data class FilterDraft(
    val name: String,
    val rules: List<FilterRule>,
    val sort: AlbumSort,
    val limit: Int,
)

/** What the contract allows (docs/plans/wave2-design.md §5.6), as the gateway checks it. */
object FilterLimits {
    const val MAX_FILTERS = 50
    const val MAX_RULES = 8
    const val MAX_GENRES = 20
    const val NAME_MAX = 64
    const val LIMIT_MIN = 1
    const val LIMIT_MAX = 500
    const val DEFAULT_LIMIT = 100

    /** Albums in one page of a filter's results, at most. */
    const val PAGE_MAX = 100

    /** Albums a play or shuffle loads, at most. */
    const val PLAY_MAX = 50

    /** The decades a rule can name: 1900, 1910, … 2090. */
    val DECADES: List<Int> = (1900..2090 step 10).toList()

    /** A valid album, genre or artist id (anything else from the server is ignored). */
    fun validId(id: Long) = id in 1..999_999_999_999
}

/**
 * Smart filters (docs/plans/wave2-design.md §5.6): the user's own rules that produce albums,
 * kept on the gateway and the same on every device. Every filter is checked before it is
 * sent ([problem]); one that would be refused is an [AppError.InvalidInput] and nothing goes
 * to the server.
 */
interface FiltersRepository {
    /** The user's filters, oldest first. */
    suspend fun filters(): ApiResult<List<SmartFilter>>

    /** Save a new filter; it comes back as saved, with its id. */
    suspend fun create(draft: FilterDraft): ApiResult<SmartFilter>

    /** Replace filter [id]. */
    suspend fun update(id: Long, draft: FilterDraft): ApiResult<SmartFilter>

    suspend fun delete(id: Long): ApiResult<Unit>

    /**
     * A page of filter [id]'s album ids: [count] (1–100) from [from] (0–500). The first page
     * runs the filter afresh (a new order for a random one).
     */
    suspend fun albums(id: Long, from: Int, count: Int): ApiResult<FilterAlbums>

    /** What the gateway would refuse in [draft], fit to show; null when nothing. */
    fun problem(draft: FilterDraft): String?
}

/**
 * The Rust layer's filters for the signed-in server. The check is the Rust one
 * (`filter_problem`), the same the calls make; [check] is replaceable so this class is
 * tested on the JVM, without the native library.
 */
class RustFiltersRepository(
    private val source: ConnectionSource,
    private val check: (String, List<FilterRule>, UInt) -> String? = { n, r, l -> filterProblem(n, r, l) },
) : FiltersRepository {
    private suspend fun <T> withMe(block: suspend (Connection) -> T): ApiResult<T> {
        val c = source.connection.value ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(c) }
    }

    override fun problem(draft: FilterDraft): String? {
        if (draft.limit !in FilterLimits.LIMIT_MIN..FilterLimits.LIMIT_MAX) return "a filter shows 1 to 500 albums"
        return check(draft.name, draft.rules, draft.limit.toUInt())
    }

    /** Refused here (nothing sent) when the gateway would refuse it. */
    private fun refused(draft: FilterDraft): ApiResult.Err? = problem(draft)?.let { ApiResult.Err(AppError.InvalidInput(it)) }

    override suspend fun filters() = withMe { it.me.filters() }

    override suspend fun create(draft: FilterDraft): ApiResult<SmartFilter> =
        refused(draft) ?: withMe { it.me.createFilter(draft.name, draft.rules, draft.sort, draft.limit.toUInt()) }

    override suspend fun update(id: Long, draft: FilterDraft): ApiResult<SmartFilter> {
        if (!FilterLimits.validId(id)) return ApiResult.Err(AppError.InvalidInput("bad id"))
        return refused(draft) ?: withMe { it.me.updateFilter(id, draft.name, draft.rules, draft.sort, draft.limit.toUInt()) }
    }

    override suspend fun delete(id: Long): ApiResult<Unit> {
        if (!FilterLimits.validId(id)) return ApiResult.Err(AppError.InvalidInput("bad id"))
        return withMe { it.me.deleteFilter(id) }
    }

    override suspend fun albums(id: Long, from: Int, count: Int): ApiResult<FilterAlbums> {
        if (!FilterLimits.validId(id) || from !in 0..FilterLimits.LIMIT_MAX || count !in 1..FilterLimits.PAGE_MAX) {
            return ApiResult.Err(AppError.InvalidInput("bad page"))
        }
        return withMe { it.me.filterAlbums(id, from.toUInt(), count.toUInt()) }
    }
}

/**
 * Playing a smart filter (Wave 2 §5.6), as the web app does: its first albums, up to 50,
 * through the usual album play path. The first replaces the queue and plays; the rest are
 * added. Shuffle turns on track shuffle first, so the albums load shuffled.
 */
object FilterPlay {
    /** The albums to play from a page of ids: valid ones, each once, at most 50. */
    fun albumsToPlay(ids: List<Long>): List<Long> = ids.filter(FilterLimits::validId).distinct().take(FilterLimits.PLAY_MAX)

    /**
     * Play filter [id]. [shuffleOn] turns on track shuffle; [enqueue] queues one album. Stops
     * at the first failure. Ok with how many albums were queued (0: none match, nothing sent).
     */
    suspend fun play(
        filters: FiltersRepository,
        id: Long,
        shuffle: Boolean,
        shuffleOn: suspend () -> ApiResult<Unit>,
        enqueue: suspend (album: Long, placement: Placement) -> ApiResult<Unit>,
    ): ApiResult<Int> {
        val ids = when (val r = filters.albums(id, 0, FilterLimits.PLAY_MAX)) {
            is ApiResult.Ok -> albumsToPlay(r.value.albums)
            is ApiResult.Err -> return r
        }
        if (ids.isEmpty()) return ApiResult.Ok(0)
        if (shuffle) (shuffleOn() as? ApiResult.Err)?.let { return it }
        ids.forEachIndexed { i, album ->
            (enqueue(album, if (i == 0) Placement.PLAY_NOW else Placement.ADD_TO_QUEUE) as? ApiResult.Err)?.let { return it }
        }
        return ApiResult.Ok(ids.size)
    }
}
