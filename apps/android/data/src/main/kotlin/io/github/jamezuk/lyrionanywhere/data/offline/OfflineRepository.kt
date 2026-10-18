package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.DownloadObserver
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.DownloadRequest
import io.github.jamezuk.lyrionanywhere.core.DownloadsSnapshot
import io.github.jamezuk.lyrionanywhere.core.FormatChoice
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.PlaysSyncOutcome
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.toAppError
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull

/** Everything the app shows about downloads. */
data class OfflineState(
    /** Whose downloads are open (null: none, e.g. signed out). */
    val owner: Owner? = null,
    val albums: List<OfflineAlbum> = emptyList(),
    val tracks: List<OfflineTrack> = emptyList(),
    /** The download manager's view; null while it isn't running (offline or signed out). */
    val downloads: DownloadsSnapshot? = null,
    val usedBytes: Long = 0,
    val pendingPlays: Int = 0,
    val settings: DownloadSettings = DownloadSettings(),
    val network: NetworkKind = NetworkKind.WIFI,
    /** Signed in: downloads can be started. */
    val canDownload: Boolean = false,
    val error: AppError? = null,
) {
    val hasDownloads: Boolean get() = tracks.isNotEmpty()

    /** LMS track ids on the device (for "downloaded" marks). */
    val downloadedIds: Set<Long> get() = tracks.mapTo(HashSet()) { it.trackId }
}

/** Downloads, the offline library and smart downloads (milestone A5). */
interface OfflineRepository {
    val state: StateFlow<OfflineState>

    /** Download library tracks (those without an id are skipped). */
    fun downloadTracks(tracks: List<Track>)

    /** Download an album's tracks; the number queued. */
    suspend fun downloadAlbum(albumId: Long): ApiResult<Int>

    /** Download a saved playlist's tracks; the number queued. */
    suspend fun downloadPlaylist(playlistId: Long): ApiResult<Int>

    fun pause(trackId: Long?)

    fun resume(trackId: Long?)

    fun cancel(trackId: Long?)

    fun clearFinished()

    suspend fun remove(keys: List<String>)

    suspend fun removeAlbum(albumRef: String)

    suspend fun removeAll()

    /** An album's cached cover. */
    suspend fun artwork(albumRef: String): ByteArray?

    suspend fun setSettings(settings: DownloadSettings)

    /** Bring smart downloads up to date now (Wi-Fi only). */
    suspend fun refreshSmart(): ApiResult<Unit>

    /**
     * The user is signing out: send any plays waiting, then forget this account's
     * downloads on this device (removing them too if asked). The caller then signs out.
     */
    suspend fun signOut(removeDownloads: Boolean)

    /** Queue a rating of a downloaded track made while the server can't be reached. */
    suspend fun rateOffline(trackId: Long, rating: Int): Boolean = false
}

/**
 * The real repository: opens the offline library of the user signed in (or, offline, the
 * last one signed in on this server), runs the download manager while signed in, sends
 * plays when online, keeps smart downloads up to date on Wi-Fi, and removes everything when
 * the server revokes this device.
 */
class RustOfflineRepository(
    /** App-private storage for downloads (never backed up). */
    private val root: String,
    private val auth: StateFlow<AuthState>,
    /** The signed-in session's link (null while signed out). */
    private val link: StateFlow<OnlineLink?>,
    private val settings: SettingsStore,
    private val network: StateFlow<NetworkKind>,
    private val library: LibraryRepository,
    private val scope: CoroutineScope,
    private val io: CoroutineDispatcher,
    private val openStore: (String, Owner) -> OfflineStore = RustOfflineStore::open,
    private val wipe: (String, Owner) -> Unit = RustOfflineStore::wipe,
    private val clock: () -> Long = System::currentTimeMillis,
) : OfflineRepository {
    private val _state = MutableStateFlow(OfflineState())
    override val state: StateFlow<OfflineState> = _state.asStateFlow()

    private val _store = MutableStateFlow<OfflineStore?>(null)

    /** The open library, for the offline player. */
    val store: StateFlow<OfflineStore?> = _store.asStateFlow()

    /** The open library if it is [owner]'s, else null ("Continue on" looks only there; the
     *  Rust layer checks again that it is the signed-in account's). */
    fun storeOf(owner: Owner): OfflineStore? {
        val s = _store.value ?: return null
        return s.takeIf { _state.value.owner == owner && _store.value === s }
    }

    private val lock = Mutex()
    private var owner: Owner? = null
    private var lastSignedIn: Owner? = null
    private var downloader: DownloaderHandle? = null
    private var downloaderLink: OnlineLink? = null
    /** Signing out: don't remember this account as the offline owner again. */
    private var signingOut = false
    private var lastSmartAt = 0L
    private var lastSmartSettings: DownloadSettings? = null

    private val refreshes = MutableSharedFlow<Unit>(extraBufferCapacity = 1, onBufferOverflow = BufferOverflow.DROP_OLDEST)

    private val observer = object : DownloadObserver {
        override fun onDownloads(snapshot: DownloadsSnapshot) {
            _state.update { it.copy(downloads = snapshot, usedBytes = snapshot.usedBytes.toLong()) }
        }

        override fun onLibraryChanged() {
            refreshes.tryEmit(Unit)
        }
    }

    val playsSender = PlaysSender(
        scope,
        combine(link, _state, network) { l, s, n -> l != null && s.owner != null && s.pendingPlays > 0 && n != NetworkKind.OFFLINE },
        send = { sendPlays() },
    )

    init {
        scope.launch {
            combine(auth, settings.settings.map { it.offlineOwner }.distinctUntilChanged(), link) { a, o, l ->
                Triple(a, o, l)
            }.collect { (a, stored, l) -> lock.withLock { apply(a, stored, l) } }
        }
        scope.launch {
            settings.settings.map { it.downloads }.distinctUntilChanged().collect { d ->
                _state.update { it.copy(settings = d) }
                lock.withLock { downloader?.setPolicy(d.policy()) }
            }
        }
        scope.launch {
            network.collect { n ->
                _state.update { it.copy(network = n) }
                lock.withLock { downloader?.setNetwork(n) }
            }
        }
        scope.launch {
            refreshes.collectLatest {
                delay(REFRESH_COALESCE_MS)
                reload()
            }
        }
        scope.launch {
            // Smart downloads: when signed in, on Wi-Fi, and the settings ask for them.
            combine(link, network, settings.settings.map { it.downloads }.distinctUntilChanged()) { l, n, d ->
                Triple(l != null, n == NetworkKind.WIFI, d)
            }.collectLatest { (online, wifi, d) ->
                if (online && wifi && d.smartOn) {
                    val due = d != lastSmartSettings || clock() - lastSmartAt >= SMART_EVERY_MS
                    if (due) refreshSmart()
                }
            }
        }
        playsSender.start()
    }

    /** Whose downloads to show for this sign-in state. */
    private fun ownerFor(a: AuthState, stored: String?): Owner? = when (a) {
        is AuthState.SignedInState -> Owner(a.server.url, a.who.user.username)
        is AuthState.Unreachable -> stored?.let { Owner(a.server.url, it) }
        is AuthState.Restoring -> stored?.let { Owner(a.server.url, it) }
        else -> null
    }

    private suspend fun apply(a: AuthState, stored: String?, l: OnlineLink?) {
        if (a !is AuthState.SignedInState) signingOut = false
        if (a is AuthState.SignedInState && !signingOut) {
            val me = Owner(a.server.url, a.who.user.username)
            lastSignedIn = me
            if (stored != me.username) settings.setOfflineOwner(me.username)
        }
        if (a is AuthState.SignedOut && a.revoked) {
            // The server ended this device's sign-in: its downloads go (SPEC §8.5).
            val victim = lastSignedIn?.takeIf { it.serverUrl == a.server.url }
                ?: stored?.let { Owner(a.server.url, it) }
            if (victim != null) {
                closeStore()
                withContext(io) { runCatching { wipe(root, victim) } }
                lastSignedIn = null
                settings.setOfflineOwner(null)
            }
        }
        val target = if (signingOut) null else ownerFor(a, stored)
        if (target != owner) {
            closeStore()
            if (target != null) {
                val s = withContext(io) { runCatching { openStore(root, target) } }
                s.onSuccess {
                    owner = target
                    _store.value = it
                    _state.update { st -> st.copy(owner = target, error = null) }
                }.onFailure { e ->
                    _state.update { st -> st.copy(error = (e as? Exception)?.toAppError() ?: AppError.Unexpected("storage")) }
                }
                reload()
            }
        }
        reconcileDownloader(if (a is AuthState.SignedInState) l else null)
    }

    private suspend fun closeStore() {
        stopDownloader()
        val s = _store.value
        _store.value = null
        owner = null
        _state.update { OfflineState(settings = it.settings, network = it.network) }
        if (s != null) withContext(io) { runCatching { s.close() } }
    }

    private suspend fun reconcileDownloader(l: OnlineLink?) {
        val s = _store.value
        if (l == null || s == null) {
            stopDownloader()
            return
        }
        if (downloader != null && downloaderLink === l) return
        stopDownloader()
        val d = settings.settings.first().downloads
        val handle = withContext(io) { runCatching { l.downloader(s, d.policy(), network.value, observer) } }.getOrNull()
        downloader = handle
        downloaderLink = l.takeIf { handle != null }
        _state.update { it.copy(canDownload = handle != null) }
    }

    private suspend fun stopDownloader() {
        val d = downloader ?: return
        downloader = null
        downloaderLink = null
        _state.update { it.copy(canDownload = false, downloads = null) }
        withContext(io) { runCatching { d.shutdown() } }
    }

    /** Re-read the library (after downloads finish, or removals). */
    private suspend fun reload() {
        val s = _store.value ?: return
        val r = withContext(io) {
            runCatching { Snapshot(s.tracks(), s.albums(), s.usedBytes(), s.pendingPlays()) }
        }.getOrNull() ?: return
        if (_store.value !== s) return
        _state.update {
            it.copy(tracks = r.tracks, albums = r.albums, usedBytes = r.usedBytes, pendingPlays = r.pendingPlays)
        }
    }

    private class Snapshot(
        val tracks: List<OfflineTrack>,
        val albums: List<OfflineAlbum>,
        val usedBytes: Long,
        val pendingPlays: Int,
    )

    /** One attempt at sending plays; null when not possible now. */
    private suspend fun sendPlays(): PlaysSyncOutcome? {
        val (l, s) = lock.withLock { link.value to _store.value }
        if (l == null || s == null) return null
        val r = runCatching { l.syncPlays(s) }.getOrNull()
        // Ratings made offline go back with the plays.
        runCatching { l.syncRatings(s) }
        reload()
        return r
    }

    private fun requests(tracks: List<Track>, albumId: Long?, origin: DownloadOrigin) =
        tracks.mapNotNull { t -> t.id?.takeIf { it > 0 }?.let { DownloadRequest(it, t.albumId ?: albumId, origin) } }

    private suspend fun enqueue(requests: List<DownloadRequest>): ApiResult<Int> {
        if (requests.isEmpty()) return ApiResult.Ok(0)
        val d = lock.withLock { downloader } ?: return ApiResult.Err(notSignedIn())
        withContext(io) { d.enqueue(requests, FormatChoice.AUTO) }
        return ApiResult.Ok(requests.size)
    }

    private fun notSignedIn(): AppError =
        if (auth.value is AuthState.SignedInState) AppError.Offline() else AppError.SignedOut

    override fun downloadTracks(tracks: List<Track>) {
        scope.launch { enqueue(requests(tracks, null, DownloadOrigin.MANUAL)) }
    }

    override suspend fun downloadAlbum(albumId: Long): ApiResult<Int> = when (val r = library.albumTracks(albumId)) {
        is ApiResult.Ok -> enqueue(requests(r.value, albumId, DownloadOrigin.MANUAL))
        is ApiResult.Err -> r
    }

    override suspend fun downloadPlaylist(playlistId: Long): ApiResult<Int> =
        when (val r = library.playlistTracks(playlistId)) {
            is ApiResult.Ok -> enqueue(requests(r.value, null, DownloadOrigin.MANUAL))
            is ApiResult.Err -> r
        }

    private fun withDownloader(block: (DownloaderHandle) -> Unit) {
        scope.launch {
            val d = lock.withLock { downloader } ?: return@launch
            withContext(io) { block(d) }
        }
    }

    override fun pause(trackId: Long?) = withDownloader { it.pause(trackId) }

    override fun resume(trackId: Long?) = withDownloader { it.resume(trackId) }

    override fun cancel(trackId: Long?) = withDownloader { it.cancel(trackId) }

    override fun clearFinished() = withDownloader { it.clearFinished() }

    private suspend fun onStore(block: (OfflineStore) -> Unit) {
        val s = _store.value ?: return
        val r = withContext(io) { runCatching { block(s) } }
        r.exceptionOrNull()?.let { e -> _state.update { it.copy(error = (e as? Exception)?.toAppError()) } }
        reload()
    }

    override suspend fun remove(keys: List<String>) = onStore { it.remove(keys) }

    override suspend fun removeAlbum(albumRef: String) = onStore { it.removeAlbum(albumRef) }

    override suspend fun removeAll() {
        lock.withLock { downloader }?.let { d -> withContext(io) { d.cancel(null) } }
        onStore { it.removeAll() }
    }

    override suspend fun artwork(albumRef: String): ByteArray? {
        val s = _store.value ?: return null
        return withContext(io) { runCatching { s.artwork(albumRef) }.getOrNull() }
    }

    override suspend fun setSettings(settings: DownloadSettings) {
        this.settings.setDownloads(settings.sanitised())
    }

    override suspend fun refreshSmart(): ApiResult<Unit> {
        val d = settings.settings.first()
        if (network.value != NetworkKind.WIFI) return ApiResult.Err(AppError.InvalidInput("smart downloads wait for Wi-Fi"))
        val s = _store.value ?: return ApiResult.Err(notSignedIn())
        if (lock.withLock { downloader } == null) return ApiResult.Err(notSignedIn())
        val wanted = if (d.downloads.smartOn) {
            when (val w = SmartDownloads(library).wanted(d.downloads, d.libraryId)) {
                is ApiResult.Ok -> w.value
                is ApiResult.Err -> return w
            }
        } else {
            emptyList()
        }
        val plan = withContext(io) { runCatching { s.planSmart(wanted) } }.getOrElse {
            return ApiResult.Err((it as? Exception)?.toAppError() ?: AppError.Unexpected("plan"))
        }
        if (plan.remove.isNotEmpty()) onStore { it.remove(plan.remove) }
        enqueue(plan.download)
        lastSmartAt = clock()
        lastSmartSettings = d.downloads
        return ApiResult.Ok(Unit)
    }

    override suspend fun rateOffline(trackId: Long, rating: Int): Boolean {
        val s = lock.withLock { _store.value } ?: return false
        val key = s.tracks().firstOrNull { it.trackId == trackId }?.key ?: return false
        return withContext(io) { s.rateOffline(trackId, key, rating, clock() / 1000) }
    }

    /** Called as the user signs out on purpose (the "Continue on" offer forgets its queue). */
    @Volatile var beforeSignOut: () -> Unit = {}

    override suspend fun signOut(removeDownloads: Boolean) {
        beforeSignOut()
        // Plays heard offline go first, while the sign-in still works.
        withTimeoutOrNull(SIGN_OUT_SYNC_MS) { sendPlays() }
        lock.withLock {
            signingOut = true
            val o = owner
            closeStore()
            if (removeDownloads && o != null) withContext(io) { runCatching { wipe(root, o) } }
            settings.setOfflineOwner(null)
            lastSignedIn = null
        }
    }

    companion object {
        const val REFRESH_COALESCE_MS = 150L

        /** Smart downloads are refreshed at most this often (unless their settings change). */
        const val SMART_EVERY_MS = 6 * 60 * 60 * 1000L
        const val SIGN_OUT_SYNC_MS = 5_000L
    }
}
