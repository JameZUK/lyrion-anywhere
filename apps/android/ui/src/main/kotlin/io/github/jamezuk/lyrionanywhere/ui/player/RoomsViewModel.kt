package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.ServerEvent
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.data.AlarmsRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.ROOMS_MAX
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import io.github.jamezuk.lyrionanywhere.data.nextAlarmIn
import java.time.LocalDateTime
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** When a room's alarm next rings: a day (0 Sunday, as LMS numbers them) and a time. */
data class RoomAlarm(val day: Int, val timeSecs: Int)

/** One player in the Rooms view (Wave 2 §4.3). */
data class Room(
    val player: Player,
    val status: PlayerStatus? = null,
    /** Why its status couldn't be read. */
    val error: String? = null,
    val alarm: RoomAlarm? = null,
) {
    val id: String get() = player.id
    val name: String get() = status?.name?.takeIf { it.isNotBlank() } ?: player.name
    val playing: Boolean get() = status?.mode == PlaybackMode.PLAYING
}

data class RoomsUiState(
    val rooms: List<Room> = emptyList(),
    val groups: List<SyncGroup> = emptyList(),
    val loading: Boolean = true,
    /** The players couldn't be listed (shown with Try again). */
    val error: AppError? = null,
    /** More players than the view shows ([ROOMS_MAX]). */
    val more: Boolean = false,
) {
    fun room(id: String): Room? = rooms.firstOrNull { it.id == id }

    /** The other rooms in [id]'s sync group (empty when it plays alone). */
    fun groupOf(id: String): List<Room> =
        groups.firstOrNull { id in it.playerIds }?.playerIds.orEmpty().filter { it != id }.mapNotNull { room(it) }
}

/**
 * Every room at a glance (Wave 2 §4.3): each permitted player's status, read every
 * [POLL_MS] (and at once when the event stream says a player changed), for at most
 * [ROOMS_MAX] players, only between [start] and [stop] (the screen being shown). Alarms are
 * read once on start, then again for a player when this app changes its alarms.
 */
class RoomsViewModel(
    private val players: PlayersRepository,
    private val events: EventsRepository? = null,
    private val alarms: AlarmsRepository? = null,
    /** Today's day (0 Sunday) and seconds since local midnight. */
    private val now: () -> Pair<Int, Int> = {
        val t = LocalDateTime.now()
        (t.dayOfWeek.value % 7) to t.toLocalTime().toSecondOfDay()
    },
) : ViewModel() {
    private val _state = MutableStateFlow(RoomsUiState())
    val state: StateFlow<RoomsUiState> = _state.asStateFlow()

    private val _messages = Channel<AppError>(Channel.BUFFERED)
    /** A control that failed (for a snackbar). */
    val messages: Flow<AppError> = _messages.receiveAsFlow()

    private var live: Job? = null
    private val changed = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)

    /** Bumped by every local change, so a slower read can't undo it on screen. */
    private var generation = 0

    val polling: Boolean get() = live?.isActive == true

    /** The screen is shown: read everything, then keep it fresh. */
    fun start() {
        if (live?.isActive == true) return
        live = viewModelScope.launch {
            launch {
                loadPlayers()
                readAlarms()
            }
            launch {
                while (true) {
                    delay(POLL_MS)
                    refresh()
                }
            }
            launch {
                changed.collectLatest {
                    delay(EVENT_COALESCE_MS)
                    refresh()
                }
            }
            events?.let { e ->
                launch {
                    e.updates().collect { u ->
                        val ev = (u as? StreamUpdate.Event)?.event ?: return@collect
                        when (ev) {
                            is ServerEvent.PlayerChanged -> if (_state.value.room(ev.playerId) != null) changed.tryEmit(Unit)
                            is ServerEvent.Resync -> loadPlayers()
                            // The music server answers again: catch up at once.
                            ServerEvent.LmsOk -> loadPlayers()
                            else -> {}
                        }
                    }
                }
            }
            alarms?.let { a ->
                launch { a.changes.collect { id -> if (_state.value.room(id) != null) readAlarm(id) } }
            }
        }
    }

    /** The screen is gone: nothing more is read. */
    fun stop() {
        live?.cancel()
        live = null
    }

    fun retry() {
        viewModelScope.launch { loadPlayers() }
    }

    private suspend fun loadPlayers() {
        when (val r = players.list()) {
            is ApiResult.Err -> _state.update { it.copy(loading = false, error = r.error) }
            is ApiResult.Ok -> {
                val list = r.value.take(ROOMS_MAX)
                _state.update { s ->
                    s.copy(
                        rooms = list.map { p -> s.room(p.id)?.copy(player = p) ?: Room(p) },
                        loading = false,
                        error = null,
                        more = r.value.size > ROOMS_MAX,
                    )
                }
                refresh()
            }
        }
    }

    /** Re-read every room's status, and the sync groups. */
    private suspend fun refresh() {
        val ids = _state.value.rooms.map { it.id }
        if (ids.isEmpty()) return
        val g = generation
        when (val r = players.statuses(ids)) {
            is ApiResult.Ok -> {
                val byId = r.value.associateBy { it.playerId.lowercase() }
                _state.update { s ->
                    if (g != generation) return@update s
                    s.copy(
                        rooms = s.rooms.map { room ->
                            val read = byId[room.id.lowercase()] ?: return@map room
                            room.copy(status = read.status ?: room.status.takeIf { read.error == null }, error = read.error)
                        },
                    )
                }
            }
            is ApiResult.Err -> {}
        }
        (players.syncGroups() as? ApiResult.Ok)?.value?.let { groups ->
            _state.update { if (g != generation) it else it.copy(groups = groups) }
        }
    }

    private suspend fun readAlarms() {
        if (alarms == null) return
        for (id in _state.value.rooms.map { it.id }) readAlarm(id)
    }

    private suspend fun readAlarm(id: String) {
        val a = alarms ?: return
        val listing = (a.alarms(id) as? ApiResult.Ok)?.value ?: return
        val (day, secs) = now()
        val alarm = nextAlarmIn(listing, day, secs)?.let { wait ->
            val at = secs.toLong() + wait
            RoomAlarm(((day + at / 86_400) % 7).toInt(), (at % 86_400).toInt())
        }
        _state.update { s -> s.copy(rooms = s.rooms.map { if (it.id == id) it.copy(alarm = alarm) else it }) }
    }

    /** Show [change] on [id]'s card at once, run [action], then read the server's view. */
    private fun control(id: String, change: (PlayerStatus) -> PlayerStatus, action: suspend () -> ApiResult<Unit>) {
        generation++
        _state.update { s -> s.copy(rooms = s.rooms.map { if (it.id == id) it.copy(status = it.status?.let(change)) else it }) }
        viewModelScope.launch {
            val r = action()
            if (r is ApiResult.Err) _messages.trySend(r.error)
            generation++
            refresh()
        }
    }

    fun togglePlay(id: String) {
        val playing = _state.value.room(id)?.playing ?: return
        control(id, { it.copy(mode = if (playing) PlaybackMode.PAUSED else PlaybackMode.PLAYING) }) {
            if (playing) players.setPaused(id, true) else players.play(id)
        }
    }

    fun setVolume(id: String, volume: Int) {
        val v = volume.coerceIn(0, 100)
        control(id, { it.copy(volume = v.toUInt()) }) { players.setVolume(id, v) }
    }

    fun setPower(id: String, on: Boolean) = control(id, { it.copy(powered = on) }) { players.setPower(id, on) }

    /**
     * Sync: [dragged] joins [onto]'s group (it plays what that group plays; a card dragged onto
     * another, its "Sync with…" item, or its TalkBack action).
     */
    fun sync(dragged: String, onto: String) {
        if (dragged == onto || _state.value.room(dragged) == null || _state.value.room(onto) == null) return
        val before = _state.value.groups
        generation++
        _state.update { it.copy(groups = SyncChoice.toggled(it.groups, onto, dragged, true)) }
        viewModelScope.launch {
            val r = players.setSynced(onto, dragged, true)
            if (r is ApiResult.Err) {
                _state.update { it.copy(groups = before) }
                _messages.trySend(r.error)
            }
            generation++
            refresh()
        }
    }

    /** Take [id] out of its group (it stops, keeping a copy of the queue). */
    fun unsync(id: String) {
        if (_state.value.room(id) == null) return
        val before = _state.value.groups
        generation++
        _state.update { s -> s.copy(groups = s.groups.map { SyncGroup(it.playerIds.filter { p -> p != id }) }.filter { it.playerIds.size > 1 }) }
        viewModelScope.launch {
            val r = players.setSynced(id, id, false)
            if (r is ApiResult.Err) {
                _state.update { it.copy(groups = before) }
                _messages.trySend(r.error)
            }
            generation++
            refresh()
        }
    }

    override fun onCleared() {
        stop()
    }

    companion object {
        /** Statuses are read this often while the screen shows. */
        const val POLL_MS = 5_000L
        const val EVENT_COALESCE_MS = 150L
    }
}
