package io.github.jamezuk.lyrionanywhere.data

import java.time.LocalDate
import java.util.Locale
import java.util.TimeZone

/**
 * The listening stats (docs/plans/wave3-design.md §1): `GET /v1/me/stats?period=…&tz=…`, read
 * from the user's own listening history on the gateway. Nothing new is kept.
 */
sealed interface StatsPeriod {
    /** The contract's `period` value. */
    fun wire(): String

    /** The last [days] days (7 to 400). */
    data class LastDays(val days: Int) : StatsPeriod {
        override fun wire() = "days-${days.coerceIn(MIN_DAYS, MAX_DAYS)}"
    }

    /** A calendar month ([month] 1 to 12). */
    data class Month(val year: Int, val month: Int) : StatsPeriod {
        override fun wire() = String.format(Locale.ROOT, "month-%04d-%02d", year.coerceIn(0, 9999), month.coerceIn(1, 12))
    }

    data class Year(val year: Int) : StatsPeriod {
        override fun wire() = String.format(Locale.ROOT, "year-%04d", year.coerceIn(0, 9999))
    }

    /** Everything the history keeps. */
    data object All : StatsPeriod {
        override fun wire() = "all"
    }

    companion object {
        const val MIN_DAYS = 7
        const val MAX_DAYS = 400
    }
}

data class StatsRange(val from: Long, val to: Long)

data class StatsTotals(
    val plays: Long = 0,
    val seconds: Long = 0,
    val tracks: Long = 0,
    val albums: Long = 0,
    val artists: Long = 0,
    /** Days with at least one play. */
    val days: Long = 0,
)

/** A most-played artist: the gateway sends its name (LMS looks artists up one at a time). */
data class TopArtist(val id: Long, val name: String, val plays: Long, val seconds: Long)

/** A most-played album or track, by id: names and art are loaded by id, as for shelves. */
data class TopEntry(val id: Long, val plays: Long, val seconds: Long)

/** An artist first played in the period ([firstPlayed]: Unix seconds). */
data class NewArtist(val id: Long, val name: String, val firstPlayed: Long)

/** One month of a year (or a long period): [month] is "YYYY-MM". */
data class MonthTotal(val month: String, val plays: Long, val seconds: Long)

/** The longest run of days with music ([from] and [to]: "YYYY-MM-DD", in the tz sent). */
data class Streak(val days: Int, val from: String, val to: String)

data class BusiestDay(val date: String, val seconds: Long)

data class ListeningStats(
    /** Is history recorded? Off: zeros and empty lists. */
    val enabled: Boolean,
    /** How far back the history reaches. */
    val retentionDays: Int,
    val period: StatsRange,
    val totals: StatsTotals,
    val topArtists: List<TopArtist>,
    val topAlbums: List<TopEntry>,
    val topTracks: List<TopEntry>,
    val newArtists: List<NewArtist>,
    /** 24 entries, midnight first. */
    val byHour: List<Long>,
    /** 7 entries, Monday first. */
    val byWeekday: List<Long>,
    /** Only for years, everything, and periods over 60 days. */
    val byMonth: List<MonthTotal>,
    val longestStreak: Streak?,
    val busiestDay: BusiestDay?,
) {
    val isEmpty: Boolean get() = totals.plays <= 0

    /**
     * The answer as the UI may rely on it, whatever arrives: lists cut to the contract's
     * lengths, the hour and weekday lists exactly 24 and 7 long, no negative counts, and names
     * trimmed and bounded.
     */
    fun normalised(): ListeningStats {
        fun Long.n() = coerceAtLeast(0)
        fun fixed(values: List<Long>, size: Int) = List(size) { i -> values.getOrNull(i)?.n() ?: 0L }
        fun name(s: String) = s.trim().take(MAX_NAME)
        return copy(
            retentionDays = retentionDays.coerceAtLeast(0),
            totals = StatsTotals(
                totals.plays.n(), totals.seconds.n(), totals.tracks.n(), totals.albums.n(), totals.artists.n(), totals.days.n(),
            ),
            topArtists = topArtists.take(MAX_LIST).map { it.copy(name = name(it.name), plays = it.plays.n(), seconds = it.seconds.n()) },
            topAlbums = topAlbums.take(MAX_LIST).map { it.copy(plays = it.plays.n(), seconds = it.seconds.n()) },
            topTracks = topTracks.take(MAX_LIST).map { it.copy(plays = it.plays.n(), seconds = it.seconds.n()) },
            newArtists = newArtists.take(MAX_LIST).map { it.copy(name = name(it.name)) },
            byHour = fixed(byHour, 24),
            byWeekday = fixed(byWeekday, 7),
            byMonth = byMonth.take(MAX_MONTHS).map { it.copy(plays = it.plays.n(), seconds = it.seconds.n()) },
            longestStreak = longestStreak?.takeIf { it.days > 0 },
            busiestDay = busiestDay?.takeIf { it.seconds > 0 },
        )
    }

    companion object {
        const val MAX_LIST = 20
        const val MAX_NAME = 200

        /** More than enough months for "everything kept" (13 months) and 400 days. */
        const val MAX_MONTHS = 24
    }
}

/** The user's own listening stats. */
interface StatsRepository {
    /** Stats for [period], bucketed by hour and day at [tzMinutes] east of UTC. */
    suspend fun stats(period: StatsPeriod, tzMinutes: Int): ApiResult<ListeningStats>
}

/** Minutes east of UTC, as the contract allows (−840 to 840). */
const val MAX_TZ_MINUTES = 840

/** The device's offset from UTC now, in minutes east, within the contract's range. */
fun deviceTzMinutes(nowMillis: Long = System.currentTimeMillis(), zone: TimeZone = TimeZone.getDefault()): Int =
    (zone.getOffset(nowMillis) / 60_000).coerceIn(-MAX_TZ_MINUTES, MAX_TZ_MINUTES)

/**
 * The periods offered for [today]: this month, the last 30 days, this year, each past year
 * the history (kept [retentionDays] days) reaches into, newest first, and everything kept.
 */
fun statsPeriodChoices(today: LocalDate, retentionDays: Int): List<StatsPeriod> {
    val oldest = today.minusDays(retentionDays.coerceIn(0, 4000).toLong()).year
    val past = (today.year - 1 downTo oldest).map { StatsPeriod.Year(it) }
    return listOf(StatsPeriod.Month(today.year, today.monthValue), StatsPeriod.LastDays(30), StatsPeriod.Year(today.year)) +
        past + StatsPeriod.All
}

/** "Your year in sound" is offered from 1 December to 31 January: for this year, or last year in January. */
fun yearInSoundOffer(today: LocalDate): Int? = when (today.monthValue) {
    12 -> today.year
    1 -> today.year - 1
    else -> null
}

/** The message shown until the Rust stats call is wired (see [RustStatsRepository]). */
const val STATS_NOT_AVAILABLE = "Listening stats are not available yet"

/** The stats through the Rust layer for the signed-in server. */
class RustStatsRepository(private val source: ConnectionSource) : StatsRepository {
    override suspend fun stats(period: StatsPeriod, tzMinutes: Int): ApiResult<ListeningStats> {
        source.connection.value ?: return ApiResult.Err(AppError.SignedOut)
        // TODO(stats-api): wire to the UniFFI call once the mobile layer has it, e.g.
        //   apiCall { c.me.stats(period.toCore(), tzMinutes.coerceIn(-MAX_TZ_MINUTES, MAX_TZ_MINUTES)).toKotlin().normalised() }
        // mapping the core `StatsPeriod` and `ListeningStats` records onto the classes above.
        return ApiResult.Err(AppError.Unexpected(STATS_NOT_AVAILABLE))
    }
}
