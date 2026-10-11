package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.SearchResults
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import kotlinx.coroutines.FlowPreview
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.debounce
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
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

/** Loads pages on demand; a new query (e.g. another sort) starts over. */
abstract class PagedViewModel<T> : ViewModel() {
    protected val _state = MutableStateFlow(PagedState<T>())
    val state: StateFlow<PagedState<T>> = _state.asStateFlow()
    private var job: Job? = null

    protected abstract suspend fun page(start: Int, count: Int): ApiResult<Pair<List<T>, Int>>

    /** Load the next page, unless one is loading or everything is here. */
    fun loadMore() {
        val s = _state.value
        if (s.loading || s.endReached) return
        _state.update { it.copy(loading = true, error = null) }
        job = viewModelScope.launch {
            when (val r = page(s.items.size, PAGE_SIZE)) {
                is ApiResult.Ok -> _state.update {
                    val (items, total) = r.value
                    // A short page means the end, whatever the count said.
                    val newTotal = if (items.size < PAGE_SIZE) it.items.size + items.size else total
                    it.copy(items = it.items + items, total = newTotal, loading = false)
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

class AlbumsViewModel(private val library: LibraryRepository) : PagedViewModel<Album>() {
    private val _sort = MutableStateFlow(AlbumSort.TITLE)
    val sort: StateFlow<AlbumSort> = _sort.asStateFlow()

    init {
        loadMore()
    }

    fun setSort(sort: AlbumSort) {
        if (sort == _sort.value) return
        _sort.value = sort
        restart()
    }

    override suspend fun page(start: Int, count: Int): ApiResult<Pair<List<Album>, Int>> =
        when (val r = library.albums(start, count, _sort.value)) {
            is ApiResult.Ok -> ApiResult.Ok(r.value.items to r.value.total.toInt())
            is ApiResult.Err -> r
        }
}

class ArtistsViewModel(private val library: LibraryRepository) : PagedViewModel<Artist>() {
    init {
        loadMore()
    }

    override suspend fun page(start: Int, count: Int): ApiResult<Pair<List<Artist>, Int>> =
        when (val r = library.artists(start, count)) {
            is ApiResult.Ok -> ApiResult.Ok(r.value.items to r.value.total.toInt())
            is ApiResult.Err -> r
        }
}

/** Something loaded once, with retry. */
sealed interface Loadable<out T> {
    data object Loading : Loadable<Nothing>

    data class Ready<T>(val value: T) : Loadable<T>

    data class Failed(val error: AppError) : Loadable<Nothing>
}

data class AlbumDetail(val album: Album?, val tracks: List<Track>)

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
            val album = library.album(albumId)
            val tracks = library.albumTracks(albumId)
            _state.value = when {
                album is ApiResult.Err -> Loadable.Failed(album.error)
                tracks is ApiResult.Err -> Loadable.Failed(tracks.error)
                else -> Loadable.Ready(
                    AlbumDetail(
                        (album as ApiResult.Ok).value,
                        (tracks as ApiResult.Ok).value,
                    ),
                )
            }
        }
    }
}

class ArtistViewModel(
    private val library: LibraryRepository,
    private val artistId: Long,
) : ViewModel() {
    private val _state = MutableStateFlow<Loadable<List<Album>>>(Loadable.Loading)
    val state: StateFlow<Loadable<List<Album>>> = _state.asStateFlow()

    init {
        load()
    }

    fun load() {
        _state.value = Loadable.Loading
        viewModelScope.launch {
            _state.value = when (val r = library.artistAlbums(artistId)) {
                is ApiResult.Ok -> Loadable.Ready(r.value)
                is ApiResult.Err -> Loadable.Failed(r.error)
            }
        }
    }
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
