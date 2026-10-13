package io.github.jamezuk.lyrionanywhere.ui.stats

import android.content.res.Resources
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.ListeningStats
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.data.NewArtist
import io.github.jamezuk.lyrionanywhere.data.STATS_NOT_AVAILABLE
import io.github.jamezuk.lyrionanywhere.data.StatsPeriod
import io.github.jamezuk.lyrionanywhere.data.StatsRepository
import io.github.jamezuk.lyrionanywhere.data.Streak
import io.github.jamezuk.lyrionanywhere.data.TopArtist
import io.github.jamezuk.lyrionanywhere.data.TopEntry
import io.github.jamezuk.lyrionanywhere.data.deviceTzMinutes
import io.github.jamezuk.lyrionanywhere.data.statsPeriodChoices
import io.github.jamezuk.lyrionanywhere.data.yearInSoundOffer
import io.github.jamezuk.lyrionanywhere.ui.R
import java.time.LocalDate
import java.time.YearMonth
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** History kept by default (13 months), until an answer says otherwise. */
internal const val DEFAULT_RETENTION_DAYS = 396

/** The gateway has no stats endpoint (an older gateway), or the app can't ask yet. */
internal fun AppError.statsUnavailable(): Boolean =
    (this is AppError.Server && status == 404) || (this is AppError.Unexpected && detail == STATS_NOT_AVAILABLE)

/** Albums and tracks named by the stats, loaded by id (as for shelves); missing ones are left out. */
private suspend fun resolve(me: MeRepository, stats: ListeningStats, limit: Int): Pair<Map<Long, Album>, Map<Long, Track>> =
    kotlinx.coroutines.coroutineScope {
        val albums = async { (me.albums(stats.topAlbums.take(limit).map { it.id }) as? ApiResult.Ok)?.value.orEmpty() }
        val tracks = async { (me.tracks(stats.topTracks.take(limit).map { it.id }) as? ApiResult.Ok)?.value.orEmpty() }
        albums.await().associateBy { it.id } to tracks.await().mapNotNull { t -> t.id?.let { it to t } }.toMap()
    }

data class StatsUiState(
    val today: LocalDate,
    val period: StatsPeriod,
    val choices: List<StatsPeriod>,
    val loading: Boolean = true,
    val stats: ListeningStats? = null,
    val albums: Map<Long, Album> = emptyMap(),
    val tracks: Map<Long, Track> = emptyMap(),
    val error: AppError? = null,
) {
    /** The gateway doesn't offer stats (yet). */
    val unavailable: Boolean get() = error?.statsUnavailable() == true

    /** "Your year in sound" for the period shown, when it is a year with plays. */
    val storyYear: Int? get() = (period as? StatsPeriod.Year)?.year?.takeIf { stats?.enabled == true && stats.isEmpty.not() }
}

/**
 * The "Your listening" page (docs/plans/wave3-design.md §1): stats for the chosen period,
 * bucketed in the device's time zone, with the most played albums and tracks loaded by id.
 */
class StatsViewModel(
    private val stats: StatsRepository,
    private val me: MeRepository,
    private val today: () -> LocalDate = LocalDate::now,
    private val tz: () -> Int = { deviceTzMinutes() },
    initial: StatsPeriod? = null,
) : ViewModel() {
    private val _state: MutableStateFlow<StatsUiState>
    val state: StateFlow<StatsUiState>
    private var job: Job? = null

    init {
        val d = today()
        val first = initial ?: StatsPeriod.Month(d.year, d.monthValue)
        val choices = statsPeriodChoices(d, DEFAULT_RETENTION_DAYS).let { if (first in it) it else it + first }
        _state = MutableStateFlow(StatsUiState(d, first, choices))
        state = _state.asStateFlow()
        load()
    }

    fun select(period: StatsPeriod) {
        if (period == _state.value.period && _state.value.stats != null) return
        _state.update { it.copy(period = period) }
        load()
    }

    fun retry() = load()

    private fun load() {
        job?.cancel()
        val period = _state.value.period
        _state.update { it.copy(loading = true, error = null) }
        job = viewModelScope.launch {
            when (val r = stats.stats(period, tz())) {
                is ApiResult.Ok -> {
                    val s = r.value.normalised()
                    val (albums, tracks) = resolve(me, s, ListeningStats.MAX_LIST)
                    val d = _state.value.today
                    _state.update {
                        it.copy(
                            loading = false,
                            stats = s,
                            albums = albums,
                            tracks = tracks,
                            choices = statsPeriodChoices(d, s.retentionDays).let { c -> if (period in c) c else c + period },
                        )
                    }
                }
                is ApiResult.Err -> _state.update { it.copy(loading = false, stats = null, error = r.error) }
            }
        }
    }
}

/** The card at the end of Home: this month's listening, and "Your year in sound" in season. */
data class ListeningCardState(
    val loaded: Boolean = false,
    /** This month's listening (zero: the line is left out). */
    val monthSeconds: Long = 0,
    val monthPlays: Long = 0,
    /** The year offered as a story (1 December to 31 January, when it has plays). */
    val storyYear: Int? = null,
) {
    /** Hidden while loading, with history off, with nothing played, and on any error. */
    val visible: Boolean get() = loaded && (monthPlays > 0 || storyYear != null)
}

class ListeningCardViewModel(
    private val stats: StatsRepository,
    private val today: () -> LocalDate = LocalDate::now,
    private val tz: () -> Int = { deviceTzMinutes() },
) : ViewModel() {
    private val _state = MutableStateFlow(ListeningCardState())
    val state: StateFlow<ListeningCardState> = _state.asStateFlow()
    private var job: Job? = null

    /** Load (again): called as Home is shown. The gateway caches answers for 5 minutes. */
    fun refresh() {
        if (job?.isActive == true) return
        job = viewModelScope.launch {
            val d = today()
            val offset = tz()
            val month = (stats.stats(StatsPeriod.Month(d.year, d.monthValue), offset) as? ApiResult.Ok)?.value?.normalised()
            if (month == null || !month.enabled) {
                _state.value = ListeningCardState(loaded = true)
                return@launch
            }
            val offer = yearInSoundOffer(d)
            val storyYear = offer?.let { y ->
                val year = if (y == d.year) null else (stats.stats(StatsPeriod.Year(y), offset) as? ApiResult.Ok)?.value?.normalised()
                // This year's month has plays, so this year has some; last year needs asking.
                when {
                    y == d.year -> y.takeIf { !month.isEmpty }
                    year != null && year.enabled && !year.isEmpty -> y
                    else -> null
                }
            }
            _state.value = ListeningCardState(true, month.totals.seconds, month.totals.plays, storyYear)
        }
    }
}

/** One card of "Your year in sound". */
sealed interface StoryCard {
    data class Hours(val seconds: Long, val plays: Long, val days: Long) : StoryCard

    data class TopArtistCard(val artist: TopArtist) : StoryCard

    data class TopAlbum(val entry: TopEntry, val album: Album?) : StoryCard

    data class TopTrack(val entry: TopEntry, val track: Track?) : StoryCard

    data class TopFive(val artists: List<TopArtist>) : StoryCard

    data class Discovery(val artist: NewArtist) : StoryCard

    data class BusiestHour(val hour: Int, val plays: Long) : StoryCard

    data class LongestStreak(val streak: Streak) : StoryCard

    data class Summary(val seconds: Long, val plays: Long, val artists: Long, val albums: Long, val topArtist: String?) : StoryCard
}

/**
 * The story's cards for a year, in order: total hours, top artist, top album, top track, top
 * five artists, a new discovery, the busiest hour, the longest streak and a closing summary.
 * Cards with nothing to show are left out; no plays, no cards.
 */
fun storyCards(stats: ListeningStats, albums: Map<Long, Album>, tracks: Map<Long, Track>): List<StoryCard> {
    if (!stats.enabled || stats.isEmpty) return emptyList()
    val t = stats.totals
    val cards = mutableListOf<StoryCard>(StoryCard.Hours(t.seconds, t.plays, t.days))
    stats.topArtists.firstOrNull()?.let { cards += StoryCard.TopArtistCard(it) }
    stats.topAlbums.firstOrNull()?.let { cards += StoryCard.TopAlbum(it, albums[it.id]) }
    stats.topTracks.firstOrNull()?.let { cards += StoryCard.TopTrack(it, tracks[it.id]) }
    if (stats.topArtists.size > 1) cards += StoryCard.TopFive(stats.topArtists.take(5))
    stats.newArtists.firstOrNull()?.let { cards += StoryCard.Discovery(it) }
    busiest(stats.byHour)?.let { (hour, plays) -> cards += StoryCard.BusiestHour(hour, plays) }
    stats.longestStreak?.let { cards += StoryCard.LongestStreak(it) }
    cards += StoryCard.Summary(t.seconds, t.plays, t.artists, t.albums, stats.topArtists.firstOrNull()?.name)
    return cards
}

/** The index and count of the largest value (the first on a tie), or null when all are zero. */
internal fun busiest(values: List<Long>): Pair<Int, Long>? {
    val max = values.maxOrNull() ?: return null
    if (max <= 0) return null
    return values.indexOf(max) to max
}

data class StoryUiState(
    val year: Int,
    val loading: Boolean = true,
    val cards: List<StoryCard> = emptyList(),
    val enabled: Boolean = true,
    val error: AppError? = null,
) {
    val unavailable: Boolean get() = error?.statsUnavailable() == true
}

/** "Your year in sound" for [year]: on demand for any year with history. */
class YearStoryViewModel(
    private val stats: StatsRepository,
    private val me: MeRepository,
    val year: Int,
    private val tz: () -> Int = { deviceTzMinutes() },
) : ViewModel() {
    private val _state = MutableStateFlow(StoryUiState(year))
    val state: StateFlow<StoryUiState> = _state.asStateFlow()
    private var job: Job? = null

    init {
        load()
    }

    fun retry() = load()

    private fun load() {
        job?.cancel()
        _state.update { it.copy(loading = true, error = null) }
        job = viewModelScope.launch {
            when (val r = stats.stats(StatsPeriod.Year(year), tz())) {
                is ApiResult.Ok -> {
                    val s = r.value.normalised()
                    val (albums, tracks) = resolve(me, s, 1)
                    _state.value = StoryUiState(year, loading = false, cards = storyCards(s, albums, tracks), enabled = s.enabled)
                }
                is ApiResult.Err -> _state.update { it.copy(loading = false, error = r.error) }
            }
        }
    }
}

/* Formatting, usable outside composition (and so in tests and image sharing). */

/** "45 min", "2 h 15 min" (under 10 hours) or "23 h". */
fun listenTime(res: Resources, seconds: Long): String {
    val minutes = (seconds.coerceAtLeast(0) / 60).toInt()
    val hours = minutes / 60
    return when {
        hours == 0 -> res.getString(R.string.stats_minutes, minutes)
        hours < 10 && minutes % 60 != 0 -> res.getString(R.string.stats_hours_minutes, hours, minutes % 60)
        else -> res.getString(R.string.stats_hours, hours)
    }
}

fun playsText(res: Resources, plays: Long): String {
    val n = plays.coerceIn(0, Int.MAX_VALUE.toLong()).toInt()
    return res.getQuantityString(R.plurals.stats_plays, n, n)
}

fun daysText(res: Resources, days: Long): String {
    val n = days.coerceIn(0, Int.MAX_VALUE.toLong()).toInt()
    return res.getQuantityString(R.plurals.stats_days, n, n)
}

/** The period's name in the picker. */
fun periodLabel(res: Resources, period: StatsPeriod, today: LocalDate): String = when (period) {
    is StatsPeriod.Month ->
        if (period.year == today.year && period.month == today.monthValue) {
            res.getString(R.string.stats_period_month)
        } else {
            monthName(period.year, period.month)
        }
    is StatsPeriod.LastDays -> res.getQuantityString(R.plurals.stats_period_days, period.days, period.days)
    is StatsPeriod.Year -> if (period.year == today.year) res.getString(R.string.stats_period_year) else period.year.toString()
    StatsPeriod.All -> res.getString(R.string.stats_period_all)
}

/** "October 2026". */
fun monthName(year: Int, month: Int): String =
    YearMonth.of(year, month.coerceIn(1, 12)).format(DateTimeFormatter.ofPattern("LLLL yyyy", Locale.getDefault()))

/** A "YYYY-MM-DD" date from the gateway in the local style, or as sent if it isn't one. */
fun dayText(date: String): String = runCatching {
    LocalDate.parse(date).format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(Locale.getDefault()))
}.getOrElse { date.take(32) }

/** Unix seconds as a local date. */
fun dayText(epochSeconds: Long, tzMinutes: Int = deviceTzMinutes()): String =
    java.time.Instant.ofEpochSecond(epochSeconds)
        .atOffset(java.time.ZoneOffset.ofTotalSeconds(tzMinutes.coerceIn(-840, 840) * 60))
        .toLocalDate()
        .format(DateTimeFormatter.ofLocalizedDate(FormatStyle.MEDIUM).withLocale(Locale.getDefault()))

/** "21:00". */
fun hourText(hour: Int): String = String.format(Locale.getDefault(), "%02d:00", hour.mod(24))
