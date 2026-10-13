package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.FavouritePage
import io.github.jamezuk.lyrionanywhere.core.FolderEntry
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.PinnedMenu
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationBatch
import io.github.jamezuk.lyrionanywhere.core.StationSpec
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** A list loaded a page at a time. */
data class PagedState<T>(
    val items: List<T> = emptyList(),
    val total: Int? = null,
    val loading: Boolean = false,
    val error: AppError? = null,
) {
    val endReached: Boolean get() = total != null && items.size >= total

    /** Nothing loaded yet, and the first page failed. */
    val failedEmpty: Boolean get() = items.isEmpty() && error != null
}

/**
 * Loads pages on demand; a new query (another sort, library or filter) starts over. Items
 * LMS lists twice (a rescan meanwhile, a random order) are shown once, and a page with
 * nothing new ends the list, so a confused server can't keep it asking.
 */
abstract class PagedViewModel<T> : ViewModel() {
    protected val _state = MutableStateFlow(PagedState<T>())
    val state: StateFlow<PagedState<T>> = _state.asStateFlow()
    private var job: Job? = null

    protected abstract suspend fun page(start: Int, count: Int): ApiResult<Pair<List<T>, Int>>

    /** A stable key for an item, to drop repeats (null: keep everything). */
    protected open fun keyOf(item: T): Any? = null

    /** Items a page asks for. */
    protected open val pageSize: Int = PAGE_SIZE

    /** Load the next page, unless one is loading or everything is here. */
    fun loadMore() {
        val s = _state.value
        if (s.loading || s.endReached) return
        _state.update { it.copy(loading = true, error = null) }
        job = viewModelScope.launch {
            when (val r = page(s.items.size, pageSize)) {
                is ApiResult.Ok -> _state.update {
                    val (items, total) = r.value
                    val seen = it.items.mapNotNullTo(HashSet()) { x -> keyOf(x) }
                    val fresh = items.filter { x -> keyOf(x)?.let(seen::add) ?: true }
                    val all = it.items + fresh
                    // A short page, or one with nothing new, means the end, whatever the count said.
                    val newTotal = if (items.size < pageSize || fresh.isEmpty()) all.size else maxOf(total, all.size)
                    it.copy(items = all, total = newTotal, loading = false)
                }
                is ApiResult.Err -> _state.update { it.copy(loading = false, error = r.error) }
            }
        }
    }

    protected fun restart() {
        job?.cancel()
        _state.value = PagedState()
        loadMore()
    }

    fun retry() = loadMore()

    companion object {
        const val PAGE_SIZE = 100
    }
}

private fun <T, R> ApiResult<T>.page(items: (T) -> List<R>, total: (T) -> UInt): ApiResult<Pair<List<R>, Int>> =
    when (this) {
        is ApiResult.Ok -> ApiResult.Ok(items(value) to total(value).toInt())
        is ApiResult.Err -> this
    }

/**
 * An album list: one filtered `albums` query, paged as it scrolls, in the chosen virtual
 * library. With [batch], a single batch of that many instead (random albums: paging a
 * random order would only repeat albums).
 */
class AlbumsViewModel(
    private val library: LibraryRepository,
    private val filter: LibraryFilter = LibraryFilter(),
    sort: AlbumSort = AlbumSort.TITLE,
    libraryId: Flow<String?> = flowOf(null),
    private val batch: Int? = null,
) : PagedViewModel<Album>() {
    private val _sort = MutableStateFlow(sort)
    val sort: StateFlow<AlbumSort> = _sort.asStateFlow()
    private var currentLibrary: String? = null

    override val pageSize: Int get() = batch ?: PAGE_SIZE

    init {
        viewModelScope.launch {
            libraryId.distinctUntilChanged().collect {
                currentLibrary = it
                restart()
            }
        }
    }

    fun setSort(sort: AlbumSort) {
        if (sort == _sort.value) return
        _sort.value = sort
        restart()
    }

    /** Load again (e.g. "Shuffle again" for random albums). */
    fun reload() = restart()

    override fun keyOf(item: Album): Any = item.id

    override suspend fun page(start: Int, count: Int): ApiResult<Pair<List<Album>, Int>> {
        if (batch != null && start > 0) return ApiResult.Ok(emptyList<Album>() to start)
        return library.albums(start, count, _sort.value, null, filter.copy(libraryId = currentLibrary))
            .page({ it.items }, { if (batch != null) it.items.size.toUInt() else it.total })
    }
}

/** Ways to list artists; album artists first, as in LMS. */
enum class ArtistView(val role: ArtistRole?) {
    ALBUM_ARTISTS(ArtistRole.ALBUM_ARTIST),
    ALL(null),
    COMPOSERS(ArtistRole.COMPOSER),
    CONDUCTORS(ArtistRole.CONDUCTOR),
    BANDS(ArtistRole.BAND),
    ;

    /** Offered whatever the library has. */
    val always: Boolean get() = this == ALBUM_ARTISTS || this == ALL
}

/** Artists, by role (the optional roles offered only when the library has them). */
class ArtistsViewModel(
    private val library: LibraryRepository,
    private val filter: LibraryFilter = LibraryFilter(),
    libraryId: Flow<String?> = flowOf(null),
    view: ArtistView = if (filter.genreId != null) ArtistView.ALL else ArtistView.ALBUM_ARTISTS,
    /** Offer the role views (not inside a genre). */
    private val roles: Boolean = filter.genreId == null,
) : PagedViewModel<Artist>() {
    private val _view = MutableStateFlow(view)
    val view: StateFlow<ArtistView> = _view.asStateFlow()
    private val _views = MutableStateFlow(if (roles) ArtistView.entries.filter { it.always } else emptyList())

    /** The views to offer as chips. */
    val views: StateFlow<List<ArtistView>> = _views.asStateFlow()
    private var currentLibrary: String? = null

    init {
        viewModelScope.launch {
            libraryId.distinctUntilChanged().collect {
                currentLibrary = it
                restart()
                if (roles) probeRoles(it)
            }
        }
    }

    private fun probeRoles(lib: String?) {
        _views.value = ArtistView.entries.filter { it.always || it == _view.value }
        ArtistView.entries.filter { !it.always }.forEach { v ->
            viewModelScope.launch {
                val has = (library.hasRole(v.role!!, lib) as? ApiResult.Ok)?.value == true
                if (has && lib == currentLibrary) {
                    _views.update { list -> ArtistView.entries.filter { it in list || it == v } }
                }
            }
        }
    }

    fun setView(v: ArtistView) {
        if (v == _view.value) return
        _view.value = v
        restart()
    }

    override fun keyOf(item: Artist): Any = item.id

    override val pageSize: Int get() = 200

    override suspend fun page(start: Int, count: Int) =
        library.artists(start, count, null, filter.copy(role = _view.value.role, libraryId = currentLibrary))
            .page({ it.items }, { it.total })
}

/** Genres, with a search box (after a short pause). */
@OptIn(FlowPreview::class)
class GenresViewModel(
    private val library: LibraryRepository,
    libraryId: Flow<String?> = flowOf(null),
) : PagedViewModel<Genre>() {
    private val _query = MutableStateFlow("")
    val query: StateFlow<String> = _query.asStateFlow()
    private var currentLibrary: String? = null

    init {
        viewModelScope.launch {
            libraryId.distinctUntilChanged().collect {
                currentLibrary = it
                restart()
            }
        }
        viewModelScope.launch {
            _query.map { it.trim() }.distinctUntilChanged().drop(1).debounce(SearchViewModel.DEBOUNCE_MS).collect { restart() }
        }
    }

    fun onQueryChange(q: String) {
        _query.value = q.take(200)
    }

    override fun keyOf(item: Genre): Any = item.id

    override val pageSize: Int get() = 200

    override suspend fun page(start: Int, count: Int) =
        library.genres(start, count, currentLibrary, _query.value.trim().ifEmpty { null })
            .page({ it.items }, { it.total })
}

/** Years that have albums. */
class YearsViewModel(
    private val library: LibraryRepository,
    libraryId: Flow<String?> = flowOf(null),
) : PagedViewModel<UInt>() {
    private var currentLibrary: String? = null

    init {
        viewModelScope.launch {
            libraryId.distinctUntilChanged().collect {
                currentLibrary = it
                restart()
            }
        }
    }

    override fun keyOf(item: UInt): Any = item

    override val pageSize: Int get() = 200

    override suspend fun page(start: Int, count: Int) =
        library.years(start, count, currentLibrary).page({ it.items }, { it.total })
}

/** One folder of the music folder (the top level without an id). */
class FolderViewModel(
    private val library: LibraryRepository,
    private val folderId: Long?,
) : PagedViewModel<FolderEntry>() {
    init {
        loadMore()
    }

    override fun keyOf(item: FolderEntry): Any = item.id

    override val pageSize: Int get() = 200

    override suspend fun page(start: Int, count: Int) =
        library.musicFolder(start, count, folderId).page({ it.items }, { it.total })
}

/** Saved playlists. */
class PlaylistsViewModel(private val library: LibraryRepository) : PagedViewModel<SavedPlaylist>() {
    init {
        loadMore()
    }

    override fun keyOf(item: SavedPlaylist): Any = item.id

    override suspend fun page(start: Int, count: Int) =
        library.playlists(start, count).page({ it.items }, { it.total })
}

/** Something loaded once, with retry. */
sealed interface Loadable<out T> {
    data object Loading : Loadable<Nothing>

    data class Ready<T>(val value: T) : Loadable<T>

    data class Failed(val error: AppError) : Loadable<Nothing>
}

/** Loads one thing, with retry. */
open class LoadViewModel<T>(private val load: suspend () -> ApiResult<T>) : ViewModel() {
    private val _state = MutableStateFlow<Loadable<T>>(Loadable.Loading)
    val state: StateFlow<Loadable<T>> = _state.asStateFlow()

    init {
        reload()
    }

    fun reload() {
        _state.value = Loadable.Loading
        viewModelScope.launch {
            _state.value = when (val r = load()) {
                is ApiResult.Ok -> Loadable.Ready(r.value)
                is ApiResult.Err -> Loadable.Failed(r.error)
            }
        }
    }
}

/** A saved playlist's tracks. */
class PlaylistViewModel(library: LibraryRepository, playlistId: Long) :
    LoadViewModel<List<Track>>({ library.playlistTracks(playlistId) })

/** Favourites, or a folder of them. */
class FavouritesViewModel(library: LibraryRepository, itemId: String?) :
    LoadViewModel<FavouritePage>({ library.favourites(0, 500, itemId) })

/** A genre's name. */
class GenreNameViewModel(library: LibraryRepository, genreId: Long) :
    LoadViewModel<String?>({
        when (val r = library.genre(genreId)) {
            is ApiResult.Ok -> ApiResult.Ok(r.value?.name)
            is ApiResult.Err -> r
        }
    })

/** One disc of an album (all tracks, for a single-disc album). */
data class Disc(val number: Int?, val tracks: List<Track>)

/** The album page: the album, its tracks by disc, and its details. */
data class AlbumDetail(val album: Album?, val tracks: List<Track>) {
    /** Discs, in order; one unnumbered disc unless the album has several. */
    val discs: List<Disc>
        get() {
            val numbers = tracks.mapNotNull { it.discNumber?.toInt() }.toSet()
            val many = numbers.size > 1 || tracks.any { (it.discCount?.toInt() ?: 1) > 1 } && numbers.isNotEmpty()
            if (!many) return listOf(Disc(null, tracks))
            return tracks.groupBy { it.discNumber?.toInt() ?: 1 }.toSortedMap().map { (n, t) -> Disc(n, t) }
        }

    val genres: List<String> get() = tracks.mapNotNull { it.genre }.distinct()

    val formats: List<String> get() = tracks.mapNotNull { it.format }.distinct()

    val totalSecs: Double get() = tracks.sumOf { it.durationSecs ?: 0.0 }

    /** More than one artist on the tracks: show each track's artist. */
    val variousArtists: Boolean get() = tracks.mapNotNull { it.artist }.distinct().size > 1

    val year: Int? get() = album?.year ?: tracks.firstNotNullOfOrNull { it.year }
}

class AlbumViewModel(
    private val library: LibraryRepository,
    private val albumId: Long,
) : ViewModel() {
    private val _state = MutableStateFlow<Loadable<AlbumDetail>>(Loadable.Loading)
    val state: StateFlow<Loadable<AlbumDetail>> = _state.asStateFlow()

    init {
        load()
    }

    fun load() {
        _state.value = Loadable.Loading
        viewModelScope.launch {
            val album = async { library.album(albumId) }
            val tracks = async { library.albumTracks(albumId) }
            val a = album.await()
            val t = tracks.await()
            _state.value = when {
                a is ApiResult.Err -> Loadable.Failed(a.error)
                t is ApiResult.Err -> Loadable.Failed(t.error)
                else -> Loadable.Ready(AlbumDetail((a as ApiResult.Ok).value, (t as ApiResult.Ok).value))
            }
        }
    }
}

/** An artist page's sections. */
enum class ArtistSection { ALBUMS, APPEARS_ON, COMPILATIONS }

data class ArtistDetail(
    /** Every album, oldest first. */
    val albums: List<Album>,
    /** The albums grouped by the artist's role on them, in a fixed order; empty groups left out. */
    val sections: List<Pair<ArtistSection, List<Album>>>,
) {
    /** The cover for the page's header: the newest album's. */
    val headerArt: String? get() = albums.lastOrNull { it.artwork != null }?.artwork

    companion object {
        /**
         * Albums where the artist is the album artist come first; compilations and albums
         * they only appear on follow. Without role information everything is "Albums".
         */
        fun group(all: List<Album>, asAlbumArtist: List<Album>?): ArtistDetail {
            val main = asAlbumArtist?.map { it.id }?.toSet()
            if (main.isNullOrEmpty()) {
                return ArtistDetail(all, listOf(ArtistSection.ALBUMS to all).filter { it.second.isNotEmpty() })
            }
            val albums = all.filter { it.id in main && !it.compilation }
            val compilations = all.filter { it.compilation }
            val appears = all.filter { it.id !in main && !it.compilation }
            val sections = listOf(
                ArtistSection.ALBUMS to albums,
                ArtistSection.APPEARS_ON to appears,
                ArtistSection.COMPILATIONS to compilations,
            ).filter { it.second.isNotEmpty() }
            return ArtistDetail(all, sections)
        }
    }
}

class ArtistViewModel(
    private val library: LibraryRepository,
    private val artistId: Long,
) : ViewModel() {
    private val _state = MutableStateFlow<Loadable<ArtistDetail>>(Loadable.Loading)
    val state: StateFlow<Loadable<ArtistDetail>> = _state.asStateFlow()

    init {
        load()
    }

    fun load() {
        _state.value = Loadable.Loading
        viewModelScope.launch {
            val all = async { library.artistAlbums(artistId) }
            val main = async { library.artistAlbums(artistId, ArtistRole.ALBUM_ARTIST) }
            _state.value = when (val a = all.await()) {
                is ApiResult.Err -> Loadable.Failed(a.error)
                // Role information is a nicety: without it, one list.
                is ApiResult.Ok -> Loadable.Ready(ArtistDetail.group(a.value, (main.await() as? ApiResult.Ok)?.value))
            }
        }
    }
}

/** Home's shelves (D20: the library first; docs/plans/wave1-design.md §7). */
data class HomeFeed(
    /** New Music (recently added albums). */
    val recent: Loadable<List<Album>> = Loadable.Loading,
    val random: Loadable<List<Album>> = Loadable.Loading,
    val favourites: Loadable<List<Favourite>> = Loadable.Loading,
    val playlists: Loadable<List<SavedPlaylist>> = Loadable.Loading,
    /** The user's layout (from the gateway); null: the fixed carousels above. */
    val layout: HomeLayout? = null,
    /**
     * [layout] is the one saved on the server (it loaded). Only then can Home be edited: an
     * edit started from the default shown after a failed load would save over the user's
     * real layout.
     */
    val layoutLoaded: Boolean = false,
    /** Why the layout didn't load (the default is shown meanwhile). */
    val layoutError: AppError? = null,
    /** Albums for the history shelves (recently played, most played, on this day). */
    val history: Map<ShelfKind, Loadable<List<Album>>> = emptyMap(),
    val stations: Loadable<List<Station>> = Loadable.Loading,
    val menus: List<PinnedMenu> = emptyList(),
    /** Albums of each genre shelf, by genre id. */
    val genres: Map<Long, Loadable<List<Album>>> = emptyMap(),
    /** Genre names, for genre shelves and pickers. */
    val genreNames: Map<Long, String> = emptyMap(),
    /** The layout being edited (edit mode), or null. */
    val draft: HomeLayout? = null,
    val saving: Boolean = false,
    /** The station kept going, if any. */
    val activeStation: Long? = null,
)

/** What Home has to say (a snackbar). */
sealed interface HomeMessage {
    data class Failed(val error: AppError) : HomeMessage

    data class StationBroken(val reason: String) : HomeMessage

    data object NoPlayer : HomeMessage

    data class StationStarted(val name: String) : HomeMessage
}

class HomeFeedViewModel(
    private val library: LibraryRepository,
    libraryId: Flow<String?> = flowOf(null),
    /** The user's layout, history shelves and stations (none: the fixed carousels). */
    private val me: MeRepository? = null,
    /** The player stations start on. */
    private val player: () -> String? = { null },
) : ViewModel() {
    private val _state = MutableStateFlow(HomeFeed())
    val state: StateFlow<HomeFeed> = _state.asStateFlow()
    private val _messages = Channel<HomeMessage>(Channel.BUFFERED)
    val messages: Flow<HomeMessage> = _messages.receiveAsFlow()
    private var currentLibrary: String? = null

    init {
        viewModelScope.launch {
            libraryId.distinctUntilChanged().collect {
                currentLibrary = it
                refresh()
            }
        }
        me?.let { m ->
            viewModelScope.launch { m.activeStation.collect { id -> _state.update { it.copy(activeStation = id) } } }
        }
    }

    private fun <T> ApiResult<T>.loadable(): Loadable<T> = when (this) {
        is ApiResult.Ok -> Loadable.Ready(value)
        is ApiResult.Err -> Loadable.Failed(error)
    }

    fun refresh() {
        val lib = LibraryFilter(libraryId = currentLibrary)
        viewModelScope.launch {
            val r = library.albums(0, CAROUSEL, AlbumSort.NEWEST, null, lib)
            _state.update { it.copy(recent = r.loadable().mapReady { p -> p.items }) }
        }
        viewModelScope.launch {
            val r = library.albums(0, CAROUSEL, AlbumSort.RANDOM, null, lib)
            _state.update { it.copy(random = r.loadable().mapReady { p -> p.items }) }
        }
        viewModelScope.launch {
            val r = library.favourites(0, CAROUSEL)
            _state.update { it.copy(favourites = r.loadable().mapReady { p -> p.items }) }
        }
        viewModelScope.launch {
            val r = library.playlists(0, CAROUSEL)
            _state.update { it.copy(playlists = r.loadable().mapReady { p -> p.items }) }
        }
        val m = me ?: return
        viewModelScope.launch {
            when (val r = m.home()) {
                is ApiResult.Ok -> _state.update { it.copy(layout = r.value, layoutLoaded = true, layoutError = null) }
                is ApiResult.Err -> _state.update {
                    // A layout loaded before is still the user's own; otherwise show the
                    // default, which can't be edited (it isn't what the server holds).
                    if (it.layoutLoaded) {
                        it.copy(layoutError = r.error)
                    } else {
                        it.copy(layout = HomeLayouts.DEFAULT, layoutError = r.error)
                    }
                }
            }
            loadShelves(m, _state.value.layout ?: HomeLayouts.DEFAULT)
        }
        viewModelScope.launch { loadStations(m) }
    }

    private suspend fun loadStations(m: MeRepository) {
        val st = m.stations()
        val menus = (m.menus() as? ApiResult.Ok)?.value.orEmpty()
        _state.update { it.copy(stations = st.loadable(), menus = menus) }
    }

    /** Load what the shelves in [layout] show (only those not loaded yet, unless [all]). */
    private fun loadShelves(m: MeRepository, layout: HomeLayout, all: Boolean = true) {
        val lib = LibraryFilter(libraryId = currentLibrary)
        for (shelf in layout.shelves) {
            when (shelf.kind) {
                ShelfKind.RECENT, ShelfKind.MOST, ShelfKind.ON_THIS_DAY -> {
                    if (!all && _state.value.history[shelf.kind] is Loadable.Ready) continue
                    viewModelScope.launch {
                        val r = when (val ids = m.shelfAlbums(shelf.kind, CAROUSEL)) {
                            is ApiResult.Ok -> m.albums(ids.value)
                            is ApiResult.Err -> ids
                        }
                        _state.update { it.copy(history = it.history + (shelf.kind to r.loadable())) }
                    }
                }
                ShelfKind.GENRE -> {
                    val id = shelf.id ?: continue
                    if (!all && _state.value.genres[id] is Loadable.Ready) continue
                    viewModelScope.launch {
                        val r = library.albums(0, CAROUSEL, AlbumSort.NEWEST, null, lib.copy(genreId = id))
                        val name = (library.genre(id) as? ApiResult.Ok)?.value?.name
                        _state.update {
                            it.copy(
                                genres = it.genres + (id to r.loadable().mapReady { p -> p.items }),
                                genreNames = if (name != null) it.genreNames + (id to name) else it.genreNames,
                            )
                        }
                    }
                }
                else -> {}
            }
        }
    }

    /** Shuffle the random albums again. */
    fun reshuffle() {
        viewModelScope.launch {
            val r = library.albums(0, CAROUSEL, AlbumSort.RANDOM, null, LibraryFilter(libraryId = currentLibrary))
            _state.update { it.copy(random = r.loadable().mapReady { p -> p.items }) }
        }
    }

    // --- stations ----------------------------------------------------------------------------

    fun startStation(station: Station) {
        val m = me ?: return
        val p = player()
        if (p == null) {
            _messages.trySend(HomeMessage.NoPlayer)
            return
        }
        viewModelScope.launch {
            when (val r = m.startStation(station.id, p)) {
                is ApiResult.Ok -> when (val b = r.value) {
                    is StationBatch.Broken -> {
                        _messages.trySend(HomeMessage.StationBroken(b.reason))
                        loadStations(m)
                    }
                    else -> _messages.trySend(HomeMessage.StationStarted(station.name))
                }
                is ApiResult.Err -> _messages.trySend(HomeMessage.Failed(r.error))
            }
        }
    }

    fun createStation(name: String, spec: StationSpec) {
        val m = me ?: return
        viewModelScope.launch {
            when (val r = m.createStation(name, spec)) {
                is ApiResult.Ok -> loadStations(m)
                is ApiResult.Err -> _messages.trySend(HomeMessage.Failed(r.error))
            }
        }
    }

    fun deleteStation(station: Station) {
        val m = me ?: return
        viewModelScope.launch {
            when (val r = m.deleteStation(station.id)) {
                is ApiResult.Ok -> loadStations(m)
                is ApiResult.Err -> _messages.trySend(HomeMessage.Failed(r.error))
            }
        }
    }

    /** Genres to choose from (new stations, genre shelves). */
    suspend fun genreChoices(): List<Genre> =
        (library.genres(0, 500, currentLibrary) as? ApiResult.Ok)?.value?.items.orEmpty()

    // --- edit mode ---------------------------------------------------------------------------

    /** Start editing, from the layout the server holds (not offered until it has loaded). */
    fun edit() = _state.update { s ->
        val layout = s.layout
        if (!s.layoutLoaded || layout == null) s else s.copy(draft = layout)
    }

    /** The draft differs from the saved layout (leaving would lose changes). */
    fun draftChanged(): Boolean = _state.value.let { s -> s.draft != null && s.draft != s.layout }

    fun cancelEdit() = _state.update { it.copy(draft = null) }

    private fun change(f: (HomeLayout) -> HomeLayout?) = _state.update { s ->
        val d = s.draft ?: return@update s
        f(d)?.let { s.copy(draft = it) } ?: s
    }

    fun move(from: Int, to: Int) = change { HomeLayouts.move(it, from, to) }

    fun moveUp(index: Int) = move(index, index - 1)

    fun moveDown(index: Int) = move(index, index + 1)

    fun setHidden(index: Int, hidden: Boolean) = change { HomeLayouts.setHidden(it, index, hidden) }

    fun remove(index: Int) = change { HomeLayouts.remove(it, index) }

    fun add(shelf: HomeShelf) = change { HomeLayouts.add(it, shelf) }

    fun resetDraft() = _state.update { if (it.draft == null) it else it.copy(draft = HomeLayouts.DEFAULT) }

    /** Save the draft; on failure the edit stays open and says why. */
    fun save() {
        val m = me ?: return
        val draft = _state.value.draft ?: return
        // Never over a layout that didn't load (see [HomeFeed.layoutLoaded]).
        if (!_state.value.layoutLoaded || _state.value.saving) return
        _state.update { it.copy(saving = true) }
        viewModelScope.launch {
            val r = if (draft.isDefault) m.resetHome() else m.saveHome(draft)
            when (r) {
                is ApiResult.Ok -> {
                    _state.update { it.copy(layout = r.value, draft = null, saving = false) }
                    loadShelves(m, r.value, all = false)
                }
                is ApiResult.Err -> {
                    _state.update { it.copy(saving = false) }
                    _messages.trySend(HomeMessage.Failed(r.error))
                }
            }
        }
    }

    companion object {
        const val CAROUSEL = 20
    }
}

fun <T, R> Loadable<T>.mapReady(f: (T) -> R): Loadable<R> = when (this) {
    Loadable.Loading -> Loadable.Loading
    is Loadable.Failed -> this
    is Loadable.Ready -> Loadable.Ready(f(value))
}

data class SearchUiState(
    val query: String = "",
    val results: SearchResults? = null,
    val loading: Boolean = false,
    val error: AppError? = null,
)

/** Search as you type (after a short pause). */
@OptIn(FlowPreview::class)
class SearchViewModel(private val library: LibraryRepository) : ViewModel() {
    private val _state = MutableStateFlow(SearchUiState())
    val state: StateFlow<SearchUiState> = _state.asStateFlow()
    private var job: Job? = null

    init {
        viewModelScope.launch {
            _state.map { it.query.trim() }
                .distinctUntilChanged()
                .debounce(DEBOUNCE_MS)
                .collect(::run)
        }
    }

    fun onQueryChange(q: String) = _state.update { it.copy(query = q.take(200)) }

    fun retry() = run(_state.value.query.trim())

    private fun run(term: String) {
        job?.cancel()
        if (term.length < MIN_LENGTH) {
            _state.update { it.copy(results = null, loading = false, error = null) }
            return
        }
        _state.update { it.copy(loading = true, error = null) }
        job = viewModelScope.launch {
            when (val r = library.search(term, RESULTS)) {
                is ApiResult.Ok -> _state.update { it.copy(results = r.value, loading = false) }
                is ApiResult.Err -> _state.update { it.copy(error = r.error, loading = false) }
            }
        }
    }

    companion object {
        const val DEBOUNCE_MS = 300L
        const val MIN_LENGTH = 2
        const val RESULTS = 20
    }
}
