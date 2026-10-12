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
import io.github.jamezuk.lyrionanywhere.core.SleepEnd
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TransferOutcome
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.data.NowPlaying
import io.github.jamezuk.lyrionanywhere.data.NowPlayingHub
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
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
import io.github.jamezuk.lyrionanywhere.data.LyricsCache
import io.github.jamezuk.lyrionanywhere.data.RatingsCache
import io.github.jamezuk.lyrionanywhere.data.RatingsRepository
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
    /** The selected player's queue (the first page), each entry with a key for the UI. */
    val queue: List<QueueEntry> = emptyList(),
    val loading: Boolean = true,
    val error: AppError? = null,
    /** The event stream is connected (updates are instant rather than polled). */
    val live: Boolean = false,
    /** This phone as a player ("Play on this phone"). */
    val local: LocalPlayerState = LocalPlayerState(),
    /** The current track's waveform for the seek bar (null: the plain seek bar). */
    val waveform: Waveform? = null,
    /** Sync groups among the players this account may control. */
    val syncGroups: List<SyncGroup> = emptyList(),
    /** What each other player is playing, for the "Play on…" sheet (player id → track). */
    val playingOn: Map<String, Track> = emptyMap(),
    /** The default-player setting (D20). */
    val defaultPlayer: DefaultPlayer = DefaultPlayer.THIS_PHONE,
    val defaultPlayerId: String? = null,
    /** Waiting for this phone's player to appear in LMS before controlling it. */
    val waitingForPhone: Boolean = false,
    /** Moving playback to another player (docs/PROTOCOL.md › Transfer). */
    val transfer: TransferState = TransferState.Idle,
    /** The sleep timer's fade-out on this phone's player (seconds; Settings). */
    val sleepFade: Int = io.github.jamezuk.lyrionanywhere.data.SLEEP_FADE_DEFAULT,
    /** A timer set to an end of the queue, kept in step with it (see [SleepTimer]). */
    val sleepArm: SleepArm? = null,
    /** The current track's lyrics (null: none, or not known yet). */
    val lyrics: TrackLyrics? = null,
    /** The track [lyrics] belong to. */
    val lyricsTrackId: Long? = null,
    /** Show the current lyric line on Now playing (Settings). */
    val lyricsPeek: Boolean = true,
    /** The current track's rating (null: not known yet, or not rateable). */
    val rating: TrackRating? = null,
    /** Stars or a heart (the user's choice, the same on every device). */
    val ratingStyle: RatingStyle = RatingStyle.STARS,
    /** The last rating couldn't be saved. */
    val ratingFailed: Boolean = false,
) {
    val selected: Player? get() = players.firstOrNull { it.id == selectedId }
    val playing: Boolean get() = status?.mode == PlaybackMode.PLAYING

    /** The selected player is this phone. */
    val localSelected: Boolean get() = local.playerId != null && local.playerId == selectedId

    /** Controlling a player other than this phone (shown as "Playing on …", D20). */
    val controllingOther: Boolean get() = selectedId != null && !localSelected

    /** The other players this account may control (for "Play on…"). */
    val otherPlayers: List<Player> get() = players.filter { it.id != local.playerId }
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

    /** A queue entry was removed; [canUndo] offers Undo. */
    data class Removed(val title: String, val canUndo: Boolean, val token: Long = 0) : PlayerMessage

    /** A queue edit failed (the queue shown is put back); shown with the error's detail. */
    data class QueueFailed(val error: AppError, val undo: Boolean = false) : PlayerMessage

    /** Playback moved to [targetName], which is now controlled. */
    data class Moved(val targetName: String) : PlayerMessage

    /** The sleep timer was set: the player stops in [seconds]. */
    data class SleepSet(val seconds: Int) : PlayerMessage

    /** The sleep timer was turned off. */
    data object SleepOff : PlayerMessage

    /** "End of …" can't be worked out (a stream with no set length, or over a day away). */
    data object SleepUnknown : PlayerMessage
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
    /** Lyrics for Now playing (none without). */
    private val lyrics: LyricsCache? = null,
    /** Ratings (none without). */
    private val ratings: RatingsRepository? = null,
    /** Where the home-screen widget and tile learn what plays (none in tests). */
    private val nowPlaying: NowPlayingHub? = null,
    private val clock: () -> Long,
) : ViewModel() {
    /** The last removal, for Undo (only for the player it was removed from). */
    private data class Undo(val player: String, val track: Track, val index: Int, val token: Long)

    private var undo: Undo? = null
    /** Queue edits run one at a time, in order. */
    private val edits = Mutex()
    /** Edits sent but not finished: a queue read meanwhile would undo what is shown. */
    private var pendingEdits = 0
    private var nextQueueKey = 0L
    private var undoToken = 0L

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
    /** Whether the start-up choice (D20) has been made for this sign-in. */
    private var started = false
    /** Bumped by every local change, so a slow status answer can't undo it. */
    private var changeGeneration = 0

    /** Start live updates (the UI became visible). */
    @OptIn(ExperimentalCoroutinesApi::class)
    fun start() {
        if (live?.isActive == true) return
        live = viewModelScope.launch {
            launch {
                settings.settings.map { it.defaultPlayer to it.defaultPlayerId }.distinctUntilChanged().collect { (d, id) ->
                    _state.update { it.copy(defaultPlayer = d, defaultPlayerId = id) }
                }
            }
            launch {
                settings.settings.map { it.sleepFadeSecs }.distinctUntilChanged().collect { f ->
                    _state.update { it.copy(sleepFade = SleepTimer.fade(f)) }
                }
            }
            launch {
                settings.settings.map { it.lyricsPeek }.distinctUntilChanged().collect { on ->
                    _state.update { it.copy(lyricsPeek = on) }
                }
            }
            launch {
                // This phone is the default player: make sure it is on.
                val s = settings.settings.first()
                if (!started && PlayerChoice.wantsPhone(s) && !_state.value.local.enabled && !localPlayer.state.value.enabled) {
                    localPlayer.setEnabled(true)
                }
                loadPlayers()
            }
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
                    // Still waiting for the phone to show up in LMS: look again.
                    if (_state.value.selectedId == null) loadPlayers() else refreshStatus()
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
            if (lyrics != null) {
                launch {
                    // The current track's lyrics, as for the waveform.
                    state.map { it.status?.current?.id }.distinctUntilChanged().collectLatest { id ->
                        _state.update { s -> if (s.lyricsTrackId == id) s else s.copy(lyrics = null, lyricsTrackId = null) }
                        if (id == null || id <= 0) return@collectLatest
                        val l = lyrics.load(id)
                        _state.update { s -> if (s.status?.current?.id == id) s.copy(lyrics = l, lyricsTrackId = id) else s }
                    }
                }
            }
            if (ratings != null) {
                launch {
                    (ratings.style() as? ApiResult.Ok)?.value?.let { st -> _state.update { it.copy(ratingStyle = st) } }
                }
                launch {
                    // The current track's rating, as for the lyrics.
                    state.map { it.status?.current?.id }.distinctUntilChanged().collectLatest { id ->
                        _state.update { s -> if (s.rating?.trackId == id) s else s.copy(rating = null, ratingFailed = false) }
                        if (id == null || id <= 0) return@collectLatest
                        val r = ratingsCache?.get(id)
                        _state.update { s -> if (s.status?.current?.id == id) s.copy(rating = r) else s }
                    }
                }
            }
            if (nowPlaying != null) {
                launch {
                    // What the controlled player plays, for the home-screen widget.
                    state.map { s ->
                        s.selectedId?.let { id -> NowPlaying.from(id, s.selected?.name.orEmpty(), s.status, s.localSelected) }
                    }.distinctUntilChanged().collect { n -> if (n != null) nowPlaying.publishSelected(n) }
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
                val s = settings.settings.first()
                val local = localPlayer.state.value
                val current = _state.value.selectedId?.takeIf { id -> list.any { it.id == id } }
                var waiting = false
                val phoneAvailable = local.enabled && local.error == null
                val phoneListed = local.playerId?.takeIf { id -> list.any { it.id == id } }
                val chosen = current ?: if (selectLocalWhenReady && phoneAvailable) {
                    // "Back to this phone" is pending: the phone, once LMS lists it.
                    phoneListed ?: run {
                        waiting = true
                        null
                    }
                } else run {
                    when (val start = PlayerChoice.start(list, local.playerId, phoneAvailable, s)) {
                        is PlayerChoice.Start.Control -> start.playerId
                        PlayerChoice.Start.WaitForPhone -> {
                            waiting = true
                            null
                        }
                        PlayerChoice.Start.Nothing -> null
                    }
                }
                if (chosen != null) started = true
                _state.update {
                    it.copy(players = list, selectedId = chosen, loading = false, error = null, waitingForPhone = waiting)
                }
                if (chosen != null && chosen != s.selectedPlayerId) settings.setSelectedPlayer(chosen)
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
            is ApiResult.Ok -> {
                _state.update {
                    if (it.selectedId != id || generation != changeGeneration) it
                    else it.copy(status = r.value, statusAt = at, error = null)
                }
                sleepCheck(id, r.value)
            }
            is ApiResult.Err -> _state.update { if (it.selectedId == id) it.copy(error = r.error) else it }
        }
    }

    fun loadQueue() {
        val id = _state.value.selectedId ?: return
        viewModelScope.launch { readQueue(id) }
    }

    private suspend fun readQueue(id: String) {
        when (val r = players.queue(id, 0, QUEUE_PAGE)) {
            is ApiResult.Ok -> _state.update {
                // An edit still on its way would be undone on screen by this older list.
                if (it.selectedId != id || pendingEdits > 0) it else it.copy(queue = r.value.map { t -> QueueEntry(nextQueueKey++, t) })
            }
            is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
        }
    }

    // --- queue editing (as the web app: optimistic, put back on failure) ---------------------

    /**
     * Show [optimistic] at once and run [action] on the selected player (after any earlier
     * edit). On failure the list shown before is put back and the error reported.
     */
    private fun edit(
        optimistic: (List<QueueEntry>) -> List<QueueEntry>,
        undoing: Boolean = false,
        onDone: (String) -> Unit = {},
        action: suspend (String) -> ApiResult<Unit>,
    ) {
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        val before = _state.value.queue
        _state.update { it.copy(queue = optimistic(it.queue)) }
        pendingEdits++
        viewModelScope.launch {
            val r = try {
                edits.withLock { action(id) }
            } finally {
                pendingEdits--
            }
            when (r) {
                is ApiResult.Ok -> onDone(id)
                is ApiResult.Err -> {
                    _state.update { if (it.selectedId == id) it.copy(queue = before) else it }
                    _messages.trySend(PlayerMessage.QueueFailed(r.error, undo = undoing))
                }
            }
            if (pendingEdits == 0 && _state.value.selectedId == id) {
                readQueue(id)
                refreshStatus()
            }
        }
    }

    /** Take the entry at [index] out (swipe), offering Undo for library tracks. */
    fun removeFromQueue(index: Int) {
        val entry = _state.value.queue.getOrNull(index) ?: return
        removeTrack(index, entry.track)
    }

    /**
     * Take the "Up next" track out of the queue (swiped away on Now playing), offering Undo.
     * It disappears at once; the next status says what follows now.
     */
    fun removeUpNext() {
        val status = _state.value.status ?: return
        val next = status.next ?: return
        val index = next.queueIndex?.toInt() ?: ((status.queueIndex?.toInt() ?: return) + 1)
        _state.update { s -> s.copy(status = s.status?.copy(next = null)) }
        removeTrack(index, next)
    }

    private fun removeTrack(index: Int, track: Track) {
        undo = null
        edit(optimistic = { QueueEdit.removeAt(it, index) }, onDone = { id ->
            val canUndo = QueueEdit.canRestore(track)
            val token = ++undoToken
            undo = if (canUndo) Undo(id, track, index, token) else null
            _messages.trySend(PlayerMessage.Removed(track.title, canUndo, token))
        }) { id -> players.removeFromQueue(id, index) }
    }

    /**
     * Put the last removed track back where it was (Undo). With [token], only if that is
     * still the last removal (an older snackbar's Undo does nothing).
     */
    fun undoRemove(token: Long? = null) {
        val u = undo ?: return
        if (token != null && token != u.token) return
        undo = null
        if (u.player != _state.value.selectedId) return
        val trackId = u.track.id ?: return
        edit(optimistic = { QueueEdit.insertAt(it, u.index, QueueEntry(nextQueueKey++, u.track)) }, undoing = true) { id ->
            players.restoreToQueue(id, trackId, u.index)
        }
    }

    /** Move the entry at [from] so it ends up at [to] (drag to reorder). */
    fun moveInQueue(from: Int, to: Int) {
        val size = _state.value.queue.size
        if (from == to || from !in 0 until size || to !in 0 until size) return
        undo = null
        edit(optimistic = { QueueEdit.reorder(it, from, to) }) { id -> players.moveInQueue(id, from, to) }
    }

    /** Empty the queue (after the UI has asked to confirm). */
    fun clearQueue() {
        undo = null
        edit(optimistic = { emptyList() }) { id -> players.clearQueue(id) }
    }

    // --- sync groups -------------------------------------------------------------------------

    /** Put [otherId] in the selected player's group ([on]), or take it out of its group. */
    fun setSynced(otherId: String, on: Boolean) {
        val current = _state.value.selectedId
        if (current == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        val before = _state.value.syncGroups
        _state.update { it.copy(syncGroups = SyncChoice.toggled(it.syncGroups, current, otherId, on)) }
        viewModelScope.launch {
            when (val r = players.setSynced(current, otherId, on)) {
                is ApiResult.Ok -> {}
                is ApiResult.Err -> {
                    _state.update { it.copy(syncGroups = before) }
                    _messages.trySend(PlayerMessage.Failed(r.error))
                }
            }
            (players.syncGroups() as? ApiResult.Ok)?.value?.let { g -> _state.update { it.copy(syncGroups = g) } }
            refreshStatus()
        }
    }

    // --- transfer (docs/PROTOCOL.md › Transfer) ------------------------------------------------

    /** A player's name as LMS lists it (the phone's own name for this phone). */
    private fun nameOf(id: String): String =
        _state.value.players.firstOrNull { it.id == id }?.name?.ifEmpty { null }
            ?: _state.value.local.status?.name?.takeIf { id == _state.value.local.playerId && it.isNotEmpty() }
            ?: id

    /** Move what the controlled player plays to [targetId] ("Move playback here"), then control it. */
    fun moveTo(targetId: String) {
        val source = _state.value.selectedId
        if (source == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        if (_state.value.transfer is TransferState.Running) return
        val name = nameOf(targetId)
        _state.update { it.copy(transfer = TransferState.Running(targetId, name, TransferStage.READING)) }
        viewModelScope.launch {
            val r = players.transfer(source, targetId) { stage ->
                _state.update { s ->
                    val running = s.transfer as? TransferState.Running
                    if (running?.targetId == targetId) s.copy(transfer = running.copy(stage = stage)) else s
                }
            }
            when (r) {
                is ApiResult.Ok -> when (val outcome = r.value) {
                    is TransferOutcome.Moved -> {
                        _state.update { it.copy(transfer = TransferState.Idle) }
                        if (targetId == _state.value.local.playerId) backToPhone() else select(targetId)
                        _messages.trySend(PlayerMessage.Moved(name))
                    }
                    is TransferOutcome.Refused ->
                        _state.update { it.copy(transfer = TransferState.Refused(name, outcome.reason)) }
                }
                is ApiResult.Err -> _state.update { it.copy(transfer = TransferState.Failed(name, r.error)) }
            }
            refreshStatus()
        }
    }

    /** "Bring to this phone": move what the controlled player plays to this phone's player. */
    fun bringToPhone() {
        val s = _state.value
        val phone = s.local.playerId?.takeIf { id -> s.players.any { it.id == id } }
        if (phone == null) {
            // The phone isn't an LMS player (yet): start it, and say so.
            if (!s.local.enabled) setPlayOnPhone(true)
            _state.update { it.copy(transfer = TransferState.PhoneNotReady) }
            return
        }
        moveTo(phone)
    }

    /** Hide a transfer's refusal or error. */
    fun dismissTransfer() {
        _state.update { if (it.transfer is TransferState.Running) it else it.copy(transfer = TransferState.Idle) }
    }

    /** Control this phone again ("Back to this phone"), starting its player if need be. */
    fun backToPhone() {
        val localId = _state.value.local.playerId
        if (localId != null && _state.value.players.any { it.id == localId }) {
            select(localId)
            return
        }
        changeGeneration++
        selectLocalWhenReady = true
        _state.update { it.copy(selectedId = null, status = null, queue = emptyList(), waitingForPhone = true) }
        setPlayOnPhone(true)
        viewModelScope.launch { loadPlayers() }
    }

    /** Players, sync groups and what each is playing, for the "Play on…" sheet. */
    fun loadOutputs() {
        viewModelScope.launch {
            (players.list() as? ApiResult.Ok)?.value?.let { list -> _state.update { it.copy(players = list) } }
            val groups = (players.syncGroups() as? ApiResult.Ok)?.value.orEmpty()
            _state.update { it.copy(syncGroups = groups) }
            val playing = mutableMapOf<String, Track>()
            for (p in _state.value.otherPlayers.take(MAX_OUTPUT_STATUS)) {
                val t = (players.status(p.id) as? ApiResult.Ok)?.value?.current ?: continue
                playing[p.id] = t
            }
            _state.update { it.copy(playingOn = playing) }
        }
    }

    /** The default player when the app starts (per device, D20). */
    fun setDefaultPlayer(choice: DefaultPlayer, playerId: String? = null) {
        viewModelScope.launch { settings.setDefaultPlayer(choice, playerId) }
    }

    fun select(playerId: String) {
        if (playerId == _state.value.selectedId) return
        started = true
        selectLocalWhenReady = false
        changeGeneration++
        _state.update { it.copy(selectedId = playerId, status = null, queue = emptyList(), waitingForPhone = false) }
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

    /**
     * Seek, starting playback there if the player was paused or stopped (as tapping the
     * position bar suggests). LMS keeps a paused player paused after a seek, and ignores a
     * seek on a stopped one, hence the order.
     */
    fun seek(seconds: Double) {
        val mode = _state.value.status?.mode
        command(
            optimistic = { it.copy(elapsedSecs = seconds.coerceAtLeast(0.0), mode = PlaybackMode.PLAYING) },
        ) { id ->
            when (mode) {
                PlaybackMode.PLAYING -> players.seek(id, seconds)
                PlaybackMode.PAUSED -> players.seek(id, seconds).then { players.setPaused(id, false) }
                else -> players.play(id).then { players.seek(id, seconds) }
            }
        }
    }

    /** Run [next] only if this succeeded. */
    private suspend fun ApiResult<Unit>.then(next: suspend () -> ApiResult<Unit>): ApiResult<Unit> =
        if (this is ApiResult.Err) this else next()

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

    /** Play a selection shuffled: shuffle by songs, then replace the queue and play. */
    fun playShuffled(selection: Selection) {
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        viewModelScope.launch {
            val r = players.setShuffle(id, ShuffleMode.SONGS).let {
                if (it is ApiResult.Err) it else players.enqueue(id, selection, Placement.PLAY_NOW)
            }
            when (r) {
                is ApiResult.Ok -> _messages.trySend(PlayerMessage.Queued(Placement.PLAY_NOW))
                is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
            }
            refreshStatus()
        }
    }

    /** Play a favourite (station, track, app item) on the selected player. */
    fun playFavourite(itemId: String, placement: Placement) {
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        viewModelScope.launch {
            when (val r = players.playFavourite(id, itemId, placement)) {
                is ApiResult.Ok -> _messages.trySend(PlayerMessage.Queued(placement))
                is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
            }
            refreshStatus()
        }
    }

    /** The player commands go to (for menus): the controlled player, or none yet. */
    fun target(): String? = _state.value.selectedId

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

    // --- sleep timer (Wave 1 §1) ---------------------------------------------------------------

    /** The queue after the current track (durations and albums), read once per queue state. */
    private var upcoming: Triple<String, Long, List<Track>>? = null
    private var sleepBusy = false

    private suspend fun upcoming(id: String, status: PlayerStatus?): ApiResult<List<Track>> {
        val cur = status?.queueIndex?.toInt() ?: 0
        val key = "$id|$cur|${status?.queueLength}"
        val now = clock()
        upcoming?.let { (k, at, list) -> if (k == key && now - at < UPCOMING_TTL_MS) return ApiResult.Ok(list) }
        return when (val r = players.queueTimes(id, cur + 1)) {
            is ApiResult.Ok -> {
                upcoming = Triple(key, now, r.value)
                r
            }
            is ApiResult.Err -> r
        }
    }

    /**
     * Set the sleep timer on the selected player, its sync group or every connected player.
     * LMS's own `sleep` does the stopping (also on hardware players, with the app closed);
     * "end of …" timers are kept in step afterwards ([sleepCheck]). This phone's player
     * also fades out over the user's fade-out length.
     */
    fun setSleep(choice: SleepChoice, scope: SleepScope = SleepScope.PLAYER) {
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        viewModelScope.launch {
            val status = _state.value.status
            val list = if (choice is SleepChoice.End && choice.end != SleepEnd.TRACK) {
                when (val r = upcoming(id, status)) {
                    is ApiResult.Ok -> r.value
                    is ApiResult.Err -> {
                        _messages.trySend(PlayerMessage.Failed(r.error))
                        return@launch
                    }
                }
            } else {
                emptyList()
            }
            val secs = SleepTimer.seconds(choice, status, list)
            if (secs == null || secs <= 0) {
                _messages.trySend(PlayerMessage.SleepUnknown)
                return@launch
            }
            val group = if (scope == SleepScope.GROUP) {
                val groups = (players.syncGroups() as? ApiResult.Ok)?.value ?: _state.value.syncGroups
                groups.firstOrNull { id in it.playerIds }?.playerIds.orEmpty()
            } else {
                emptyList()
            }
            val all = if (scope == SleepScope.ALL) _state.value.players.filter { it.connected }.map { it.id } else emptyList()
            val targets = SleepTimer.targets(id, group, all)
            var failed: AppError? = null
            for (p in targets) (players.setSleep(p, secs) as? ApiResult.Err)?.let { failed = it.error }
            val now = clock()
            _state.update {
                it.copy(sleepArm = if (choice is SleepChoice.End) SleepArm(choice.end, targets, now + secs * 1000L, now) else null)
            }
            val local = _state.value.local.playerId
            if (local != null && local in targets) localPlayer.setSleepFade(secs.toDouble(), _state.value.sleepFade)
            _messages.trySend(failed?.let { PlayerMessage.Failed(it) } ?: PlayerMessage.SleepSet(secs))
            refreshStatus()
        }
    }

    /** Turn the timer off (one tap): on the players it was set on, and the selected one. */
    fun cancelSleep() {
        val selected = _state.value.selectedId
        val targets = (_state.value.sleepArm?.players.orEmpty() + listOfNotNull(selected)).distinct()
        if (targets.isEmpty()) return
        _state.update { it.copy(sleepArm = null) }
        val local = _state.value.local.playerId
        if (local != null && local in targets) localPlayer.setSleepFade(null, _state.value.sleepFade)
        viewModelScope.launch {
            var failed: AppError? = null
            for (p in targets) (players.setSleep(p, 0) as? ApiResult.Err)?.let { failed = it.error }
            _messages.trySend(failed?.let { PlayerMessage.Failed(it) } ?: PlayerMessage.SleepOff)
            refreshStatus()
        }
    }

    private val ratingsCache = ratings?.let { RatingsCache(it) }

    /** Rate the current track (0 clears): shown at once, put back if it can't be saved. */
    fun rate(value: Int) {
        val id = _state.value.status?.current?.id?.takeIf { it > 0 } ?: return
        val cache = ratingsCache ?: return
        val before = _state.value.rating
        val v = ((value.coerceIn(0, 100) + 5) / 10) * 10
        val mine = v.takeIf { it > 0 }?.toUInt()
        _state.update { it.copy(rating = TrackRating(id, mine, before?.shared, mine ?: before?.shared), ratingFailed = false) }
        viewModelScope.launch {
            when (val r = cache.rate(id, v)) {
                is ApiResult.Ok -> _state.update { s -> if (s.status?.current?.id == id) s.copy(rating = r.value ?: s.rating) else s }
                is ApiResult.Err -> _state.update { s -> if (s.status?.current?.id == id) s.copy(rating = before, ratingFailed = true) else s }
            }
        }
    }

    /** Stars or a heart (kept on the server for this user). */
    fun setRatingStyle(style: RatingStyle) {
        _state.update { it.copy(ratingStyle = style) }
        viewModelScope.launch { ratings?.setStyle(style) }
    }

    /** Show the current lyric line on Now playing (Settings). */
    fun setLyricsPeek(on: Boolean) {
        viewModelScope.launch { settings.setLyricsPeek(on) }
    }

    /** The fade-out length (Settings; checked before it is kept). */
    fun setSleepFade(seconds: Int) {
        viewModelScope.launch { settings.setSleepFade(SleepTimer.fade(seconds)) }
    }

    /**
     * After each status: this phone's fade follows its timer (set from any controller), and
     * a timer set to an end of the queue is set again when that end moves (a seek, skip,
     * pause or queue edit), or forgotten once LMS no longer reports it.
     */
    private suspend fun sleepCheck(id: String, status: PlayerStatus) {
        val local = _state.value.local.playerId
        if (id == local) localPlayer.setSleepFade(status.sleepSecs, _state.value.sleepFade)
        val arm = _state.value.sleepArm ?: return
        if (arm.players.firstOrNull() != id || sleepBusy) return
        val now = clock()
        if (SleepTimer.over(arm, status.sleepSecs, now)) {
            _state.update { if (it.sleepArm == arm) it.copy(sleepArm = null) else it }
            return
        }
        sleepBusy = true
        try {
            val list = if (arm.end == SleepEnd.TRACK) {
                emptyList()
            } else {
                (upcoming(id, status) as? ApiResult.Ok)?.value ?: return
            }
            if (_state.value.sleepArm != arm) return
            val secs = SleepTimer.rearm(arm, SleepTimer.remaining(arm.end, status, list), now) ?: return
            for (p in arm.players) players.setSleep(p, secs)
            _state.update {
                if (it.sleepArm == arm) it.copy(sleepArm = arm.copy(endsAtMs = now + secs * 1000L, setAtMs = now)) else it
            }
            if (local != null && local in arm.players) localPlayer.setSleepFade(secs.toDouble(), _state.value.sleepFade)
        } finally {
            sleepBusy = false
        }
    }

    companion object {
        /** The queue read for an armed timer is used again for this long at most. */
        const val UPCOMING_TTL_MS = 30_000L

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
        /** Players whose status the "Play on…" sheet reads. */
        const val MAX_OUTPUT_STATUS = 20
    }
}
