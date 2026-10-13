package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.IndexEntry
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LibraryAlbumOrder
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.data.LibraryPrefs
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * A long list read a page at a time, wherever it is looked at (Wave 2 §3.2): jumping to a
 * letter far down reads only the page there, not everything before it. [get] is null for
 * entries not read yet (shown as placeholders).
 */
data class SparseState<T>(
    /** How many entries the list has (null until the first page is read). */
    val total: Int? = null,
    val pages: Map<Int, List<T>> = emptyMap(),
    val loading: Set<Int> = emptySet(),
    val error: AppError? = null,
    /** The A–Z index (empty: no scroller). */
    val index: List<IndexEntry> = emptyList(),
) {
    operator fun get(i: Int): T? = pages[i / SparsePagedViewModel.PAGE]?.getOrNull(i % SparsePagedViewModel.PAGE)

    val count: Int get() = total ?: 0

    /** Nothing read yet, and the first page failed. */
    val failedEmpty: Boolean get() = total == null && error != null
}

abstract class SparsePagedViewModel<T> : ViewModel() {
    protected val _state = MutableStateFlow(SparseState<T>())
    val state: StateFlow<SparseState<T>> = _state.asStateFlow()

    /** Bumped by every restart, so an answer for an older list is dropped. */
    private var generation = 0

    protected abstract suspend fun page(start: Int, count: Int): ApiResult<Pair<List<T>, Int>>

    /** The A–Z index for the list as it is now (none by default). */
    protected open suspend fun index(): ApiResult<List<IndexEntry>> = ApiResult.Ok(emptyList())

    /** Read the page holding entry [i], unless it is read or being read. */
    fun ensure(i: Int) {
        val s = _state.value
        if (i < 0 || (s.total != null && i >= s.total)) return
        val p = i / PAGE
        if (p in s.pages || p in s.loading || (s.error != null && s.total == null && p > 0)) return
        _state.update { it.copy(loading = it.loading + p, error = null) }
        val gen = generation
        viewModelScope.launch {
            val r = page(p * PAGE, PAGE)
            if (gen != generation) return@launch
            _state.update { st ->
                when (r) {
                    is ApiResult.Err -> st.copy(loading = st.loading - p, error = r.error)
                    is ApiResult.Ok -> {
                        val (items, reported) = r.value
                        val got = items.take(PAGE)
                        var total = st.total ?: reported.coerceIn(0, MAX_TOTAL)
                        // A short page ends the list there, whatever the count said.
                        if (got.size < PAGE) total = minOf(total, p * PAGE + got.size)
                        // A full page past the end: the count was low (as far as this goes).
                        total = maxOf(total, p * PAGE + got.size).coerceAtMost(MAX_TOTAL)
                        st.copy(total = total, pages = st.pages + (p to got), loading = st.loading - p)
                    }
                }
            }
        }
    }

    /** Start over (another order, library or view): the first page and the index again. */
    protected fun restart() {
        generation++
        val gen = generation
        _state.value = SparseState()
        ensure(0)
        viewModelScope.launch {
            val idx = (index() as? ApiResult.Ok)?.value.orEmpty()
            if (gen == generation) _state.update { it.copy(index = idx) }
        }
    }

    fun retry() {
        val s = _state.value
        _state.update { it.copy(error = null) }
        if (s.total == null) ensure(0)
    }

    companion object {
        const val PAGE = 100

        /** The most entries a list shows (a bound on what a server's count can make it). */
        const val MAX_TOTAL = 200_000
    }
}

/** Where the A–Z scroller stands, and where its letters lead (pure, so it is unit-tested). */
object AtoZ {
    /** The entry whose letter holds list position [first] (the first one visible). */
    fun currentAt(index: List<IndexEntry>, first: Int): Int {
        if (index.isEmpty()) return -1
        val i = index.indexOfLast { it.offset.toLong() <= first.toLong() }
        return if (i < 0) 0 else i
    }

    /** Where a letter leads, kept within a list of [total] entries. */
    fun target(entry: IndexEntry, total: Int): Int =
        if (total <= 0) 0 else entry.offset.toLong().coerceIn(0, total.toLong() - 1).toInt()

    /** The letter for a typed character: the first key starting with it (ignoring case). */
    fun forChar(index: List<IndexEntry>, c: Char): Int =
        index.indexOfFirst { it.key.firstOrNull()?.equals(c, ignoreCase = true) == true }

    /** The letter at [y] down a scroller [height] tall (touch). */
    fun atY(index: List<IndexEntry>, y: Float, height: Float): Int {
        if (index.isEmpty() || !y.isFinite() || !height.isFinite() || height <= 0f) return -1
        return ((y / height) * index.size).toInt().coerceIn(0, index.size - 1)
    }
}

/**
 * The Library's albums (Wave 2 §3.2): in the remembered order, read where they are looked
 * at, with the A–Z index when in title order.
 *
 * "Recently played" is the user's own: the albums from their listening history on the
 * gateway ([MeRepository.shelfAlbums]), newest first, at most
 * [LibraryAlbumOrder.RECENTLY_PLAYED_MAX], read by id in that order. LMS's `recentlyplayed`
 * order is never asked for (it is the whole server's, everyone's listening).
 */
class LibraryAlbumsViewModel(
    private val library: LibraryRepository,
    order: LibraryAlbumOrder = LibraryAlbumOrder.DEFAULT,
    libraryId: Flow<String?> = flowOf(null),
    private val me: MeRepository? = null,
) : SparsePagedViewModel<Album>() {
    private val _order = MutableStateFlow(if (order in LibraryAlbumOrder.OFFERED) order else LibraryAlbumOrder.DEFAULT)
    val order: StateFlow<LibraryAlbumOrder> = _order.asStateFlow()
    private var currentLibrary: String? = null

    init {
        viewModelScope.launch {
            libraryId.distinctUntilChanged().collect {
                currentLibrary = it
                restart()
            }
        }
    }

    fun setOrder(order: LibraryAlbumOrder) {
        if (order == _order.value || order !in LibraryAlbumOrder.OFFERED) return
        _order.value = order
        restart()
    }

    private val filter get() = LibraryFilter(libraryId = currentLibrary)

    override suspend fun page(start: Int, count: Int): ApiResult<Pair<List<Album>, Int>> {
        val sort = _order.value.lmsSort ?: return recentlyPlayed(start, count)
        return when (val r = library.albums(start, count, sort, null, filter)) {
            is ApiResult.Ok -> ApiResult.Ok(r.value.items to r.value.total.toInt())
            is ApiResult.Err -> r
        }
    }

    /** The user's recently played albums, newest first (from their own history). */
    private suspend fun recentlyPlayed(start: Int, count: Int): ApiResult<Pair<List<Album>, Int>> {
        val m = me ?: return ApiResult.Ok(emptyList<Album>() to 0)
        val ids = when (val r = m.shelfAlbums(ShelfKind.RECENT, LibraryAlbumOrder.RECENTLY_PLAYED_MAX)) {
            is ApiResult.Ok -> r.value.distinct().take(LibraryAlbumOrder.RECENTLY_PLAYED_MAX)
            is ApiResult.Err -> return r
        }
        val wanted = ids.drop(start.coerceAtLeast(0)).take(count.coerceAtLeast(0))
        if (wanted.isEmpty()) return ApiResult.Ok(emptyList<Album>() to ids.size)
        return when (val r = m.albums(wanted)) {
            is ApiResult.Ok -> {
                // In history order, whatever order they came back in; ones gone from the library are left out.
                val byId = r.value.associateBy { it.id }
                ApiResult.Ok(wanted.mapNotNull { byId[it] } to ids.size)
            }
            is ApiResult.Err -> r
        }
    }

    override suspend fun index(): ApiResult<List<IndexEntry>> =
        if (_order.value == LibraryAlbumOrder.TITLE) library.albumIndex(filter) else ApiResult.Ok(emptyList())
}

/** The Library's artists, by role (as [ArtistsViewModel]), with the A–Z index. */
class LibraryArtistsViewModel(
    private val library: LibraryRepository,
    libraryId: Flow<String?> = flowOf(null),
) : SparsePagedViewModel<Artist>() {
    private val _view = MutableStateFlow(ArtistView.ALBUM_ARTISTS)
    val view: StateFlow<ArtistView> = _view.asStateFlow()
    private val _views = MutableStateFlow(ArtistView.entries.filter { it.always })
    val views: StateFlow<List<ArtistView>> = _views.asStateFlow()
    private var currentLibrary: String? = null

    init {
        viewModelScope.launch {
            libraryId.distinctUntilChanged().collect {
                currentLibrary = it
                restart()
                probeRoles(it)
            }
        }
    }

    private fun probeRoles(lib: String?) {
        _views.value = ArtistView.entries.filter { it.always || it == _view.value }
        ArtistView.entries.filter { !it.always }.forEach { v ->
            viewModelScope.launch {
                val has = (library.hasRole(v.role!!, lib) as? ApiResult.Ok)?.value == true
                if (has && lib == currentLibrary) _views.update { list -> ArtistView.entries.filter { it in list || it == v } }
            }
        }
    }

    fun setView(v: ArtistView) {
        if (v == _view.value) return
        _view.value = v
        restart()
    }

    private val filter get() = LibraryFilter(role = _view.value.role, libraryId = currentLibrary)

    override suspend fun page(start: Int, count: Int): ApiResult<Pair<List<Artist>, Int>> =
        when (val r = library.artists(start, count, null, filter)) {
            is ApiResult.Ok -> ApiResult.Ok(r.value.items to r.value.total.toInt())
            is ApiResult.Err -> r
        }

    override suspend fun index(): ApiResult<List<IndexEntry>> = library.artistIndex(filter)
}

/** The Library page's remembered chip, album order and view (per device). */
class LibraryPrefsViewModel(private val settings: SettingsStore) : ViewModel() {
    val prefs: StateFlow<LibraryPrefs?> = settings.settings.map { it.library }
        .stateIn(viewModelScope, SharingStarted.Eagerly, null)

    private fun change(f: (LibraryPrefs) -> LibraryPrefs) {
        val now = prefs.value ?: LibraryPrefs()
        viewModelScope.launch { settings.setLibraryPrefs(f(now).sanitised()) }
    }

    /** Choose a chip; choosing the one chosen goes back to the overview. */
    fun toggleChip(chip: LibraryChip) = change { it.copy(chip = if (it.chip == chip) null else chip) }

    fun setOrder(order: LibraryAlbumOrder) = change { it.copy(albumOrder = order) }

    fun setGrid(grid: Boolean) = change { it.copy(albumGrid = grid) }
}
