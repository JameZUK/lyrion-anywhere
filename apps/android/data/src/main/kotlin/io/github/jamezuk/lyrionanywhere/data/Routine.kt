package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AlarmChange
import io.github.jamezuk.lyrionanywhere.core.AlarmListing
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock

/**
 * Ratings (docs/plans/wave2-design.md §2): the user's own, blended with LMS's shared rating.
 * A rating made while the server can't be reached is queued on the phone for a downloaded
 * track ([offlineRate]) and sent with the plays.
 */
interface RatingsRepository {
    suspend fun style(): ApiResult<RatingStyle>

    suspend fun setStyle(style: RatingStyle): ApiResult<Unit>

    suspend fun ratings(trackIds: List<Long>): ApiResult<List<TrackRating>>

    /** Rate (0 clears; 0–100 in steps of 10). Queued offline when the server can't be reached. */
    suspend fun rate(trackId: Long, value: Int): ApiResult<TrackRating?>

    suspend fun clear(): ApiResult<Long>
}

class RustRatingsRepository(
    private val source: ConnectionSource,
    /** Queue a rating of a downloaded track offline; false if it isn't downloaded. */
    private val offlineRate: suspend (Long, Int) -> Boolean = { _, _ -> false },
) : RatingsRepository {
    private suspend fun <T> withRoutine(block: suspend (Connection) -> T): ApiResult<T> {
        val c = source.connection.value ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(c) }
    }

    override suspend fun style() = withRoutine { it.routine.ratingStyle() }

    override suspend fun setStyle(style: RatingStyle) = withRoutine { it.routine.setRatingStyle(style) }

    override suspend fun ratings(trackIds: List<Long>) = withRoutine { it.routine.ratings(trackIds) }

    override suspend fun rate(trackId: Long, value: Int): ApiResult<TrackRating?> {
        val v = value.coerceIn(0, 100)
        val r = withRoutine { it.routine.rate(trackId, v.toUInt()) }
        if (r is ApiResult.Err && (r.error is AppError.Offline || r.error is AppError.SignedOut) && offlineRate(trackId, v)) {
            val mine = (v + 5) / 10 * 10
            return ApiResult.Ok(TrackRating(trackId, mine.takeIf { it > 0 }?.toUInt(), null, mine.takeIf { it > 0 }?.toUInt()))
        }
        return r
    }

    override suspend fun clear() = withRoutine { it.routine.clearRatings().toLong() }
}

/** Ratings for the tracks on screen: the most recent [maxEntries] kept, the style once. */
class RatingsCache(private val repo: RatingsRepository, private val maxEntries: Int = 300) {
    private val mutex = Mutex()
    private val results = LinkedHashMap<Long, TrackRating>(16, 0.75f, true)

    suspend fun get(trackId: Long): TrackRating? {
        if (trackId <= 0) return null
        mutex.withLock { results[trackId]?.let { return it } }
        val r = (repo.ratings(listOf(trackId)) as? ApiResult.Ok)?.value?.firstOrNull { it.trackId == trackId } ?: return null
        keep(r)
        return r
    }

    suspend fun rate(trackId: Long, value: Int): ApiResult<TrackRating?> {
        val r = repo.rate(trackId, value)
        if (r is ApiResult.Ok) r.value?.let { keep(it) }
        return r
    }

    private suspend fun keep(r: TrackRating) = mutex.withLock {
        results[r.trackId] = r
        while (results.size > maxEntries) results.remove(results.keys.first())
    }

    suspend fun clear() = mutex.withLock { results.clear() }
}

/** Alarms (docs/plans/wave2-design.md §1) on players the user may control. */
interface AlarmsRepository {
    suspend fun alarms(player: String): ApiResult<AlarmListing>

    suspend fun add(player: String, change: AlarmChange): ApiResult<AlarmListing>

    suspend fun change(player: String, id: String, change: AlarmChange): ApiResult<AlarmListing>

    suspend fun delete(player: String, id: String): ApiResult<AlarmListing>

    suspend fun setAll(player: String, enabled: Boolean): ApiResult<AlarmListing>

    /** The player whose alarms this app just changed (for views that show the next alarm). */
    val changes: Flow<String> get() = emptyFlow()
}

class RustAlarmsRepository(private val source: ConnectionSource) : AlarmsRepository {
    private val changed = MutableSharedFlow<String>(extraBufferCapacity = 8, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    override val changes: Flow<String> = changed

    private suspend fun <T> withRoutine(block: suspend (Connection) -> T): ApiResult<T> {
        val c = source.connection.value ?: return ApiResult.Err(AppError.SignedOut)
        return apiCall { block(c) }
    }

    /** A change that went through: say so. */
    private fun <T> ApiResult<T>.told(player: String): ApiResult<T> = also { if (it is ApiResult.Ok) changed.tryEmit(player) }

    override suspend fun alarms(player: String) = withRoutine { it.routine.alarms(player) }

    override suspend fun add(player: String, change: AlarmChange) = withRoutine { it.routine.addAlarm(player, change) }.told(player)

    override suspend fun change(player: String, id: String, change: AlarmChange) =
        withRoutine { it.routine.changeAlarm(player, id, change) }.told(player)

    override suspend fun delete(player: String, id: String) = withRoutine { it.routine.deleteAlarm(player, id) }.told(player)

    override suspend fun setAll(player: String, enabled: Boolean) =
        withRoutine { it.routine.setAllAlarms(player, enabled) }.told(player)
}

/**
 * When a player's alarms next ring (Wave 2 §4.3, Rooms): the soonest enabled alarm with a
 * day, as seconds from now, counted from [weekdayNow] (0 Sunday, as LMS numbers days) and
 * [secsToday] (since local midnight). Null when none is set, or the player's alarms are
 * all off. (The same rule as the Rust layer's `next_alarm_in`.)
 */
fun nextAlarmIn(listing: AlarmListing?, weekdayNow: Int, secsToday: Int): Int? {
    if (listing == null || !listing.enabled) return null
    val today = Math.floorMod(weekdayNow, 7)
    val now = secsToday.coerceIn(0, 86_399)
    var best: Int? = null
    for (a in listing.alarms) {
        if (!a.enabled || a.days.isEmpty()) continue
        val days = a.days.map { it.toInt() }.toSet()
        val time = a.timeSecs.toLong().coerceIn(0, 86_399).toInt()
        for (ahead in 0..7) {
            if ((today + ahead) % 7 !in days) continue
            val at = ahead * 86_400 + time
            if (at > now) {
                val wait = at - now
                best = if (best == null) wait else minOf(best, wait)
                break
            }
        }
    }
    return best
}
