package io.github.jamezuk.lyrionanywhere.ui.offline

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.DownloadItem
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.NowPlayingPrefs
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.VisualFrame
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.data.offline.DownloadSettings
import io.github.jamezuk.lyrionanywhere.ui.player.VisualSource
import kotlinx.coroutines.flow.distinctUntilChanged
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** The Downloads screen's state. */
data class DownloadsUiState(
    /** Downloads not yet finished: running first, then waiting, queued, paused, failed. */
    val inProgress: List<DownloadItem> = emptyList(),
    val albums: List<OfflineAlbum> = emptyList(),
    val trackCount: Int = 0,
    val usedBytes: Long = 0,
    val capBytes: Long = 0,
    /** The server refused downloads for this account. */
    val forbidden: Boolean = false,
    /** Downloads wait for Wi-Fi (or any connection). */
    val waitingFor: NetworkKind? = null,
    /** Signed in: new downloads can start. */
    val canDownload: Boolean = false,
    val pendingPlays: Int = 0,
) {
    val empty: Boolean get() = inProgress.isEmpty() && albums.isEmpty()
    val anyRunning: Boolean get() = inProgress.any { it.state != DownloadState.PAUSED && it.state != DownloadState.FAILED }
    val anyPaused: Boolean get() = inProgress.any { it.state == DownloadState.PAUSED || it.state == DownloadState.FAILED }
}

private val ORDER = listOf(
    DownloadState.ACTIVE,
    DownloadState.WAITING,
    DownloadState.QUEUED,
    DownloadState.PAUSED,
    DownloadState.FAILED,
)

/** Pure: unit-tested. */
fun downloadsUi(s: OfflineState): DownloadsUiState {
    val d = s.downloads
    val pending = d?.items.orEmpty()
        .filter { it.state != DownloadState.DONE }
        .sortedBy { ORDER.indexOf(it.state) }
    return DownloadsUiState(
        inProgress = pending,
        albums = s.albums,
        trackCount = s.tracks.size,
        usedBytes = s.usedBytes,
        capBytes = s.settings.capBytes,
        forbidden = d?.forbidden == true,
        waitingFor = when {
            d?.waitingForNetwork != true -> null
            s.network == NetworkKind.OFFLINE -> NetworkKind.OFFLINE
            else -> NetworkKind.WIFI
        },
        canDownload = s.canDownload,
        pendingPlays = s.pendingPlays,
    )
}

/** A download's progress, 0…1 (null when the size isn't known yet). */
fun progressOf(item: DownloadItem): Float? =
    if (item.total > 0uL) (item.received.toDouble() / item.total.toDouble()).toFloat().coerceIn(0f, 1f) else null

class DownloadsViewModel(private val repo: OfflineRepository) : ViewModel() {
    val state: StateFlow<DownloadsUiState> =
        repo.state.map(::downloadsUi).stateIn(viewModelScope, SharingStarted.Eagerly, downloadsUi(repo.state.value))

    fun pause(trackId: Long?) = repo.pause(trackId)

    fun resume(trackId: Long?) = repo.resume(trackId)

    fun cancel(trackId: Long?) = repo.cancel(trackId)

    fun clearFinished() = repo.clearFinished()

    fun removeAlbum(albumRef: String) {
        viewModelScope.launch { repo.removeAlbum(albumRef) }
    }

    fun removeAll() {
        viewModelScope.launch { repo.removeAll() }
    }
}

/** What happened when the user asked for a download. */
sealed interface DownloadMessage {
    data class Queued(val count: Int) : DownloadMessage

    data class Failed(val error: AppError) : DownloadMessage
}

/** "Download" on album, playlist and track pages, and in "More…". */
class DownloadActions(private val repo: OfflineRepository, private val scope: kotlinx.coroutines.CoroutineScope) {
    private val _messages = MutableSharedFlow<DownloadMessage>(extraBufferCapacity = 4, onBufferOverflow = BufferOverflow.DROP_OLDEST)
    val messages: SharedFlow<DownloadMessage> = _messages.asSharedFlow()

    private fun report(r: ApiResult<Int>) {
        _messages.tryEmit(
            when (r) {
                is ApiResult.Ok -> DownloadMessage.Queued(r.value)
                is ApiResult.Err -> DownloadMessage.Failed(r.error)
            },
        )
    }

    fun album(albumId: Long) {
        scope.launch { report(repo.downloadAlbum(albumId)) }
    }

    fun playlist(playlistId: Long) {
        scope.launch { report(repo.downloadPlaylist(playlistId)) }
    }

    /** One library track, by id (from "More…"). */
    fun track(trackId: Long) = tracks(
        listOf(io.github.jamezuk.lyrionanywhere.core.Track(trackId, "", null, null, null, null, null, null, null, null, null, null, null, null, null)),
    )

    fun tracks(tracks: List<io.github.jamezuk.lyrionanywhere.core.Track>) {
        val n = tracks.count { (it.id ?: 0) > 0 }
        if (!repo.state.value.canDownload) {
            _messages.tryEmit(DownloadMessage.Failed(AppError.SignedOut))
            return
        }
        repo.downloadTracks(tracks)
        _messages.tryEmit(DownloadMessage.Queued(n))
    }
}

/** How much of a list of tracks is on the device. */
enum class DownloadMark { NONE, SOME, ALL }

fun downloadMark(trackIds: List<Long>, downloaded: Set<Long>): DownloadMark {
    if (trackIds.isEmpty()) return DownloadMark.NONE
    val n = trackIds.count { it in downloaded }
    return when (n) {
        0 -> DownloadMark.NONE
        trackIds.size -> DownloadMark.ALL
        else -> DownloadMark.SOME
    }
}

/** Storage settings: the cap, the formats, Wi-Fi only and smart downloads. */
class StorageSettingsViewModel(private val repo: OfflineRepository) : ViewModel() {
    val state: StateFlow<OfflineState> = repo.state

    private val _refreshing = MutableStateFlow(false)
    val refreshing: StateFlow<Boolean> = _refreshing.asStateFlow()

    private val _error = MutableStateFlow<AppError?>(null)
    val error: StateFlow<AppError?> = _error.asStateFlow()

    fun update(change: (DownloadSettings) -> DownloadSettings) {
        val next = change(repo.state.value.settings).sanitised()
        viewModelScope.launch { repo.setSettings(next) }
    }

    fun refreshSmart() {
        if (_refreshing.value) return
        _refreshing.value = true
        viewModelScope.launch {
            _error.value = (repo.refreshSmart() as? ApiResult.Err)?.error
            _refreshing.value = false
        }
    }
}

/** A downloaded album: its tracks, in order, ready to play offline. */
data class OfflineAlbumUi(val album: OfflineAlbum?, val tracks: List<OfflineTrack>)

class OfflineAlbumViewModel(
    private val repo: OfflineRepository,
    private val player: OfflinePlayerRepository,
    private val albumRef: String,
) : ViewModel() {
    val state: StateFlow<OfflineAlbumUi> = repo.state.map { s -> albumOf(s, albumRef) }
        .stateIn(viewModelScope, SharingStarted.Eagerly, albumOf(repo.state.value, albumRef))

    fun play(start: Int = 0, shuffle: Boolean = false) {
        val tracks = state.value.tracks
        if (tracks.isNotEmpty()) player.play(tracks, start.coerceIn(0, tracks.size - 1), shuffle)
    }

    fun remove() {
        viewModelScope.launch { repo.removeAlbum(albumRef) }
    }

    companion object {
        /** The album's tracks by number, then title. Pure. */
        fun albumOf(s: OfflineState, ref: String): OfflineAlbumUi {
            val tracks = s.tracks.filter { refOf(it) == ref }
                .sortedWith(compareBy<OfflineTrack> { it.trackNumber ?: UInt.MAX_VALUE }.thenBy { it.title })
            return OfflineAlbumUi(s.albums.firstOrNull { it.albumRef == ref }, tracks)
        }

        fun refOf(t: OfflineTrack): String = if (t.albumId != null) "a${t.albumId}" else "t${t.key.take(16)}"
    }
}

/** The offline player for screens: its state, with the position ticking between reports. */
data class OfflinePlayerUi(
    val state: OfflinePlayerState = OfflinePlayerState(),
    /** When the state arrived (the screen's clock). */
    val at: Long = 0,
    /** Now playing's choices on this device: the visualiser and its style (Wave 2 §5.7). */
    val nowPlaying: NowPlayingPrefs = NowPlayingPrefs(),
    /** The quality badge and Download → This device diagram of the track playing (null: none). */
    val diagram: io.github.jamezuk.lyrionanywhere.core.QualityDiagram? = null,
) {
    /** The position now (ms), extrapolated while playing. */
    fun positionAt(now: Long): Long {
        val s = state.status ?: return 0
        val base = s.elapsedMs.toLong()
        val p = if (s.playback == io.github.jamezuk.lyrionanywhere.core.LocalPlayback.PLAYING) base + (now - at).coerceAtLeast(0) else base
        val d = s.durationMs.toLong()
        return if (d > 0) p.coerceAtMost(d) else p
    }
}

class OfflinePlayerViewModel(
    private val player: OfflinePlayerRepository,
    private val clock: () -> Long,
    /** Where the visualiser's choices are kept (none: not remembered). */
    private val settings: SettingsStore? = null,
    /** The quality badge's and signal path's rules (none: no badge). */
    private val quality: io.github.jamezuk.lyrionanywhere.data.QualityDescriber? = null,
) : ViewModel(), VisualSource {
    private val _state = MutableStateFlow(OfflinePlayerUi(player.state.value, clock()))
    val state: StateFlow<OfflinePlayerUi> = _state.asStateFlow()

    /** What the diagram was last built from, and the diagram. */
    private var diagramOf: Pair<io.github.jamezuk.lyrionanywhere.core.SignalPathInfo?, io.github.jamezuk.lyrionanywhere.core.OfflineTrack>? = null
    private var diagram: io.github.jamezuk.lyrionanywhere.core.QualityDiagram? = null

    /**
     * The download's diagram: the track (its format, and the original it was made from) and
     * the engine's path.
     */
    private fun diagramFor(s: OfflinePlayerState): io.github.jamezuk.lyrionanywhere.core.QualityDiagram? {
        val describer = quality ?: return null
        val t = s.current ?: return null
        val key = s.signalPath to t
        if (key != diagramOf) {
            diagramOf = key
            diagram = runCatching { describer.offline(s.signalPath, t) }.getOrNull()
        }
        return diagram
    }

    init {
        viewModelScope.launch {
            player.state.collect { s -> _state.update { it.copy(state = s, at = clock(), diagram = diagramFor(s)) } }
        }
        settings?.let { store ->
            viewModelScope.launch {
                store.settings.map { it.nowPlaying }.distinctUntilChanged().collect { p -> _state.update { it.copy(nowPlaying = p) } }
            }
        }
    }

    fun toggle() = player.toggle()

    fun next() = player.next()

    fun previous() = player.previous()

    fun seek(seconds: Double) = player.seekTo((seconds * 1000).toLong().coerceAtLeast(0))

    fun jumpTo(index: Int) = player.jumpTo(index)

    fun toggleShuffle() = player.setShuffle(!(_state.value.state.status?.shuffle ?: false))

    /** Off → all → one → off, as Media3 and LMS cycle. */
    fun cycleRepeat() = player.setRepeat(
        when (_state.value.state.status?.repeat ?: LocalRepeat.OFF) {
            LocalRepeat.OFF -> LocalRepeat.ALL
            LocalRepeat.ALL -> LocalRepeat.ONE
            LocalRepeat.ONE -> LocalRepeat.OFF
        },
    )

    fun stop() = player.stop()

    // The visualiser (Wave 2 §5.7): the same engine analysis as streaming, from the downloads.
    override fun setActive(on: Boolean) = player.setVisualiser(on)

    override fun frame(): VisualFrame? = player.visualFrame()

    /** The visualiser on or off (remembered on this device, as on the online Now playing). */
    fun setVisualiser(on: Boolean) = savePrefs { it.copy(visualiser = on) }

    fun setVisualStyle(style: VisualStyle) = savePrefs { it.copy(visualStyle = style) }

    private fun savePrefs(change: (NowPlayingPrefs) -> NowPlayingPrefs) {
        val p = change(_state.value.nowPlaying)
        _state.update { it.copy(nowPlaying = p) }
        val store = settings ?: return
        viewModelScope.launch { store.setNowPlayingPrefs(p) }
    }
}
