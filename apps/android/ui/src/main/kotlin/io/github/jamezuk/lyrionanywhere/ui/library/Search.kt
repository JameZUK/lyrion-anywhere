package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.TravelExplore
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.SearchKind
import io.github.jamezuk.lyrionanywhere.core.SearchableApp
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.menus.More
import io.github.jamezuk.lyrionanywhere.ui.menus.MoreRequest

/** What search can do. */
data class SearchActions(
    val onQuery: (String) -> Unit,
    /** The search was submitted (keep it as a recent search). */
    val onSubmit: () -> Unit,
    val onType: (SearchType) -> Unit,
    val onRetry: () -> Unit,
    val onLoadMore: () -> Unit,
    val onRecent: (String) -> Unit,
    val onRemoveRecent: (String) -> Unit,
    val onClearRecents: () -> Unit,
    val onArtist: (Artist) -> Unit,
    val onAlbum: (Album) -> Unit,
    val onPlaylist: (SavedPlaylist) -> Unit,
    /** Play (now, next or at the end) straight from a result. */
    val onPlay: (Selection, Placement) -> Unit,
    val onMore: (MoreRequest) -> Unit,
    /** Search an app for the same text (its own results, in the menu browser). */
    val onSearchOn: (SearchableApp) -> Unit = {},
)

@Composable
private fun typeLabel(t: SearchType): String = stringResource(
    when (t) {
        SearchType.ALL -> R.string.search_all
        SearchType.ARTISTS -> R.string.section_artists
        SearchType.ALBUMS -> R.string.section_albums
        SearchType.TRACKS -> R.string.section_tracks
        SearchType.PLAYLISTS -> R.string.section_playlists
    },
)

/**
 * Search (Wave 2 §3.3): the field; recent searches (this device only) while it is empty;
 * then results grouped with a Top result, or one type alone, and below them "Search on …"
 * rows handing the text to each app that offers a search. A track plays from its row and
 * an album has a play button; rows swipe right to play next and left to add to the queue,
 * and their menus offer the same.
 */
@Composable
fun SearchScreen(state: SearchUiState, typed: PagedState<SearchHit>, actions: SearchActions, modifier: Modifier = Modifier) {
    val focusManager = LocalFocusManager.current
    Column(modifier.fillMaxSize()) {
        OutlinedTextField(
            value = state.query,
            onValueChange = actions.onQuery,
            label = { Text(stringResource(R.string.search_field)) },
            singleLine = true,
            leadingIcon = { Icon(Icons.Outlined.Search, contentDescription = null) },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            // Results come as you type: Search keeps the term and closes the keyboard.
            keyboardActions = KeyboardActions(onSearch = {
                actions.onSubmit()
                focusManager.clearFocus()
            }),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp).testTag("search-field"),
        )
        if (!state.searched) {
            RecentSearches(state.recents, actions)
            return@Column
        }
        TypeChips(state.type, actions.onType)
        if (state.type == SearchType.ALL) AllResults(state, actions) else TypedResults(state, typed, actions)
    }
}

@Composable
private fun RecentSearches(recents: List<String>, actions: SearchActions) {
    if (recents.isEmpty()) {
        EmptyState(stringResource(R.string.search_hint))
        return
    }
    LazyColumn(Modifier.fillMaxSize().testTag("search-recents")) {
        item {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Box(Modifier.weight(1f)) { SectionHeader(stringResource(R.string.search_recent)) }
                TextButton(onClick = actions.onClearRecents, modifier = Modifier.padding(end = 8.dp)) {
                    Text(stringResource(R.string.search_recent_clear))
                }
            }
        }
        items(recents, key = { it }) { term ->
            ListItem(
                leadingContent = { Icon(Icons.Outlined.History, contentDescription = null) },
                headlineContent = { Text(term, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                trailingContent = {
                    IconButton(onClick = { actions.onRemoveRecent(term) }) {
                        Icon(Icons.Outlined.Close, stringResource(R.string.search_recent_remove, term))
                    }
                },
                modifier = Modifier.clickable(onClickLabel = stringResource(R.string.search_recent_open)) { actions.onRecent(term) },
            )
        }
    }
}

@Composable
private fun TypeChips(type: SearchType, onType: (SearchType) -> Unit) {
    val group = stringResource(R.string.search_filter)
    Row(
        Modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 12.dp, vertical = 4.dp)
            .semantics { contentDescription = group }
            .testTag("search-types"),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        SearchType.entries.forEach { t ->
            FilterChip(selected = t == type, onClick = { onType(t) }, label = { Text(typeLabel(t)) })
        }
    }
}

@Composable
private fun AllResults(state: SearchUiState, actions: SearchActions) {
    val r = state.results
    when {
        state.error != null -> ErrorState(state.error, actions.onRetry)
        r == null -> LoadingState()
        state.empty && state.searchOn.isEmpty() -> EmptyState(stringResource(R.string.search_nothing, state.query.trim()))
        else -> LazyColumn(Modifier.fillMaxSize().testTag("search-all")) {
            if (state.empty) {
                item(key = "nothing") {
                    Text(
                        stringResource(R.string.search_nothing, state.query.trim()),
                        style = MaterialTheme.typography.bodyLarge,
                        modifier = Modifier.padding(16.dp),
                    )
                }
            }
            r.top?.let { top ->
                item(key = "top") { TopResultCard(top.kind, top.index.toInt(), state, actions) }
            }
            section(
                SearchType.ARTISTS,
                r.artists.take(SearchViewModel.SECTION),
                r.artistsTotal.toInt(),
                actions,
                key = { "ar${it.id}" },
            ) { a -> ArtistResultRow(a, actions) }
            section(
                SearchType.ALBUMS,
                r.albums.take(SearchViewModel.SECTION),
                r.albumsTotal.toInt(),
                actions,
                key = { "al${it.id}" },
            ) { a -> AlbumResultRow(a, actions) }
            section(
                SearchType.TRACKS,
                r.tracks.take(SearchViewModel.SECTION),
                r.tracksTotal.toInt(),
                actions,
                key = { "tr${it.id}" },
            ) { t -> TrackResultRow(t, actions) }
            section(
                SearchType.PLAYLISTS,
                state.playlists.take(SearchViewModel.SECTION),
                state.playlists.size,
                actions,
                key = { "pl${it.id}" },
            ) { p -> PlaylistResultRow(p, actions) }
            searchOn(state.searchOn, state.query.trim(), actions)
        }
    }
}

/** "Search on …": the same text in each app that offers a search, below the library's results. */
private fun LazyListScope.searchOn(apps: List<SearchableApp>, query: String, actions: SearchActions) {
    if (apps.isEmpty() || query.isEmpty()) return
    item(key = "h-search-on") { SectionHeader(stringResource(R.string.search_on)) }
    items(apps, key = { "on-${it.cmd}" }) { app -> SearchOnRow(app, query, actions) }
}

@Composable
private fun SearchOnRow(app: SearchableApp, query: String, actions: SearchActions) {
    ListItem(
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.TravelExplore, contentDescription = null) },
        headlineContent = { Text(stringResource(R.string.search_on_row, app.name, query), maxLines = 2, overflow = TextOverflow.Ellipsis) },
        modifier = Modifier
            .clickable(onClickLabel = stringResource(R.string.search_on_open)) { actions.onSearchOn(app) }
            .testTag("search-on-${app.cmd}"),
    )
}

/** A type's few rows under "All", with "See all" when there are more. */
private fun <T> LazyListScope.section(
    type: SearchType,
    rows: List<T>,
    total: Int,
    actions: SearchActions,
    key: (T) -> Any,
    row: @Composable (T) -> Unit,
) {
    if (rows.isEmpty()) return
    item(key = "h-$type") {
        val title = typeLabel(type)
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
            Box(Modifier.weight(1f)) { SectionHeader(title) }
            if (total > rows.size) {
                val seeAll = stringResource(R.string.see_all_named, title)
                TextButton(onClick = { actions.onType(type) }, modifier = Modifier.padding(end = 4.dp).semantics { contentDescription = seeAll }) {
                    Text(stringResource(R.string.see_all))
                }
            }
        }
    }
    items(rows, key = { "$type-${key(it)}" }) { row(it) }
}

@Composable
private fun TopResultCard(kind: SearchKind, index: Int, state: SearchUiState, actions: SearchActions) {
    val r = state.results ?: return
    val label = stringResource(R.string.search_top)
    Card(
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.secondaryContainer),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp).testTag("search-top"),
    ) {
        Text(
            label,
            style = MaterialTheme.typography.labelLarge,
            modifier = Modifier.padding(start = 16.dp, top = 12.dp).semantics { heading() },
        )
        when (kind) {
            SearchKind.ARTIST -> r.artists.getOrNull(index)?.let { ArtistResultRow(it, actions) }
            SearchKind.ALBUM -> r.albums.getOrNull(index)?.let { AlbumResultRow(it, actions) }
            SearchKind.TRACK -> r.tracks.getOrNull(index)?.let { TrackResultRow(it, actions) }
        }
    }
}

@Composable
private fun ArtistResultRow(a: Artist, actions: SearchActions) {
    val name = a.name.ifEmpty { stringResource(R.string.unknown_artist) }
    ListItem(
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.Person, contentDescription = null) },
        headlineContent = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        trailingContent = {
            IconButton(onClick = { actions.onMore(More.artist(a)) }) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, name)) }
        },
        modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_artist)) { actions.onArtist(a) },
    )
}

@Composable
private fun AlbumResultRow(a: Album, actions: SearchActions) {
    BrowseSwipe({ p -> actions.onPlay(Selection.Album(a.id), p) }) { m ->
        AlbumRow(
            a,
            onClick = { actions.onAlbum(a) },
            onMore = { actions.onMore(More.album(a)) },
            // One trailing control (Wave 3.5 rule 3): ⋮, whose sheet plays it too.
            modifier = m,
        )
    }
}

@Composable
private fun TrackResultRow(t: Track, actions: SearchActions) {
    val id = t.id
    BrowseSwipe(id?.let { i -> { p: Placement -> actions.onPlay(Selection.Track(i), p) } }) { m ->
        TrackRow(
            t,
            onClick = { id?.let { actions.onPlay(Selection.Track(it), Placement.PLAY_NOW) } },
            showArt = true,
            onMore = More.track(t)?.let { r -> { actions.onMore(r) } },
            modifier = m,
        )
    }
}

@Composable
private fun PlaylistResultRow(p: SavedPlaylist, actions: SearchActions) {
    ListItem(
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.AutoMirrored.Outlined.QueueMusic, contentDescription = null) },
        headlineContent = { Text(p.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        trailingContent = {
            IconButton(onClick = { actions.onMore(More.playlist(p)) }) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, p.name)) }
        },
        modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_playlist)) { actions.onPlaylist(p) },
    )
}

@Composable
private fun TypedResults(state: SearchUiState, typed: PagedState<SearchHit>, actions: SearchActions) {
    when {
        typed.failedEmpty -> ErrorState(typed.error!!, actions.onLoadMore)
        typed.items.isEmpty() && !typed.endReached -> LoadingState()
        typed.items.isEmpty() -> EmptyState(stringResource(R.string.search_nothing, state.query.trim()))
        else -> {
            val list = rememberLazyListState()
            LoadMoreWhenNear(
                lastVisible = { list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                total = { typed.items.size },
                onLoadMore = actions.onLoadMore,
            )
            LazyColumn(state = list, modifier = Modifier.fillMaxSize().testTag("search-typed"), contentPadding = PaddingValues(bottom = 16.dp)) {
                itemsIndexed(typed.items, key = { i, h -> "$i-${h.hashCode()}" }) { _, h ->
                    when (h) {
                        is SearchHit.OfArtist -> ArtistResultRow(h.artist, actions)
                        is SearchHit.OfAlbum -> AlbumResultRow(h.album, actions)
                        is SearchHit.OfTrack -> TrackResultRow(h.track, actions)
                        is SearchHit.OfPlaylist -> PlaylistResultRow(h.playlist, actions)
                    }
                }
                if (!typed.endReached || typed.error != null) item { PageFooter(typed, actions.onLoadMore) }
            }
        }
    }
}

