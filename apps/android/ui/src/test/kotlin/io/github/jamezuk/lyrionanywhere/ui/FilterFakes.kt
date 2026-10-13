package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.FilterAlbums
import io.github.jamezuk.lyrionanywhere.core.FilterRule
import io.github.jamezuk.lyrionanywhere.core.SmartFilter
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.FilterDraft
import io.github.jamezuk.lyrionanywhere.data.FiltersRepository

/** Invented filters, as fixtures. */
val LATE_JAZZ = SmartFilter(11, "Late night jazz", listOf(FilterRule.Genre(listOf(4, 5)), FilterRule.Decade(1960u)), AlbumSort.YEAR, 40u)
val COMPOSERS = SmartFilter(
    12,
    "Example Composer",
    listOf(FilterRule.Artist(3, ArtistRole.COMPOSER), FilterRule.Compilation(false)),
    AlbumSort.RANDOM,
    100u,
)

/**
 * The gateway's `/v1/me/filters`, in memory: [albumCount] albums for every filter (ids from
 * 100), and the same checks as the Rust layer for what it refuses (a subset, enough for the UI).
 */
class FakeFilters(var list: List<SmartFilter> = listOf(LATE_JAZZ, COMPOSERS)) : FiltersRepository {
    var albumCount = 120
    var failList: AppError? = null
    var failSave: AppError? = null
    var failDelete: AppError? = null
    var failAlbums: AppError? = null
    val created = mutableListOf<FilterDraft>()
    val updated = mutableListOf<Pair<Long, FilterDraft>>()
    val deleted = mutableListOf<Long>()
    val pages = mutableListOf<Triple<Long, Int, Int>>()
    var listCalls = 0

    override suspend fun filters(): ApiResult<List<SmartFilter>> {
        listCalls++
        return failList?.let { ApiResult.Err(it) } ?: ApiResult.Ok(list)
    }

    private fun saved(id: Long, d: FilterDraft) = SmartFilter(id, d.name, d.rules, d.sort, d.limit.toUInt())

    override suspend fun create(draft: FilterDraft): ApiResult<SmartFilter> {
        problem(draft)?.let { return ApiResult.Err(AppError.InvalidInput(it)) }
        failSave?.let { return ApiResult.Err(it) }
        created += draft
        val f = saved(90L + created.size, draft)
        list = list + f
        return ApiResult.Ok(f)
    }

    override suspend fun update(id: Long, draft: FilterDraft): ApiResult<SmartFilter> {
        problem(draft)?.let { return ApiResult.Err(AppError.InvalidInput(it)) }
        failSave?.let { return ApiResult.Err(it) }
        updated += id to draft
        val f = saved(id, draft)
        list = list.map { if (it.id == id) f else it }
        return ApiResult.Ok(f)
    }

    override suspend fun delete(id: Long): ApiResult<Unit> {
        failDelete?.let { return ApiResult.Err(it) }
        deleted += id
        list = list.filterNot { it.id == id }
        return ApiResult.Ok(Unit)
    }

    override suspend fun albums(id: Long, from: Int, count: Int): ApiResult<FilterAlbums> {
        pages += Triple(id, from, count)
        failAlbums?.let { return ApiResult.Err(it) }
        if (list.none { it.id == id }) return ApiResult.Err(AppError.Server(404, "no such filter"))
        val ids = (from until minOf(from + count, albumCount)).map { 100L + it }
        return ApiResult.Ok(FilterAlbums(ids, albumCount.toUInt()))
    }

    override fun problem(draft: FilterDraft): String? = when {
        draft.name.isBlank() || draft.name.length > 64 -> "a filter needs a name of 1 to 64 characters"
        draft.rules.isEmpty() || draft.rules.size > 8 -> "a filter has 1 to 8 rules"
        draft.rules.any { it is FilterRule.Genre && (it.genres.isEmpty() || it.genres.size > 20) } -> "choose 1 to 20 genres"
        draft.limit !in 1..500 -> "a filter shows 1 to 500 albums"
        else -> null
    }
}
