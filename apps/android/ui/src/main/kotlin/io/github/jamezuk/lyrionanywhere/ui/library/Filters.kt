package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.FilterRule
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.SmartFilter
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.FilterDraft
import io.github.jamezuk.lyrionanywhere.data.FilterLimits
import io.github.jamezuk.lyrionanywhere.data.FilterPlay
import io.github.jamezuk.lyrionanywhere.data.FilterProblem
import io.github.jamezuk.lyrionanywhere.data.FiltersRepository
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** What the smart filter screens have to say (a snackbar, or a spoken announcement). */
sealed interface FilterMessage {
    data object NoPlayer : FilterMessage

    data class Playing(val name: String, val shuffle: Boolean) : FilterMessage

    /** Nothing matches the filter: nothing was sent to the player. */
    data class NoAlbums(val name: String) : FilterMessage

    data class Saved(val name: String, val added: Boolean) : FilterMessage

    data class Deleted(val name: String) : FilterMessage

    data class Failed(val error: AppError) : FilterMessage
}

/**
 * Play or shuffle [filter] on [player] (Wave 2 §5.6): its first albums, up to 50, through the
 * album play path; shuffle sets track shuffle first. Shared by Library, the filter's page and
 * Home.
 */
internal suspend fun playFilterOn(
    filters: FiltersRepository,
    players: PlayersRepository,
    player: String?,
    id: Long,
    name: String,
    shuffle: Boolean,
): FilterMessage {
    if (player == null) return FilterMessage.NoPlayer
    val r = FilterPlay.play(
        filters,
        id,
        shuffle,
        shuffleOn = { players.setShuffle(player, ShuffleMode.SONGS) },
        enqueue = { album, placement -> players.enqueue(player, Selection.Album(album), placement) },
    )
    return when (r) {
        is ApiResult.Ok -> if (r.value == 0) FilterMessage.NoAlbums(name) else FilterMessage.Playing(name, shuffle)
        is ApiResult.Err -> FilterMessage.Failed(r.error)
    }
}

/** The *Filters* chip in Library: the user's smart filters. */
data class FiltersUi(
    val filters: Loadable<List<SmartFilter>> = Loadable.Loading,
    /** A play or shuffle is on its way to the player. */
    val playing: Boolean = false,
) {
    /** The most a user may keep: no new ones until one goes. */
    val full: Boolean get() = ((filters as? Loadable.Ready)?.value?.size ?: 0) >= FilterLimits.MAX_FILTERS
}

class FiltersViewModel(
    private val filters: FiltersRepository,
    private val players: PlayersRepository,
    /** The player a filter plays on. */
    private val player: () -> String?,
) : ViewModel() {
    private val _state = MutableStateFlow(FiltersUi())
    val state: StateFlow<FiltersUi> = _state.asStateFlow()
    private val _messages = Channel<FilterMessage>(Channel.BUFFERED)
    val messages: Flow<FilterMessage> = _messages.receiveAsFlow()

    init {
        load()
    }

    fun load() {
        viewModelScope.launch {
            val r = filters.filters()
            _state.update {
                it.copy(
                    filters = when (r) {
                        is ApiResult.Ok -> Loadable.Ready(r.value)
                        // A failed reload keeps the list shown.
                        is ApiResult.Err -> if (it.filters is Loadable.Ready) it.filters else Loadable.Failed(r.error)
                    },
                )
            }
            if (r is ApiResult.Err && _state.value.filters is Loadable.Ready) _messages.trySend(FilterMessage.Failed(r.error))
        }
    }

    fun play(filter: SmartFilter, shuffle: Boolean) {
        if (_state.value.playing) return
        _state.update { it.copy(playing = true) }
        viewModelScope.launch {
            try {
                _messages.trySend(playFilterOn(filters, players, player(), filter.id, filter.name, shuffle))
            } finally {
                _state.update { it.copy(playing = false) }
            }
        }
    }

    fun delete(filter: SmartFilter) {
        viewModelScope.launch {
            when (val r = filters.delete(filter.id)) {
                is ApiResult.Ok -> {
                    _state.update { s ->
                        s.copy(filters = (s.filters as? Loadable.Ready)?.let { l -> Loadable.Ready(l.value.filterNot { it.id == filter.id }) } ?: s.filters)
                    }
                    _messages.trySend(FilterMessage.Deleted(filter.name))
                }
                is ApiResult.Err -> _messages.trySend(FilterMessage.Failed(r.error))
            }
        }
    }

    /** The rule builder saved [filter] ([added]: a new one). */
    fun saved(filter: SmartFilter, added: Boolean) {
        _messages.trySend(FilterMessage.Saved(filter.name, added))
        load()
    }
}

/** The kinds of rule, one of each at most, in the order offered. */
enum class RuleKind { GENRE, DECADE, ARTIST, COMPILATION }

fun FilterRule.kind(): RuleKind = when (this) {
    is FilterRule.Genre -> RuleKind.GENRE
    is FilterRule.Decade -> RuleKind.DECADE
    is FilterRule.Artist -> RuleKind.ARTIST
    is FilterRule.Compilation -> RuleKind.COMPILATION
}

/** The orders a filter offers, as on the web: A–Z, Artist, Year, Recently added, Random. */
val FILTER_SORTS = listOf(AlbumSort.TITLE, AlbumSort.ARTIST, AlbumSort.YEAR, AlbumSort.NEWEST, AlbumSort.RANDOM)

/** The rule builder's state. */
data class FilterEditorUi(
    /** Editing a saved filter (null: a new one). */
    val editing: SmartFilter? = null,
    val name: String = "",
    /** The rules added, in the order they were added. */
    val rules: List<RuleKind> = emptyList(),
    val genres: List<Long> = emptyList(),
    val genreChoices: Loadable<List<Genre>>? = null,
    val decade: Int? = null,
    /** The chosen artist: its id, and its name once known. */
    val artistId: Long? = null,
    val artistName: String? = null,
    val artistQuery: String = "",
    val artistResults: List<Artist> = emptyList(),
    val searching: Boolean = false,
    val role: ArtistRole? = null,
    val compilation: Boolean = true,
    val sort: AlbumSort = AlbumSort.TITLE,
    /** As typed (digits only). */
    val limit: String = FilterLimits.DEFAULT_LIMIT.toString(),
    /** Why it can't be saved (checked before anything is sent); the screen words it. */
    val problem: FilterProblem? = null,
    /** Why saving failed on the server. */
    val failure: AppError? = null,
    val saving: Boolean = false,
    /** It was saved: the screen closes. */
    val saved: SmartFilter? = null,
) {
    val addable: List<RuleKind> get() = if (rules.size >= FilterLimits.MAX_RULES) emptyList() else RuleKind.entries.filterNot { it in rules }
}

/**
 * The rule builder (Wave 2 §5.6): a name, at most one rule of each kind (all must match),
 * the order and how many albums. Checked with the Rust layer's own check (the gateway's
 * rules) before it is sent; ids come only from the server's own lists.
 */
class FilterEditorViewModel(
    private val filters: FiltersRepository,
    private val library: LibraryRepository,
    initial: SmartFilter? = null,
) : ViewModel() {
    private val _state = MutableStateFlow(start(initial))
    val state: StateFlow<FilterEditorUi> = _state.asStateFlow()
    private var search: Job? = null

    init {
        if (RuleKind.GENRE in _state.value.rules) loadGenres()
        _state.value.artistId?.let { loadArtistName(it) }
    }

    private fun start(f: SmartFilter?): FilterEditorUi {
        if (f == null) return FilterEditorUi()
        var ui = FilterEditorUi(
            editing = f,
            name = f.name,
            rules = f.rules.map { it.kind() }.distinct(),
            sort = f.sort,
            limit = f.limit.toString(),
        )
        for (r in f.rules) {
            ui = when (r) {
                is FilterRule.Genre -> ui.copy(genres = r.genres.distinct().take(FilterLimits.MAX_GENRES))
                is FilterRule.Decade -> ui.copy(decade = r.decade.toInt().takeIf { it in FilterLimits.DECADES })
                is FilterRule.Artist -> ui.copy(artistId = r.artist.takeIf(FilterLimits::validId), role = r.role)
                is FilterRule.Compilation -> ui.copy(compilation = r.compilation)
            }
        }
        return ui
    }

    private fun edit(f: (FilterEditorUi) -> FilterEditorUi) = _state.update { f(it).copy(problem = null, failure = null) }

    fun setName(name: String) = edit { it.copy(name = name.take(FilterLimits.NAME_MAX)) }

    fun addRule(kind: RuleKind) {
        if (kind !in _state.value.addable) return
        edit { it.copy(rules = it.rules + kind) }
        if (kind == RuleKind.GENRE) loadGenres()
    }

    fun removeRule(kind: RuleKind) = edit { it.copy(rules = it.rules - kind) }

    /** Choose or unchoose a genre (at most 20). */
    fun toggleGenre(id: Long) = edit {
        when {
            id in it.genres -> it.copy(genres = it.genres - id)
            it.genres.size >= FilterLimits.MAX_GENRES || !FilterLimits.validId(id) -> it
            else -> it.copy(genres = it.genres + id)
        }
    }

    fun setDecade(decade: Int) = edit { if (decade in FilterLimits.DECADES) it.copy(decade = decade) else it }

    /** Find artists as the name is typed (after a short pause, from two characters). */
    fun setArtistQuery(q: String) {
        val query = q.take(SEARCH_MAX)
        edit { it.copy(artistQuery = query) }
        search?.cancel()
        val term = query.trim()
        if (term.length < 2) {
            _state.update { it.copy(artistResults = emptyList(), searching = false) }
            return
        }
        search = viewModelScope.launch {
            delay(SEARCH_PAUSE_MS)
            _state.update { it.copy(searching = true) }
            val r = library.artists(0, ARTIST_RESULTS, term)
            _state.update {
                it.copy(
                    searching = false,
                    artistResults = (r as? ApiResult.Ok)?.value?.items.orEmpty().filter { a -> FilterLimits.validId(a.id) },
                )
            }
        }
    }

    fun chooseArtist(a: Artist) {
        if (!FilterLimits.validId(a.id)) return
        search?.cancel()
        edit { it.copy(artistId = a.id, artistName = a.name, artistQuery = "", artistResults = emptyList(), searching = false) }
    }

    fun clearArtist() = edit { it.copy(artistId = null, artistName = null) }

    fun setRole(role: ArtistRole?) = edit { it.copy(role = role) }

    fun setCompilation(on: Boolean) = edit { it.copy(compilation = on) }

    fun setSort(sort: AlbumSort) = edit { it.copy(sort = sort) }

    fun setLimit(text: String) = edit { it.copy(limit = text.filter(Char::isDigit).take(3)) }

    fun retryGenres() {
        _state.update { it.copy(genreChoices = null) }
        loadGenres()
    }

    private fun loadGenres() {
        if (_state.value.genreChoices is Loadable.Ready || _state.value.genreChoices == Loadable.Loading) return
        _state.update { it.copy(genreChoices = Loadable.Loading) }
        viewModelScope.launch {
            val r = library.genres(0, GENRE_CHOICES, null)
            _state.update {
                it.copy(
                    genreChoices = when (r) {
                        is ApiResult.Ok -> Loadable.Ready(r.value.items.filter { g -> FilterLimits.validId(g.id) })
                        is ApiResult.Err -> Loadable.Failed(r.error)
                    },
                )
            }
        }
    }

    /** A saved filter keeps only the artist's id: its name, from one of its albums. */
    private fun loadArtistName(id: Long) {
        viewModelScope.launch {
            val albums = (library.artistAlbums(id) as? ApiResult.Ok)?.value.orEmpty()
            val name = (albums.firstOrNull { it.artistId == id } ?: albums.firstOrNull())?.artist
            _state.update { if (it.artistId == id && it.artistName == null) it.copy(artistName = name ?: "") else it }
        }
    }

    /** The filter as built so far. */
    fun draft(): FilterDraft {
        val s = _state.value
        val rules = s.rules.map { k ->
            when (k) {
                RuleKind.GENRE -> FilterRule.Genre(s.genres)
                RuleKind.DECADE -> FilterRule.Decade((s.decade ?: 0).toUInt())
                RuleKind.ARTIST -> FilterRule.Artist(s.artistId ?: 0, s.role)
                RuleKind.COMPILATION -> FilterRule.Compilation(s.compilation)
            }
        }
        return FilterDraft(s.name.trim(), rules, s.sort, s.limit.toIntOrNull() ?: 0)
    }

    /** What would be refused, before anything is sent; null when it can be saved. */
    fun check(): FilterProblem? {
        val s = _state.value
        if (RuleKind.ARTIST in s.rules && s.artistId == null) return FilterProblem.ChooseArtist
        if (RuleKind.DECADE in s.rules && s.decade == null) return FilterProblem.ChooseDecade
        // The Rust check's reason, as a sentence; the app's own are worded by the screen.
        return when (val p = filters.problem(draft())) {
            is FilterProblem.Refused -> FilterProblem.Refused(sentence(p.text))
            else -> p
        }
    }

    fun save() {
        val s = _state.value
        if (s.saving || s.saved != null) return
        val problem = check()
        if (problem != null) {
            _state.update { it.copy(problem = problem, failure = null) }
            return
        }
        val draft = draft()
        _state.update { it.copy(saving = true, problem = null, failure = null) }
        viewModelScope.launch {
            val editing = s.editing
            val r = if (editing != null) filters.update(editing.id, draft) else filters.create(draft)
            _state.update {
                when (r) {
                    is ApiResult.Ok -> it.copy(saving = false, saved = r.value)
                    is ApiResult.Err -> it.copy(saving = false, failure = r.error)
                }
            }
        }
    }

    companion object {
        const val SEARCH_PAUSE_MS = 250L
        const val SEARCH_MAX = 200
        const val ARTIST_RESULTS = 10
        const val GENRE_CHOICES = 500

        fun sentence(p: String): String = p.trim().replaceFirstChar { it.uppercase() }.let { if (it.endsWith('.')) it else "$it." }
    }
}

/** One filter's page: the filter (null once it is gone) and its albums. */
data class FilterAlbumsUi(
    val filter: Loadable<SmartFilter?> = Loadable.Loading,
    val albums: PagedState<Album> = PagedState(),
    val playing: Boolean = false,
    /** It was deleted from here: the page closes. */
    val deleted: Boolean = false,
)

/**
 * A smart filter's albums, run on the gateway a page at a time (ids, then their details, as
 * for shelves); play, shuffle, edit or delete it.
 */
class FilterAlbumsViewModel(
    private val filters: FiltersRepository,
    private val me: MeRepository,
    private val players: PlayersRepository,
    private val player: () -> String?,
    private val id: Long,
) : ViewModel() {
    private val _state = MutableStateFlow(FilterAlbumsUi())
    val state: StateFlow<FilterAlbumsUi> = _state.asStateFlow()
    private val _messages = Channel<FilterMessage>(Channel.BUFFERED)
    val messages: Flow<FilterMessage> = _messages.receiveAsFlow()

    /** Ids read so far (the next page starts here). */
    private var next = 0
    private var job: Job? = null

    init {
        reload()
    }

    fun reload() {
        loadFilter()
        restart()
    }

    private fun loadFilter() {
        viewModelScope.launch {
            val r = filters.filters()
            _state.update {
                it.copy(
                    filter = when (r) {
                        is ApiResult.Ok -> Loadable.Ready(r.value.firstOrNull { f -> f.id == id })
                        is ApiResult.Err -> Loadable.Failed(r.error)
                    },
                )
            }
        }
    }

    private fun restart() {
        job?.cancel()
        next = 0
        _state.update { it.copy(albums = PagedState()) }
        loadMore()
    }

    fun loadMore() {
        val s = _state.value.albums
        if (s.loading || s.endReached) return
        _state.update { it.copy(albums = it.albums.copy(loading = true, error = null)) }
        job = viewModelScope.launch {
            val page = filters.albums(id, next, PAGE)
            val ids = (page as? ApiResult.Ok)?.value
            val details = if (ids == null) null else me.albums(ids.albums.filter(FilterLimits::validId))
            _state.update { st ->
                val a = st.albums
                when {
                    page is ApiResult.Err -> st.copy(albums = a.copy(loading = false, error = page.error))
                    details is ApiResult.Err -> st.copy(albums = a.copy(loading = false, error = details.error))
                    else -> {
                        val got = ids!!.albums
                        next += got.size
                        val seen = a.items.mapTo(HashSet()) { it.id }
                        val fresh = (details as ApiResult.Ok).value.filter { seen.add(it.id) }
                        // The end: everything read, or a page with nothing (whatever the count said).
                        val total = if (got.isEmpty() || next >= ids.count.toInt() || next >= FilterLimits.LIMIT_MAX) {
                            a.items.size + fresh.size
                        } else {
                            // More to read: never "ended" while ids remain.
                            maxOf(a.items.size + fresh.size + 1, ids.count.toInt())
                        }
                        st.copy(albums = a.copy(items = a.items + fresh, total = total, loading = false))
                    }
                }
            }
        }
    }

    fun play(shuffle: Boolean) {
        val f = (_state.value.filter as? Loadable.Ready)?.value ?: return
        if (_state.value.playing) return
        _state.update { it.copy(playing = true) }
        viewModelScope.launch {
            try {
                _messages.trySend(playFilterOn(filters, players, player(), f.id, f.name, shuffle))
            } finally {
                _state.update { it.copy(playing = false) }
            }
        }
    }

    fun delete() {
        val f = (_state.value.filter as? Loadable.Ready)?.value ?: return
        viewModelScope.launch {
            when (val r = filters.delete(f.id)) {
                is ApiResult.Ok -> {
                    _state.update { it.copy(deleted = true) }
                    _messages.trySend(FilterMessage.Deleted(f.name))
                }
                is ApiResult.Err -> _messages.trySend(FilterMessage.Failed(r.error))
            }
        }
    }

    /** The rule builder saved changes: show the filter as saved, and run it again. */
    fun saved(filter: SmartFilter) {
        if (filter.id != id) return
        _state.update { it.copy(filter = Loadable.Ready(filter)) }
        _messages.trySend(FilterMessage.Saved(filter.name, added = false))
        restart()
    }

    companion object {
        const val PAGE = 50
    }
}
