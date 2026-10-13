package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.BusiestDay
import io.github.jamezuk.lyrionanywhere.data.ListeningStats
import io.github.jamezuk.lyrionanywhere.data.MonthTotal
import io.github.jamezuk.lyrionanywhere.data.NewArtist
import io.github.jamezuk.lyrionanywhere.data.StatsPeriod
import io.github.jamezuk.lyrionanywhere.data.StatsRange
import io.github.jamezuk.lyrionanywhere.data.StatsRepository
import io.github.jamezuk.lyrionanywhere.data.StatsTotals
import io.github.jamezuk.lyrionanywhere.data.Streak
import io.github.jamezuk.lyrionanywhere.data.TopArtist
import io.github.jamezuk.lyrionanywhere.data.TopEntry
import kotlinx.coroutines.CompletableDeferred

/** The gateway's `/v1/me/stats`, in memory: answers by period, and every call recorded. */
class FakeStats(var answer: (StatsPeriod) -> ApiResult<ListeningStats> = { ApiResult.Ok(sampleStats()) }) : StatsRepository {
    val calls = mutableListOf<Pair<StatsPeriod, Int>>()

    /** When set, calls wait for it (to test answers arriving late). */
    var gate: CompletableDeferred<Unit>? = null

    override suspend fun stats(period: StatsPeriod, tzMinutes: Int): ApiResult<ListeningStats> {
        calls += period to tzMinutes
        gate?.await()
        return answer(period)
    }
}

/** Invented names only. */
fun sampleStats(
    plays: Long = 412,
    seconds: Long = 82_800,
    artists: Int = 12,
    albums: Int = 12,
    tracks: Int = 12,
    byMonth: List<MonthTotal> = emptyList(),
    enabled: Boolean = true,
    retentionDays: Int = 396,
) = ListeningStats(
    enabled = enabled,
    retentionDays = retentionDays,
    period = StatsRange(1_790_000_000, 1_792_600_000),
    totals = StatsTotals(plays, seconds, 301, 64, 41, 27),
    topArtists = List(artists) { i -> TopArtist(100L + i, if (i == 0) "An Example Band" else "Example Artist ${i + 1}", 40L - i, 9_100L - i * 100) },
    topAlbums = List(albums) { i -> TopEntry(7L + i, 22L - i, 5_200L - i * 100) },
    topTracks = List(tracks) { i -> TopEntry(9L + i, 6, 1_300) },
    newArtists = listOf(NewArtist(31, "Another Example", 1_791_000_000)),
    byHour = List(24) { h -> if (h == 21) 48L else if (h == 20) 30L else if (h == 8) 12L else 0L },
    byWeekday = listOf(12, 3, 20, 25, 30, 120, 60),
    byMonth = byMonth,
    longestStreak = Streak(9, "2026-10-02", "2026-10-10"),
    busiestDay = BusiestDay("2026-10-04", 14_400),
)

fun emptyStats(enabled: Boolean = true) = ListeningStats(
    enabled = enabled,
    retentionDays = 396,
    period = StatsRange(0, 0),
    totals = StatsTotals(),
    topArtists = emptyList(),
    topAlbums = emptyList(),
    topTracks = emptyList(),
    newArtists = emptyList(),
    byHour = List(24) { 0L },
    byWeekday = List(7) { 0L },
    byMonth = emptyList(),
    longestStreak = null,
    busiestDay = null,
)

val YEAR_MONTHS = listOf(MonthTotal("2026-01", 30, 6_000), MonthTotal("2026-02", 12, 2_000), MonthTotal("2026-10", 412, 82_800))
