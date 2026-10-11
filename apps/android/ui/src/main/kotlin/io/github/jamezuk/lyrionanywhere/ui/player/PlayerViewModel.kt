package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.EventsState
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ServerEvent
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import io.github.jamezuk.lyrionanywhere.data.Waveform
import io.github.jamezuk.lyrionanywhere.data.WaveformCache
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.ExperimentalCoroutinesApi
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
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class PlayerUiState(
    val players: List<Player> = emptyList(),
    val selectedId: String? = null,
    val status: PlayerStatus? = null,
    /** When [status] was read ([PlayerViewModel]'s clock, ms), for a ticking position. */
    val statusAt: Long = 0,
    val queue: List<Track> = emptyList(),
    val loading: Boolean = true,
    val error: AppError? = null,
    /** The event stream is connected (updates are instant rather than polled). */
    val live: Boolean = false,
    /** This phone as a player ("Play on this phone"). */
    val local: LocalPlayerState = LocalPlayerState(),
    /** The current track's waveform for the seek bar (null: the plain seek bar). */
    val waveform: Waveform? = null,
) {
    val selected: Player? get() = players.firstOrNull { it.id == selectedId }
    val playing: Boolean get() = status?.mode == PlaybackMode.PLAYING

    /** The selected player is this phone. */
    val localSelected: Boolean get() = local.playerId != null && local.playerId == selectedId
}

/** For hosts without a local player (and tests). */
object NoLocalPlayer : LocalPlayerRepository {
    override val state: StateFlow<LocalPlayerState> = MutableStateFlow(LocalPlayerState())

    override suspend fun setEnabled(on: Boolean): ApiResult<Unit> =
        ApiResult.Err(AppError.InvalidInput("this phone can't be a player here"))

    override fun setHold(hold: Boolean) {}

    override fun setDuck(duck: Boolean) {}

    override fun setOutputRoute(route: OutputRoute) {}
}

/** One-off messages for a snackbar. */
sealed interface PlayerMessage {
    data class Queued(val placement: Placement) : PlayerMessage

    data object NoPlayer : PlayerMessage

    data class Failed(val error: AppError) : PlayerMessage
}

/**
 * Remote control of the selected LMS player, shared by every screen. Live while the UI is
 * visible ([start]/[stop]): it listens to the event stream and refreshes on changes, with a
 * slow poll as a fallback.
 */
class PlayerViewModel(
    private val players: PlayersRepository,
    private val events: EventsRepository,
    private val settings: SettingsStore,
    private val localPlayer: LocalPlayerRepository = NoLocalPlayer,
    /** Waveforms for the seek bar (none without). */
    private val waveforms: WaveformCache? = null,
    private val clock: () -> Long,
) : ViewModel() {
    /** Select this phone's player as soon as LMS lists it (after "Play on this phone"). */
    private var selectLocalWhenReady = false
    private val _state = MutableStateFlow(PlayerUiState())
    val state: StateFlow<PlayerUiState> = _state.asStateFlow()

    private val _messages = Channel<PlayerMessage>(Channel.BUFFERED)
    val messages: Flow<PlayerMessage> = _messages.receiveAsFlow()

    private val refreshRequests = MutableSharedFlow<Unit>(
        extraBufferCapacity = 1,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    private var live: Job? = null
    /** Bumped by every local change, so a slow status answer can't undo it. */
    private var changeGeneration = 0

    /** Start live updates (the UI became visible). */
    @OptIn(ExperimentalCoroutinesApi::class)
    fun start() {
        if (live?.isActive == true) return
        live = viewModelScope.launch {
            launch { loadPlayers() }
            launch {
                // Coalesce bursts of events (LMS sends several per change).
                refreshRequests.collectLatest {
                    delay(EVENT_COALESCE_MS)
                    refreshStatus()
                }
            }
            launch {
                events.updates().collect { update ->
                    when (update) {
                        is StreamUpdate.State -> _state.update { it.copy(live = update.state == EventsState.Live) }
                        is StreamUpdate.Event -> onEvent(update.event)
                    }
                }
            }
            launch {
                while (true) {
                    delay(if (_state.value.live) SLOW_POLL_MS else FAST_POLL_MS)
                    refreshStatus()
                }
            }
            if (waveforms != null) {
                launch {
                    // The current track's waveform: fetched when the track changes (a
                    // change cancels the fetch for the previous one).
                    state.map { it.status?.current?.id }.distinctUntilChanged().collectLatest { id ->
                        _state.update { s -> s.copy(waveform = s.waveform?.takeIf { it.trackId == id }) }
                        if (id == null || id <= 0) return@collectLatest
                        val w = waveforms.load(id)
                        _state.update { s -> if (s.status?.current?.id == id) s.copy(waveform = w) else s }
                    }
                }
            }
            launch {
                localPlayer.state.collect { local ->
                    val newId = local.playerId != null && local.playerId != _state.value.local.playerId
                    _state.update { it.copy(local = local) }
                    // LMS lists the phone once it has said hello: re-read the players.
                    if (newId) loadPlayers()
                }
            }
        }
    }

    /** Turn "Play on this phone" on or off; on also selects the phone once it is ready. */
    fun setPlayOnPhone(on: Boolean) {
        viewModelScope.launch {
            when (val r = localPlayer.setEnabled(on)) {
                is ApiResult.Ok -> {
                    selectLocalWhenReady = on
                    if (on) _state.value.local.playerId?.let { selectIfListed(it) }
                }
                is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
            }
        }
    }

    private fun selectIfListed(id: String) {
        if (_state.value.players.any { it.id == id }) {
            selectLocalWhenReady = false
            select(id)
        }
    }

    /** Stop live updates (the UI is no longer visible). */
    fun stop() {
        live?.cancel()
        live = null
        _state.update { it.copy(live = false) }
    }

    private fun onEvent(event: ServerEvent) {
        when (event) {
            is ServerEvent.PlayerChanged ->
                if (event.playerId == _state.value.selectedId) refreshRequests.tryEmit(Unit)
            is ServerEvent.Resync -> viewModelScope.launch { loadPlayers() }
            is ServerEvent.LibraryChanged -> {}
        }
    }

    private suspend fun loadPlayers() {
        when (val r = players.list()) {
            is ApiResult.Err -> _state.update { it.copy(loading = false, error = r.error) }
            is ApiResult.Ok -> {
                val list = r.value
                val saved = settings.settings.first().selectedPlayerId
                val current = _state.value.selectedId ?: saved
                val chosen = list.firstOrNull { it.id == current }?.id
                    ?: list.firstOrNull { it.playing }?.id
                    ?: list.firstOrNull()?.id
                _state.update { it.copy(players = list, selectedId = chosen, loading = false, error = null) }
                if (chosen != saved) settings.setSelectedPlayer(chosen)
                val localId = _state.value.local.playerId
                if (selectLocalWhenReady && localId != null && localId != chosen) {
                    selectIfListed(localId)
                }
                refreshStatus()
            }
        }
    }

    /** Re-read the selected player's status (and queue). */
    fun refresh() {
        viewModelScope.launch { loadPlayers() }
    }

    private suspend fun refreshStatus() {
        val id = _state.value.selectedId ?: return
        val generation = changeGeneration
        val at = clock()
        when (val r = players.status(id)) {
            is ApiResult.Ok -> _state.update {
                if (it.selectedId != id || generation != changeGeneration) it
                else it.copy(status = r.value, statusAt = at, error = null)
            }
            is ApiResult.Err -> _state.update { if (it.selectedId == id) it.copy(error = r.error) else it }
        }
    }

    fun loadQueue() {
        val id = _state.value.selectedId ?: return
        viewModelScope.launch {
            when (val r = players.queue(id, 0, QUEUE_PAGE)) {
                is ApiResult.Ok -> _state.update { if (it.selectedId == id) it.copy(queue = r.value) else it }
                is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
            }
        }
    }

    fun select(playerId: String) {
        if (playerId == _state.value.selectedId) return
        changeGeneration++
        _state.update { it.copy(selectedId = playerId, status = null, queue = emptyList()) }
        viewModelScope.launch {
            settings.setSelectedPlayer(playerId)
            refreshStatus()
        }
    }

    /** Run a command on the selected player, with an optimistic change shown at once. */
    private fun command(
        optimistic: (PlayerStatus) -> PlayerStatus = { it },
        action: suspend (String) -> ApiResult<Unit>,
    ) {
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        changeGeneration++
        val now = clock()
        _state.update { s ->
            // Re-base the ticking position on "now" so the change doesn't make it jump.
            val rebased = s.status?.copy(elapsedSecs = elapsedAt(s, now))
            s.copy(status = rebased?.let(optimistic), statusAt = now)
        }
        viewModelScope.launch {
            val r = action(id)
            if (r is ApiResult.Err) _messages.trySend(PlayerMessage.Failed(r.error))
            // The server's view wins once the command is done.
            changeGeneration++
            refreshStatus()
        }
    }

    fun togglePlay() {
        val playing = _state.value.playing
        command(
            optimistic = { it.copy(mode = if (playing) PlaybackMode.PAUSED else PlaybackMode.PLAYING) },
        ) { id -> if (playing) players.setPaused(id, true) else players.play(id) }
    }

    fun next() = command { players.next(it) }

    fun previous() = command { players.previous(it) }

    fun seek(seconds: Double) = command(
        optimistic = { it.copy(elapsedSecs = seconds.coerceAtLeast(0.0)) },
    ) { players.seek(it, seconds) }

    fun setVolume(volume: Int) = command(
        optimistic = { it.copy(volume = volume.coerceIn(0, 100).toUInt()) },
    ) { players.setVolume(it, volume) }

    fun cycleShuffle() {
        val next = when (_state.value.status?.shuffle) {
            ShuffleMode.OFF, null -> ShuffleMode.SONGS
            ShuffleMode.SONGS -> ShuffleMode.ALBUMS
            ShuffleMode.ALBUMS -> ShuffleMode.OFF
        }
        command(optimistic = { it.copy(shuffle = next) }) { players.setShuffle(it, next) }
    }

    fun cycleRepeat() {
        val next = when (_state.value.status?.repeat) {
            RepeatMode.OFF, null -> RepeatMode.ALL
            RepeatMode.ALL -> RepeatMode.ONE
            RepeatMode.ONE -> RepeatMode.OFF
        }
        command(optimistic = { it.copy(repeat = next) }) { players.setRepeat(it, next) }
    }

    fun jumpTo(index: Int) = command { players.jumpTo(it, index) }

    fun setPower(on: Boolean) = command(optimistic = { it.copy(powered = on) }) { players.setPower(it, on) }

    /** Play an album, track or artist on the selected player. */
    fun enqueue(selection: Selection, placement: Placement, playIndex: Int? = null) {
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        viewModelScope.launch {
            when (val r = players.enqueue(id, selection, placement, playIndex)) {
                is ApiResult.Ok -> _messages.trySend(PlayerMessage.Queued(placement))
                is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
            }
            refreshStatus()
        }
    }

    companion object {
        /** The position in the current track at time [now] (ticking while playing). */
        fun elapsedAt(state: PlayerUiState, now: Long): Double {
            val status = state.status ?: return 0.0
            val base = status.elapsedSecs
            val running = if (status.mode == PlaybackMode.PLAYING) (now - state.statusAt).coerceAtLeast(0) / 1000.0 else 0.0
            val pos = base + running
            return status.durationSecs?.let { pos.coerceAtMost(it) } ?: pos
        }

        const val EVENT_COALESCE_MS = 150L
        /** Polling while the event stream is down. */
        const val FAST_POLL_MS = 5_000L
        /** A safety net while it is up. */
        const val SLOW_POLL_MS = 30_000L
        const val QUEUE_PAGE = 200
    }
}
