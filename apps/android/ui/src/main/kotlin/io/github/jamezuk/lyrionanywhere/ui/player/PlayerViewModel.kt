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
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.VisualFrame
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.core.Credit
import io.github.jamezuk.lyrionanywhere.data.NowPlayingHub
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalControl
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.data.offline.Reachability
import io.github.jamezuk.lyrionanywhere.data.offline.reachability
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.PlaylistNames
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import io.github.jamezuk.lyrionanywhere.data.Waveform
import io.github.jamezuk.lyrionanywhere.data.WaveformCache
import io.github.jamezuk.lyrionanywhere.data.LyricsCache
import io.github.jamezuk.lyrionanywhere.data.RatingsCache
import io.github.jamezuk.lyrionanywhere.data.RatingsRepository
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.withContext
import io.github.jamezuk.lyrionanywhere.core.RunnerUpdate
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
    /** What this device added with Play next (Wave 2 §3.4); null: the queue shows flat. */
    val upNext: UpNext? = null,
    /** Now playing's layout, visualiser and queue panel (this device's choices, Wave 2 §4). */
    val nowPlaying: NowPlayingPrefs = NowPlayingPrefs(),
    /** The current track's credits, for the Info layout (null: not read). */
    val credits: List<Credit>? = null,
    /** The track [credits] belong to. */
    val creditsTrackId: Long? = null,
    /** "Save as playlist" is offered: an admin, on a gateway that has it. */
    val canSaveQueue: Boolean = false,
    /** The "Save as playlist" dialog, while it is open. */
    val saveQueue: SaveQueueDialog? = null,
    /** The link to the server, as the screen shows it (which controls work: offline controls). */
    val connection: ConnectionUi = ConnectionUi(),
    /** Downloaded tracks (the queue's "Downloaded" marks, for any player). */
    val downloaded: Set<Long> = emptySet(),
    /** Tracks this phone's player holds whole (its "On this device" marks). */
    val onDevice: Set<Long> = emptySet(),
    /**
     * This is the downloads playing on this phone (Wave 3.5: shown with the main screens,
     * the engine its own): no LMS-only controls (rating, sleep timer, queue edits).
     */
    val downloads: Boolean = false,
) {
    /** Whether the server can be reached. */
    val reachability: Reachability get() = connection.reachability

    /** Which controls work now, and where they go (see [controlGate]). */
    val gate: ControlGate
        get() = controlGate(reachability, localSelected, local.running, local.offlineControls).let { g ->
            // Holding the output (the music server busy): the phone says whether it plays.
            if (g.route == ControlRoute.HOLD) g.copy(playing = local.status?.let { it.playback == io.github.jamezuk.lyrionanywhere.core.LocalPlayback.PLAYING && !it.held }) else g
        }

    /** A queue row's mark. */
    fun markOf(track: Track): QueueMark = queueMark(track.id, downloaded, onDevice, localSelected)

    /** The queue's sections (Now playing, Up next, Then), or null for a flat queue. */
    val sections: QueueSections? get() = QueueSections.of(queue.size, status?.queueIndex?.toInt(), upNext)

    /** Tracks after the current one (for "Clear upcoming"), or null if not known. */
    val upcoming: Int?
        get() {
            val s = status ?: return null
            val cur = s.queueIndex?.toInt() ?: return null
            return (s.queueLength.toInt() - cur - 1).coerceAtLeast(0)
        }

    val selected: Player? get() = players.firstOrNull { it.id == selectedId }
    val playing: Boolean get() = gate.playing ?: (status?.mode == PlaybackMode.PLAYING)

    /** The selected player is this phone. */
    val localSelected: Boolean get() = downloads || (local.playerId != null && local.playerId == selectedId)

    /** Controlling a player other than this phone (shown as "Playing on …", D20). */
    val controllingOther: Boolean get() = selectedId != null && !localSelected

    /** The other players this account may control (for "Play on…"). */
    val otherPlayers: List<Player> get() = players.filter { it.id != local.playerId }

    /** The queue has entries after those read so far (more are read as the list scrolls). */
    val queueMore: Boolean
        get() = queue.size < PlayerViewModel.QUEUE_MAX && (status?.queueLength?.toInt() ?: 0) > queue.size
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

    /** The station kept going stopped: the server refused its next batch ([reason]). */
    data class StationStopped(val reason: String) : PlayerMessage

    /** A removal found a different track at that place (the queue changed): nothing removed. */
    data object QueueChanged : PlayerMessage

    /** "Clear upcoming" found the queue changed part-way, and stopped (Wave 2 §3.4). */
    data object ClearStopped : PlayerMessage

    /** The queue was saved as playlist [id], called [name] (the message offers to open it). */
    data class SavedPlaylist(val id: Long, val name: String) : PlayerMessage
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
    /** Stations kept going (none without). */
    private val me: io.github.jamezuk.lyrionanywhere.data.MeRepository? = null,
    /** Credits for the Info layout (none without). */
    private val library: io.github.jamezuk.lyrionanywhere.data.LibraryRepository? = null,
    /** Downloaded track ids (the queue's marks; none without). */
    private val downloaded: Flow<Set<Long>> = flowOf(emptySet()),
    private val clock: () -> Long,
) : ViewModel(), VisualSource {
    /** The last removal, for Undo (only for the player it was removed from). */
    private data class Undo(val player: String, val track: Track, val index: Int, val token: Long, val upNext: Boolean = false)

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
                settings.settings.map { it.nowPlaying }.distinctUntilChanged().collect { p ->
                    _state.update { it.copy(nowPlaying = p) }
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
                // Whether "Save as playlist" is offered (asked again on the next start if
                // the answer didn't come).
                if (!_state.value.canSaveQueue) {
                    (players.canSaveQueue() as? ApiResult.Ok)?.value?.let { on -> _state.update { it.copy(canSaveQueue = on) } }
                }
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
                        is StreamUpdate.State -> {
                            // Reconnecting: the gateway says again if the music server is still busy.
                            if (update.state != EventsState.Live) lmsBusy = false
                            _state.update { it.copy(live = update.state == EventsState.Live) }
                        }
                        is StreamUpdate.Event -> onEvent(update.event)
                    }
                }
            }
            launch {
                while (true) {
                    delay(pollDelay())
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
                    val wasOffline = _state.value.local.offlineControls != null
                    _state.update { withOffline(it.copy(local = local)) }
                    // LMS lists the phone once it has said hello: re-read the players.
                    if (newId) loadPlayers()
                    // The phone's player is back with LMS: LMS's view again.
                    if (wasOffline && local.offlineControls == null) refreshRequests.tryEmit(Unit)
                }
            }
            launch {
                downloaded.distinctUntilChanged().collect { ids -> _state.update { it.copy(downloaded = ids) } }
            }
            launch {
                // The queue as last read, for this phone's player to work on while the
                // server is away (only while it is the player, and only LMS's own view).
                state.map { s ->
                    if (!s.localSelected || s.local.offlineControls != null) {
                        null
                    } else {
                        Triple(s.queue.map { it.track.id }, lmsStatus?.second?.queueIndex?.toInt(), lmsStatus?.second?.repeat ?: RepeatMode.OFF)
                    }
                }.distinctUntilChanged().collect { snap ->
                    if (snap != null) localPlayer.setQueueSnapshot(snap.first, snap.second, snap.third)
                }
            }
            launch {
                // Which queued tracks this phone holds whole ("On this device"), read again
                // as the queue, the track playing or the link changes.
                state.map { s ->
                    if (!s.localSelected) null else Triple(s.queue.mapNotNull { it.track.id }, s.status?.current?.id, s.reachability)
                }.distinctUntilChanged().collectLatest { key ->
                    val on = if (key == null) emptySet() else onDeviceIds(key.first, localPlayer.localAvailability(key.first))
                    _state.update { it.copy(onDevice = on) }
                }
            }
        }
    }

    /** LMS's last status for the selected player (by id): what [withOffline] builds on. */
    private var lmsStatus: Pair<String, PlayerStatus>? = null

    /**
     * While this phone plays with the server away, what it shows is what the phone does
     * (see [offlineStatus]); otherwise LMS's status as read.
     */
    private fun withOffline(s: PlayerUiState): PlayerUiState {
        val lms = lmsStatus?.takeIf { it.first == s.selectedId }?.second
        val controls = s.local.offlineControls?.takeIf { s.localSelected }
        if (controls == null) {
            // Back with LMS: its status again (until it is read afresh).
            if (!offlineShown) return s
            offlineShown = false
            return if (lms != null) s.copy(status = lms) else s
        }
        offlineShown = true
        return s.copy(status = offlineStatus(lms ?: s.status, s.queue, controls), statusAt = clock())
    }

    /** What is shown is the phone's own state (see [withOffline]). */
    private var offlineShown = false

    /** A pause made by holding the output while the music server was busy (told to LMS once it answers). */
    private var heldWhileBusy = false

    /** The link to the server, as the screen shows it: which controls work (offline controls). */
    fun setConnection(connection: ConnectionUi) {
        val before = _state.value.reachability
        _state.update { it.copy(connection = connection) }
        if (before != Reachability.ONLINE && connection.reachability == Reachability.ONLINE) {
            val s = _state.value
            val id = s.selectedId
            if (heldWhileBusy && id != null && s.localSelected && s.local.status?.held == true) {
                viewModelScope.launch { players.setPaused(id, true) }
            }
            heldWhileBusy = false
            // Back: catch up with what LMS did meanwhile.
            viewModelScope.launch { refreshStatus(readQueue = true) }
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
        keepGoing()
    }

    /** Stop everything, the background poll too (sign-out, tests). */
    fun close() {
        stop()
        background?.cancel()
        background = null
    }

    /** The slow poll that runs while the UI isn't visible (see [keepGoing]). */
    private var background: Job? = null

    /** Whether anything needs the player watched while the UI isn't visible. */
    private fun needsWatching() = _state.value.sleepArm != null || me?.activeStation?.value != null

    /**
     * While the UI isn't visible, a station still needs its next batch and an "end of …"
     * sleep timer still needs setting again when the end moves: poll slowly until neither
     * is wanted, or the UI is back.
     */
    private fun keepGoing() {
        if (background?.isActive == true || !needsWatching()) return
        background = viewModelScope.launch {
            while (live == null && needsWatching()) {
                delay(BACKGROUND_POLL_MS)
                if (live == null) refreshStatus(readQueue = true)
            }
        }
    }

    private fun onEvent(event: ServerEvent) {
        when (event) {
            is ServerEvent.PlayerChanged ->
                if (event.playerId == _state.value.selectedId) refreshRequests.tryEmit(Unit)
            is ServerEvent.Resync -> viewModelScope.launch { loadPlayers() }
            is ServerEvent.LibraryChanged -> {}
            is ServerEvent.LmsBusy -> lmsBusy = true
            // The music server answers again: catch up at once.
            ServerEvent.LmsOk -> {
                lmsBusy = false
                refusedBusy = false
                viewModelScope.launch { loadPlayers() }
            }
        }
    }

    /** The event stream says the music server is busy (it stopped answering; the gateway answers). */
    private var lmsBusy = false

    /** The last status read was refused as busy (until one is answered). */
    private var refusedBusy = false
    private var busyRetryMs = 0L

    /**
     * How long until the next poll. While the music server is busy it isn't asked over and
     * over: with the stream up its `ok` says when it is back; without, at most every
     * [BUSY_POLL_MS] (or the gateway's Retry-After, if longer).
     */
    internal fun pollDelay(): Long = when {
        _state.value.live -> SLOW_POLL_MS
        lmsBusy || refusedBusy -> maxOf(BUSY_POLL_MS, busyRetryMs)
        else -> FAST_POLL_MS
    }

    private suspend fun loadPlayers() {
        when (val r = players.list()) {
            is ApiResult.Err -> {
                val busy = r.error as? AppError.ServerBusy
                refusedBusy = busy != null
                if (busy != null) busyRetryMs = busy.retryAfterSeconds * 1000
                // Busy for a moment: what is shown stays (a calm note says why).
                _state.update { it.copy(loading = false, error = if (busy != null && it.status != null) it.error else r.error) }
            }
            is ApiResult.Ok -> {
                refusedBusy = false
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

    /** A status read is on its way: another one asked meanwhile waits for it (see [refreshStatus]). */
    private var statusInFlight = false

    /** Asked for while one was on its way (and whether with the queue), or null. */
    private var statusAgain: Boolean? = null

    /**
     * Re-read the selected player's status. Never two at once: one asked for while another is
     * on its way runs once after it (so a slow server isn't sent a pile of them), unless the
     * music server turned out busy (it is read again when it is back).
     */
    private suspend fun refreshStatus(readQueue: Boolean = false) {
        if (statusInFlight) {
            statusAgain = (statusAgain ?: false) || readQueue
            return
        }
        statusInFlight = true
        try {
            var next: Boolean? = readQueue
            while (next != null) {
                statusAgain = null
                readStatus(next)
                next = statusAgain.takeUnless { refusedBusy }
            }
        } finally {
            statusInFlight = false
            statusAgain = null
        }
    }

    private suspend fun readStatus(readQueue: Boolean) {
        val id = _state.value.selectedId ?: return
        val generation = changeGeneration
        val at = clock()
        when (val r = players.status(id)) {
            is ApiResult.Ok -> {
                lmsStatus = id to r.value
                _state.update {
                    if (it.selectedId != id || generation != changeGeneration) {
                        it
                    } else {
                        withOffline(
                            it.copy(
                                status = r.value,
                                statusAt = at,
                                error = null,
                                // The player moving on uses up Up next (not while an edit is on its way).
                                upNext = if (pendingEdits > 0) it.upNext else UpNextTracker.advanced(it.upNext, r.value.queueIndex?.toInt()),
                            ),
                        )
                    }
                }
                refusedBusy = false
                sleepCheck(id, r.value)
                followStation(id, r.value, readQueue)
            }
            is ApiResult.Err -> {
                val busy = r.error as? AppError.ServerBusy
                refusedBusy = busy != null
                if (busy != null) busyRetryMs = busy.retryAfterSeconds * 1000
                _state.update {
                    when {
                        it.selectedId != id -> it
                        // Busy for a moment: what is shown stays (a calm note says why).
                        busy != null && it.status != null -> it
                        // The server away: what is shown stays (one line says why).
                        it.reachability != Reachability.ONLINE && it.status != null -> it
                        else -> it.copy(error = r.error)
                    }
                }
            }
        }
    }

    fun loadQueue() {
        val id = _state.value.selectedId ?: return
        viewModelScope.launch { readQueue(id) }
    }

    /** How many queue entries the list wants (a page more each time it scrolls to the end). */
    private var queueWanted = QUEUE_PAGE
    private var queueLoadingMore = false

    /** Read the queue afresh: as many entries as the list has asked for, a page at a time. */
    private suspend fun readQueue(id: String) {
        val want = queueWanted.coerceIn(QUEUE_PAGE, QUEUE_MAX)
        val tracks = mutableListOf<Track>()
        while (tracks.size < want) {
            when (val r = players.queue(id, tracks.size, QUEUE_PAGE)) {
                is ApiResult.Ok -> {
                    tracks += r.value
                    if (r.value.size < QUEUE_PAGE) break
                }
                is ApiResult.Err -> {
                    // A read of its own accord: no message just because the music server is busy.
                    if (r.error !is AppError.ServerBusy) _messages.trySend(PlayerMessage.Failed(r.error))
                    return
                }
            }
            if (_state.value.selectedId != id) return
        }
        _state.update {
            // An edit still on its way would be undone on screen by this older list.
            if (it.selectedId != id || pendingEdits > 0) {
                it
            } else {
                it.copy(
                    queue = tracks.map { t -> QueueEntry(nextQueueKey++, t) },
                    upNext = UpNextTracker.verified(it.upNext, it.status?.queueIndex?.toInt(), tracks),
                )
            }
        }
    }

    /**
     * The queue list reached its end and there is more: read the next page. Skipped while an
     * edit is on its way (the list is read afresh after it) or a page is already coming.
     */
    fun loadMoreQueue() {
        val s = _state.value
        val id = s.selectedId ?: return
        if (queueLoadingMore || pendingEdits > 0 || !s.queueMore) return
        val before = s.queue
        queueLoadingMore = true
        queueWanted = (before.size + QUEUE_PAGE).coerceAtMost(QUEUE_MAX)
        viewModelScope.launch {
            try {
                when (val r = players.queue(id, before.size, QUEUE_PAGE)) {
                    is ApiResult.Ok -> _state.update {
                        // Only onto the list it follows: anything changed meanwhile is read afresh.
                        if (it.selectedId != id || pendingEdits > 0 || it.queue !== before) {
                            it
                        } else {
                            val queue = (before + r.value.map { t -> QueueEntry(nextQueueKey++, t) }).take(QUEUE_MAX)
                            it.copy(
                                queue = queue,
                                upNext = UpNextTracker.verified(it.upNext, it.status?.queueIndex?.toInt(), queue.map { e -> e.track }),
                            )
                        }
                    }
                    is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
                }
            } finally {
                queueLoadingMore = false
            }
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
        failed: (AppError) -> PlayerMessage = { PlayerMessage.QueueFailed(it, undo = undoing) },
        /** How the edit changes Up next and the current index (shown at once, with the list). */
        tracking: (PlayerUiState) -> Pair<UpNext?, Int?> = { it.upNext to it.status?.queueIndex?.toInt() },
        action: suspend (String) -> ApiResult<Unit>,
    ) {
        // Queue edits need the server (greyed out while it is away).
        if (!_state.value.gate.other) return
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        val before = _state.value.queue
        val upNextBefore = _state.value.upNext
        val indexBefore = _state.value.status?.queueIndex
        // A status read already on its way would show the queue as it was.
        changeGeneration++
        _state.update {
            val (upNext, index) = tracking(it)
            it.copy(
                queue = optimistic(it.queue),
                upNext = upNext,
                status = it.status?.copy(queueIndex = index?.takeIf { i -> i >= 0 }?.toUInt()),
            )
        }
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
                    _state.update {
                        if (it.selectedId == id) {
                            it.copy(queue = before, upNext = upNextBefore, status = it.status?.copy(queueIndex = indexBefore))
                        } else {
                            it
                        }
                    }
                    _messages.trySend(failed(r.error))
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

    /**
     * Remove [track] at [index], after checking that it is still the track there: the queue
     * may have moved on since it was shown (a track ended, another controller edited it). If
     * it has, nothing is removed and the queue is read afresh.
     */
    private fun removeTrack(index: Int, track: Track) {
        undo = null
        var changed = false
        val wasUpNext = _state.value.upNext?.range?.contains(index) == true
        edit(
            optimistic = { QueueEdit.removeAt(it, index) },
            tracking = { s ->
                val cur = s.status?.queueIndex?.toInt()
                UpNextTracker.removed(s.upNext, index) to if (cur != null && index < cur) cur - 1 else cur
            },
            onDone = { id ->
                val canUndo = QueueEdit.canRestore(track)
                val token = ++undoToken
                undo = if (canUndo) Undo(id, track, index, token, wasUpNext) else null
                _messages.trySend(PlayerMessage.Removed(track.title, canUndo, token))
            },
            failed = { e -> if (changed) PlayerMessage.QueueChanged else PlayerMessage.QueueFailed(e) },
        ) { id ->
            val trackId = track.id
            if (trackId != null && trackId != 0L) {
                // Checked and removed in one step on the gateway.
                when (val r = players.removeIfStill(id, index, trackId)) {
                    is ApiResult.Err -> r
                    is ApiResult.Ok -> if (r.value) {
                        ApiResult.Ok(Unit)
                    } else {
                        changed = true
                        ApiResult.Err(AppError.InvalidInput("the queue changed"))
                    }
                }
            } else {
                when (val at = players.queue(id, index, 1)) {
                    is ApiResult.Err -> at
                    is ApiResult.Ok -> if (QueueEdit.sameTrack(at.value.firstOrNull(), track)) {
                        players.removeFromQueue(id, index)
                    } else {
                        changed = true
                        ApiResult.Err(AppError.InvalidInput("the queue changed"))
                    }
                }
            }
        }
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
        edit(
            optimistic = { QueueEdit.insertAt(it, u.index, QueueEntry(nextQueueKey++, u.track)) },
            undoing = true,
            tracking = { s ->
                val cur = s.status?.queueIndex?.toInt()
                UpNextTracker.restored(s.upNext, u.index, trackId, u.upNext) to if (cur != null && u.index <= cur) cur + 1 else cur
            },
        ) { id ->
            players.restoreToQueue(id, trackId, u.index)
        }
    }

    /** Move the entry at [from] so it ends up at [to] (drag to reorder). */
    fun moveInQueue(from: Int, to: Int) {
        val size = _state.value.queue.size
        if (from == to || from !in 0 until size || to !in 0 until size) return
        undo = null
        val movedId = _state.value.queue[from].track.id
        edit(
            optimistic = { QueueEdit.reorder(it, from, to) },
            tracking = { s ->
                val cur = s.status?.queueIndex?.toInt()
                UpNextTracker.moved(s.upNext, from, to, cur, movedId) to cur?.let { QueueEdit.indexAfterMove(it, from, to) }
            },
        ) { id ->
            if (movedId != null && movedId != 0L) {
                // Only while it is still the track that was dragged (checked on the gateway).
                when (val r = players.moveIfStill(id, from, to, movedId)) {
                    is ApiResult.Err -> r
                    is ApiResult.Ok -> if (r.value) ApiResult.Ok(Unit) else ApiResult.Err(AppError.InvalidInput("the queue changed"))
                }
            } else {
                players.moveInQueue(id, from, to)
            }
        }
    }

    /**
     * Play the entry at [index] next (a right swipe on the queue, or its menu): it moves to
     * just after the current track and joins Up next. Nothing to do for the current track or
     * the one already next.
     */
    fun playNextInQueue(index: Int) {
        val cur = _state.value.status?.queueIndex?.toInt() ?: return
        val to = QueueEdit.playNextTarget(index, cur) ?: return
        moveInQueue(index, to)
    }

    /**
     * Open "Save as playlist", with a name to start from: the album when the queue shown is
     * one album, else "Queue" and [today] (as the UI writes dates).
     */
    fun openSaveQueue(today: String) {
        val s = _state.value
        if (!s.canSaveQueue || s.selectedId == null || s.queue.isEmpty() || !s.gate.other) return
        val suggestion = PlaylistNames.suggest(s.queue.map { it.track.album }, today)
        _state.update { it.copy(saveQueue = SaveQueueDialog(suggestion)) }
    }

    fun closeSaveQueue() {
        _state.update { it.copy(saveQueue = null) }
    }

    /** The name was taken and another is wanted: back to the name. */
    fun saveQueueChangeName() {
        _state.update { it.copy(saveQueue = it.saveQueue?.copy(exists = null, error = null)) }
    }

    /**
     * Save the selected player's queue as a playlist called [name]. A playlist of that name
     * is overwritten only with [replace]: otherwise the dialog asks first. Saved: the dialog
     * closes and a message offers to open the playlist.
     */
    fun saveQueue(name: String, replace: Boolean = false) {
        val id = _state.value.selectedId ?: return
        val dialog = _state.value.saveQueue ?: return
        if (dialog.busy) return
        _state.update { it.copy(saveQueue = dialog.copy(busy = true, error = null)) }
        viewModelScope.launch {
            val r = players.saveQueue(id, name, replace)
            val now = _state.value.saveQueue ?: return@launch // closed meanwhile
            val next: SaveQueueDialog? = when (r) {
                is ApiResult.Err -> now.copy(busy = false, exists = null, error = SaveQueueError.Failed(r.error))
                is ApiResult.Ok -> when (val o = r.value) {
                    is io.github.jamezuk.lyrionanywhere.core.SaveQueueOutcome.Saved -> {
                        _messages.trySend(PlayerMessage.SavedPlaylist(o.id, o.name))
                        null
                    }
                    is io.github.jamezuk.lyrionanywhere.core.SaveQueueOutcome.Exists ->
                        now.copy(busy = false, error = null, exists = ExistingPlaylist(o.id, o.name, name))
                    io.github.jamezuk.lyrionanywhere.core.SaveQueueOutcome.Empty ->
                        now.copy(busy = false, exists = null, error = SaveQueueError.Empty)
                }
            }
            _state.update { it.copy(saveQueue = next) }
        }
    }

    /** Empty the queue (after the UI has asked to confirm). */
    fun clearQueue() {
        undo = null
        edit(optimistic = { emptyList() }, tracking = { null to null }) { id -> players.clearQueue(id) }
    }

    /**
     * Take out every track after the current one (after the UI has asked to confirm): from
     * the end, each checked to be the track expected there first, with `playlist delete`.
     * If the queue changes part-way, it stops and says so. Not offered for more than
     * [CLEAR_UPCOMING_MAX] tracks (the UI offers Clear all instead).
     */
    fun clearUpcoming() {
        if (!_state.value.gate.other) return
        val status = _state.value.status ?: return
        val cur = status.queueIndex?.toInt() ?: return
        val len = status.queueLength.toInt()
        val count = len - cur - 1
        if (count <= 0 || count > CLEAR_UPCOMING_MAX) return
        undo = null
        var changed = false
        edit(
            optimistic = { it.take(cur + 1) },
            tracking = { null to cur },
            failed = { e -> if (changed) PlayerMessage.ClearStopped else PlayerMessage.QueueFailed(e) },
        ) { id -> deleteUpcoming(id, cur, len) { changed = true } }
    }

    private suspend fun deleteUpcoming(id: String, cur: Int, len: Int, onChanged: () -> Unit): ApiResult<Unit> {
        val changed = ApiResult.Err(AppError.InvalidInput("the queue changed"))
        val count = len - cur - 1
        fun same(s: PlayerStatus, length: Int) = s.queueIndex?.toInt() == cur && s.queueLength.toInt() == length
        when (val s = players.status(id)) {
            is ApiResult.Err -> return s
            is ApiResult.Ok -> if (!same(s.value, len)) return changed.also { onChanged() }
        }
        val expected = when (val r = players.queue(id, cur + 1, count)) {
            is ApiResult.Err -> return r
            is ApiResult.Ok -> r.value
        }
        if (expected.size != count) return changed.also { onChanged() }
        for (i in count - 1 downTo 0) {
            val index = cur + 1 + i
            // Now and then, check the player hasn't moved on into what is being cleared.
            if (i != count - 1 && (count - 1 - i) % STATUS_CHECK_EVERY == 0) {
                when (val s = players.status(id)) {
                    is ApiResult.Err -> return s
                    is ApiResult.Ok -> if (!same(s.value, index + 1)) return changed.also { onChanged() }
                }
            }
            val trackId = expected[i].id
            if (trackId != null && trackId != 0L) {
                // Checked and removed in one step on the gateway.
                when (val r = players.removeIfStill(id, index, trackId)) {
                    is ApiResult.Err -> return r
                    is ApiResult.Ok -> if (!r.value) return changed.also { onChanged() }
                }
                continue
            }
            when (val at = players.queue(id, index, 1)) {
                is ApiResult.Err -> return at
                is ApiResult.Ok -> if (!QueueEdit.sameTrack(at.value.firstOrNull(), expected[i])) return changed.also { onChanged() }
            }
            val r = players.removeFromQueue(id, index)
            if (r is ApiResult.Err) return r
        }
        return ApiResult.Ok(Unit)
    }

    // --- sync groups -------------------------------------------------------------------------

    /** Put [otherId] in the selected player's group ([on]), or take it out of its group. */
    fun setSynced(otherId: String, on: Boolean) {
        if (!_state.value.gate.other) return
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
        queueWanted = QUEUE_PAGE
        _state.update { it.copy(selectedId = null, status = null, queue = emptyList(), upNext = null, waitingForPhone = true) }
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
        queueWanted = QUEUE_PAGE
        _state.update { it.copy(selectedId = playerId, status = null, queue = emptyList(), upNext = null, waitingForPhone = false) }
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
        // With the server away the controls are greyed out (what works goes to the phone).
        if (!_state.value.gate.online) return
        changeGeneration++
        val now = clock()
        _state.update { s ->
            // Re-base the ticking position on "now" so the change doesn't make it jump.
            val rebased = s.status?.copy(elapsedSecs = elapsedAt(s, now))
            s.copy(status = rebased?.let(optimistic), statusAt = now)
        }
        viewModelScope.launch {
            val r = action(id)
            if (r is ApiResult.Err && !quietFailure(r.error)) _messages.trySend(PlayerMessage.Failed(r.error))
            // The server's view wins once the command is done.
            changeGeneration++
            refreshStatus()
        }
    }

    /**
     * A command refused because the server can't be reached (or is busy) while the screen
     * already says so: no message of its own (one line says it, not one per command).
     */
    private fun quietFailure(e: AppError): Boolean {
        val c = _state.value.connection
        val away = c.reachability != Reachability.ONLINE || c.reconnecting
        return away && (e is AppError.Offline || e is AppError.ServerBusy)
    }

    /**
     * Send [control] to this phone's player while the server is away (true: handled here,
     * whether or not it could be done). False while online: LMS's commands are used.
     */
    private fun onPhone(control: LocalControl, allowed: (ControlGate) -> Boolean): Boolean {
        val g = _state.value.gate
        if (g.online) return false
        if (g.route == ControlRoute.PHONE && allowed(g)) {
            if (localPlayer.localControl(control)) changeGeneration++
        }
        return true
    }

    fun togglePlay() {
        val playing = _state.value.playing
        val g = _state.value.gate
        if (g.route == ControlRoute.HOLD) {
            // The music server is busy: pause or play the phone's own output at once.
            localPlayer.setHold(playing)
            heldWhileBusy = playing
            return
        }
        if (onPhone(if (playing) LocalControl.Pause else LocalControl.Play) { it.playPause }) return
        command(
            optimistic = { it.copy(mode = if (playing) PlaybackMode.PAUSED else PlaybackMode.PLAYING) },
        ) { id -> if (playing) players.setPaused(id, true) else players.play(id) }
    }

    fun next() {
        if (onPhone(LocalControl.Next) { it.next }) return
        command { players.next(it) }
    }

    /** Stop playback (car mode's long press on play/pause). */
    fun stopPlayback() = command(optimistic = { it.copy(mode = PlaybackMode.STOPPED) }) { players.stop(it) }

    fun previous() {
        if (onPhone(LocalControl.Previous) { it.previous }) return
        command { players.previous(it) }
    }

    /**
     * Seek, starting playback there if the player was paused or stopped (as tapping the
     * position bar suggests). LMS keeps a paused player paused after a seek, and ignores a
     * seek on a stopped one, hence the order.
     */
    fun seek(seconds: Double) {
        val wasPlaying = _state.value.playing
        val ms = (seconds.coerceAtLeast(0.0) * 1000).toLong()
        if (onPhone(LocalControl.Seek(ms)) { it.seek }) {
            // As online: a seek plays from there.
            if (_state.value.gate.seek && !wasPlaying) localPlayer.localControl(LocalControl.Play)
            return
        }
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

    /**
     * A hardware volume key ([up] or down) while the app is in front: a step on the selected
     * player, as other LMS controllers do, when that player isn't this phone (the system
     * sets this phone's own volume). Returns whether the key was used.
     */
    fun volumeKey(up: Boolean): Boolean {
        val s = _state.value
        if (s.selectedId == null || s.localSelected) return false
        val status = s.status ?: return false
        // Each step builds on the last one shown, so a held key keeps going.
        setVolume((status.volume.toInt() + if (up) VOLUME_STEP else -VOLUME_STEP).coerceIn(0, 100))
        return true
    }

    /** The shuffle button (Wave 2 §3.5): Off → Tracks → Albums → Off. */
    fun cycleShuffle() = setShuffle(nextShuffle(_state.value.status?.shuffle))

    /** One shuffle mode (the shuffle button's long-press choice). */
    fun setShuffle(mode: ShuffleMode) {
        command(optimistic = { it.copy(shuffle = mode) }) { players.setShuffle(it, mode) }
    }

    fun cycleRepeat() {
        val next = when (_state.value.status?.repeat) {
            RepeatMode.OFF, null -> RepeatMode.ALL
            RepeatMode.ALL -> RepeatMode.ONE
            RepeatMode.ONE -> RepeatMode.OFF
        }
        command(optimistic = { it.copy(repeat = next) }) { players.setRepeat(it, next) }
    }

    fun jumpTo(index: Int) {
        val s = _state.value
        val mark = s.queue.getOrNull(index)?.let { s.markOf(it.track) } ?: QueueMark.NONE
        if (onPhone(LocalControl.Jump(index)) { it.canJump(mark) }) return
        command { players.jumpTo(it, index) }
    }

    fun setPower(on: Boolean) = command(optimistic = { it.copy(powered = on) }) { players.setPower(it, on) }

    /** Play a selection shuffled: shuffle by songs, then replace the queue and play. */
    fun playShuffled(selection: Selection) {
        val id = _state.value.selectedId
        if (id == null) {
            _messages.trySend(PlayerMessage.NoPlayer)
            return
        }
        if (!_state.value.gate.other) return
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
        if (!_state.value.gate.other) return
        viewModelScope.launch {
            val before = if (placement == Placement.PLAY_NEXT) (players.status(id) as? ApiResult.Ok)?.value else null
            when (val r = players.playFavourite(id, itemId, placement)) {
                is ApiResult.Ok -> {
                    _messages.trySend(PlayerMessage.Queued(placement))
                    if (before != null) trackPlayNext(id, before)
                }
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
        if (!_state.value.gate.other) return
        viewModelScope.launch {
            val before = if (placement == Placement.PLAY_NEXT) (players.status(id) as? ApiResult.Ok)?.value else null
            when (val r = players.enqueue(id, selection, placement, playIndex)) {
                is ApiResult.Ok -> {
                    _messages.trySend(PlayerMessage.Queued(placement))
                    if (before != null) trackPlayNext(id, before)
                    // A new queue (Play now) leaves nothing of Up next.
                    if (placement == Placement.PLAY_NOW) _state.update { if (it.selectedId == id) it.copy(upNext = null) else it }
                }
                is ApiResult.Err -> _messages.trySend(PlayerMessage.Failed(r.error))
            }
            refreshStatus()
        }
    }

    /**
     * After a Play next from this device: what went in right after the current track joins
     * Up next. Best effort: worked out from the queue's length before and after; if the
     * player moved on meanwhile it is dropped, and if nothing shows yet it isn't tracked.
     */
    private suspend fun trackPlayNext(id: String, before: PlayerStatus) {
        val cur = before.queueIndex?.toInt() ?: return
        val after = (players.status(id) as? ApiResult.Ok)?.value ?: return
        val added = after.queueLength.toInt() - before.queueLength.toInt()
        if (after.queueIndex?.toInt() != cur || added > UpNextTracker.MAX) {
            _state.update { if (it.selectedId == id) it.copy(upNext = null) else it }
            return
        }
        if (added <= 0) return
        val tracks = (players.queue(id, cur + 1, added) as? ApiResult.Ok)?.value ?: return
        _state.update {
            if (it.selectedId != id) it else it.copy(upNext = UpNextTracker.inserted(UpNextTracker.advanced(it.upNext, cur), cur, tracks))
        }
    }

    /**
     * A station started from Home keeps going: each status may add another batch. Not
     * cancelled by the next status (a cancelled call would leave the runner waiting on a
     * batch for ever); the runner itself makes sure only one batch is fetched at a time.
     * [readQueue]: read the queue afresh (in the background, where it isn't kept).
     */
    private suspend fun followStation(id: String, status: PlayerStatus, readQueue: Boolean) {
        val m = me ?: return
        if (m.activeStation.value == null) return
        val queued = if (readQueue || _state.value.queue.isEmpty()) {
            (players.queue(id, 0, QUEUE_PAGE) as? ApiResult.Ok)?.value?.mapNotNull { it.id } ?: return
        } else {
            _state.value.queue.mapNotNull { it.track.id }
        }
        val update = withContext(NonCancellable) {
            m.onStatus(id, status.queueLength.toInt(), status.queueIndex?.toInt() ?: 0, status.current?.id, queued)
        }
        if (update is RunnerUpdate.Stopped) _messages.trySend(PlayerMessage.StationStopped(update.reason))
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
        if (!_state.value.gate.other) return
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
        if (!_state.value.gate.other) return
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

    /**
     * Clear every rating of this user's (Settings, after asking): the number cleared, or null
     * if it failed. The ratings kept in memory are forgotten, and the current track's rating
     * is read again, so no cleared rating is still shown.
     */
    suspend fun clearRatings(): Long? {
        val repo = ratings ?: return null
        val r = repo.clear()
        if (r !is ApiResult.Ok) return null
        ratingsCache?.clear()
        val id = _state.value.status?.current?.id
        _state.update { it.copy(rating = null, ratingFailed = false) }
        if (id != null && id > 0) {
            val fresh = ratingsCache?.get(id)
            _state.update { s -> if (s.status?.current?.id == id && s.rating == null) s.copy(rating = fresh) else s }
        }
        return r.value
    }

    /** Stars or a heart (kept on the server for this user). */
    fun setRatingStyle(style: RatingStyle) {
        _state.update { it.copy(ratingStyle = style) }
        viewModelScope.launch { ratings?.setStyle(style) }
    }

    // --- Now playing's layouts and visualiser (Wave 2 §4) --------------------------------------

    private fun setPrefs(change: (NowPlayingPrefs) -> NowPlayingPrefs) {
        val p = change(_state.value.nowPlaying)
        _state.update { it.copy(nowPlaying = p) }
        viewModelScope.launch { settings.setNowPlayingPrefs(p) }
    }

    /** Now playing's layout (remembered on this device). */
    fun setLayout(layout: NowPlayingLayout) = setPrefs { it.copy(layout = layout) }

    /** The visualiser on or off (remembered on this device). */
    fun setVisualiser(on: Boolean) = setPrefs { it.copy(visualiser = on) }

    fun setVisualStyle(style: VisualStyle) = setPrefs { it.copy(visualStyle = style) }

    /** Wide windows: show or hide the queue panel. */
    fun setQueuePanel(shown: Boolean) = setPrefs { it.copy(queuePanel = shown) }

    /** The visualiser on screen asks this phone's player to analyse (or stop). */
    override fun setActive(on: Boolean) = localPlayer.setVisualiser(on)

    override fun frame(): VisualFrame? = localPlayer.visualFrame()

    private var creditsLoading: Long? = null

    /** The current track's credits, for the Info layout: read once per track while shown. */
    fun loadCredits() {
        val repo = library ?: return
        val id = _state.value.status?.current?.id?.takeIf { it > 0 } ?: return
        if (_state.value.creditsTrackId == id || creditsLoading == id) return
        creditsLoading = id
        viewModelScope.launch {
            val r = repo.trackCredits(id)
            creditsLoading = null
            val list = (r as? ApiResult.Ok)?.value ?: return@launch
            _state.update { s -> if (s.status?.current?.id == id) s.copy(credits = list, creditsTrackId = id) else s }
        }
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

        /** "Clear upcoming" takes out at most this many tracks; beyond it, Clear all is offered. */
        const val CLEAR_UPCOMING_MAX = 100

        /** While clearing, the player's position is checked again this often (tracks). */
        private const val STATUS_CHECK_EVERY = 10

        /** What the shuffle button goes to next. */
        fun nextShuffle(mode: ShuffleMode?): ShuffleMode = when (mode) {
            ShuffleMode.OFF, null -> ShuffleMode.SONGS
            ShuffleMode.SONGS -> ShuffleMode.ALBUMS
            ShuffleMode.ALBUMS -> ShuffleMode.OFF
        }

        /** The position in the current track at time [now] (ticking while playing). */
        fun elapsedAt(state: PlayerUiState, now: Long): Double {
            val status = state.status ?: return 0.0
            val base = status.elapsedSecs
            val running = if (status.mode == PlaybackMode.PLAYING) (now - state.statusAt).coerceAtLeast(0) / 1000.0 else 0.0
            val pos = base + running
            return status.durationSecs?.let { pos.coerceAtMost(it) } ?: pos
        }

        const val EVENT_COALESCE_MS = 150L
        /** Polling while the UI isn't visible and a station or sleep timer needs it. */
        const val BACKGROUND_POLL_MS = 20_000L
        /** Polling while the event stream is down. */
        const val FAST_POLL_MS = 5_000L
        /** A safety net while it is up. */
        const val SLOW_POLL_MS = 30_000L
        /** Polling while the stream is down and the music server is busy. */
        const val BUSY_POLL_MS = 15_000L
        const val QUEUE_PAGE = 200
        /** The most queue entries kept in memory (a bound on what scrolling can load). */
        const val QUEUE_MAX = 5_000
        /** A volume key's step, per cent. */
        const val VOLUME_STEP = 5
        /** Players whose status the "Play on…" sheet reads. */
        const val MAX_OUTPUT_STATUS = 20
    }
}
