package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.PlaylistAdd
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.automirrored.outlined.Sort
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration

private const val GRID_ART_PX = 300
private const val LIST_ART_PX = 120

/** Load the next page when the list nears its end. */
@Composable
private fun LoadMoreWhenNear(lastVisible: () -> Int, total: () -> Int, onLoadMore: () -> Unit) {
    val near by remember { derivedStateOf { total() > 0 && lastVisible() >= total() - 20 } }
    LaunchedEffect(near) { if (near) onLoadMore() }
}

@Composable
private fun PageFooter(state: PagedState<*>, onRetry: () -> Unit) {
    Box(Modifier.fillMaxWidth().padding(16.dp), contentAlignment = Alignment.Center) {
        when {
            state.loading -> CircularProgressIndicator(Modifier.size(24.dp))
            state.error != null -> Column(horizontalAlignment = Alignment.CenterHorizontally) {
                Text(errorMessage(state.error), style = MaterialTheme.typography.bodyMedium)
                TextButton(onClick = onRetry) { Text(stringResource(R.string.action_retry)) }
            }
        }
    }
}

@Composable
fun sortLabel(sort: AlbumSort): String = stringResource(
    when (sort) {
        AlbumSort.TITLE -> R.string.sort_title
        AlbumSort.ARTIST -> R.string.sort_artist
        AlbumSort.YEAR -> R.string.sort_year
        AlbumSort.NEWEST -> R.string.sort_newest
        AlbumSort.RANDOM -> R.string.sort_random
    },
)

@Composable
fun AlbumSortMenu(sort: AlbumSort, onSort: (AlbumSort) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(Icons.AutoMirrored.Outlined.Sort, contentDescription = stringResource(R.string.sort_albums))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            AlbumSort.entries.forEach { s ->
                DropdownMenuItem(
                    text = { Text(sortLabel(s)) },
                    onClick = {
                        open = false
                        onSort(s)
                    },
                    trailingIcon = if (s == sort) {
                        { Text("✓") }
                    } else {
                        null
                    },
                )
            }
        }
    }
}

@Composable
fun AlbumGrid(
    state: PagedState<Album>,
    onAlbum: (Album) -> Unit,
    onLoadMore: () -> Unit,
    modifier: Modifier = Modifier,
    header: (@Composable () -> Unit)? = null,
) {
    when {
        state.failedEmpty -> ErrorState(state.error!!, onRetry = onLoadMore, modifier = modifier)
        state.items.isEmpty() && state.loading -> LoadingState(modifier)
        state.items.isEmpty() && state.endReached -> EmptyState(stringResource(R.string.no_albums), modifier)
        else -> {
            val grid = rememberLazyGridState()
            LoadMoreWhenNear(
                lastVisible = { grid.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                total = { state.items.size },
                onLoadMore = onLoadMore,
            )
            LazyVerticalGrid(
                columns = GridCells.Adaptive(150.dp),
                state = grid,
                contentPadding = PaddingValues(12.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
                verticalArrangement = Arrangement.spacedBy(16.dp),
                modifier = modifier.fillMaxSize(),
            ) {
                if (header != null) item(span = { GridItemSpan(maxLineSpan) }) { header() }
                itemsIndexed(state.items, key = { i, a -> "${a.id}-$i" }) { _, album ->
                    AlbumTile(album, onClick = { onAlbum(album) })
                }
                if (!state.endReached || state.error != null) {
                    item(span = { GridItemSpan(maxLineSpan) }) { PageFooter(state, onLoadMore) }
                }
            }
        }
    }
}

@Composable
fun AlbumTile(album: Album, onClick: () -> Unit) {
    Column(
        Modifier
            .clickable(onClickLabel = stringResource(R.string.open_album), onClick = onClick)
            .padding(bottom = 4.dp),
    ) {
        ArtworkImage(album.artwork, GRID_ART_PX, contentDescription = null, modifier = Modifier.fillMaxWidth())
        Text(
            album.title.ifEmpty { stringResource(R.string.unknown_album) },
            style = MaterialTheme.typography.titleSmall,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 6.dp),
        )
        album.artist?.let {
            Text(
                it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
fun ArtistList(
    state: PagedState<Artist>,
    onArtist: (Artist) -> Unit,
    onLoadMore: () -> Unit,
    modifier: Modifier = Modifier,
) {
    when {
        state.failedEmpty -> ErrorState(state.error!!, onRetry = onLoadMore, modifier = modifier)
        state.items.isEmpty() && state.loading -> LoadingState(modifier)
        state.items.isEmpty() && state.endReached -> EmptyState(stringResource(R.string.no_artists), modifier)
        else -> {
            val list = rememberLazyListState()
            LoadMoreWhenNear(
                lastVisible = { list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                total = { state.items.size },
                onLoadMore = onLoadMore,
            )
            LazyColumn(state = list, modifier = modifier.fillMaxSize()) {
                itemsIndexed(state.items, key = { i, a -> "${a.id}-$i" }) { _, artist ->
                    ListItem(
                        headlineContent = { Text(artist.name.ifEmpty { stringResource(R.string.unknown_artist) }) },
                        modifier = Modifier.clickable(
                            onClickLabel = stringResource(R.string.open_artist),
                        ) { onArtist(artist) },
                    )
                }
                if (!state.endReached || state.error != null) {
                    item { PageFooter(state, onLoadMore) }
                }
            }
        }
    }
}

/** The album page: cover, title, actions and tracks. */
@Composable
fun AlbumScreen(
    state: Loadable<AlbumDetail>,
    onRetry: () -> Unit,
    onPlay: (Placement, Int?) -> Unit,
    onPlayTrack: (Track) -> Unit,
    onArtist: (Long, String) -> Unit,
    modifier: Modifier = Modifier,
) {
    when (state) {
        Loadable.Loading -> LoadingState(modifier)
        is Loadable.Failed -> ErrorState(state.error, onRetry, modifier)
        is Loadable.Ready -> {
            val album = state.value.album
            val tracks = state.value.tracks
            LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
                item {
                    Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        ArtworkImage(
                            album?.artwork,
                            600,
                            contentDescription = stringResource(R.string.album_cover),
                            modifier = Modifier.fillMaxWidth(0.7f).align(Alignment.CenterHorizontally),
                            corner = 12.dp,
                        )
                        Text(
                            album?.title?.ifEmpty { null } ?: stringResource(R.string.unknown_album),
                            style = MaterialTheme.typography.headlineSmall,
                            modifier = Modifier.semantics { heading() },
                        )
                        val artist = album?.artist
                        val artistId = album?.artistId
                        if (artist != null) {
                            Text(
                                artist,
                                style = MaterialTheme.typography.titleMedium,
                                color = MaterialTheme.colorScheme.primary,
                                modifier = if (artistId != null) {
                                    Modifier.clickable(onClickLabel = stringResource(R.string.open_artist)) {
                                        onArtist(artistId, artist)
                                    }
                                } else {
                                    Modifier
                                },
                            )
                        }
                        album?.year?.let {
                            Text(it.toString(), style = MaterialTheme.typography.bodyMedium)
                        }
                        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            Button(onClick = { onPlay(Placement.PLAY_NOW, null) }) {
                                Icon(Icons.Outlined.PlayArrow, contentDescription = null)
                                Text(stringResource(R.string.action_play), Modifier.padding(start = 4.dp))
                            }
                            OutlinedButton(onClick = { onPlay(Placement.PLAY_NEXT, null) }) {
                                Icon(Icons.AutoMirrored.Outlined.QueueMusic, contentDescription = null)
                                Text(stringResource(R.string.action_play_next), Modifier.padding(start = 4.dp))
                            }
                            IconButton(onClick = { onPlay(Placement.ADD_TO_QUEUE, null) }) {
                                Icon(
                                    Icons.AutoMirrored.Outlined.PlaylistAdd,
                                    contentDescription = stringResource(R.string.action_add_to_queue),
                                )
                            }
                        }
                    }
                    HorizontalDivider()
                }
                if (tracks.isEmpty()) {
                    item { EmptyState(stringResource(R.string.no_tracks), Modifier.padding(24.dp)) }
                }
                items(tracks, key = { t -> "${t.id}-${t.trackNumber}-${t.title}" }) { track ->
                    TrackRow(track, onClick = { onPlayTrack(track) })
                }
            }
        }
    }
}

@Composable
fun TrackRow(track: Track, onClick: () -> Unit, highlighted: Boolean = false, showArt: Boolean = false) {
    val playLabel = stringResource(R.string.play_track)
    ListItem(
        leadingContent = when {
            showArt -> {
                { ArtworkImage(track.artwork, LIST_ART_PX, contentDescription = null, modifier = Modifier.size(48.dp)) }
            }
            track.trackNumber != null -> {
                { Text(track.trackNumber.toString(), Modifier.width(28.dp), style = MaterialTheme.typography.bodyMedium) }
            }
            else -> null
        },
        headlineContent = {
            Text(
                track.title.ifEmpty { stringResource(R.string.unknown_track) },
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                color = if (highlighted) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
            )
        },
        supportingContent = track.artist?.let { { Text(it, maxLines = 1, overflow = TextOverflow.Ellipsis) } },
        trailingContent = track.durationSecs?.let { { Text(formatDuration(it)) } },
        modifier = Modifier.clickable(onClickLabel = playLabel, onClick = onClick),
    )
}

@Composable
fun ArtistScreen(
    name: String,
    state: Loadable<List<Album>>,
    onRetry: () -> Unit,
    onAlbum: (Album) -> Unit,
    onPlayAll: (Placement) -> Unit,
    modifier: Modifier = Modifier,
) {
    when (state) {
        Loadable.Loading -> LoadingState(modifier)
        is Loadable.Failed -> ErrorState(state.error, onRetry, modifier)
        is Loadable.Ready -> AlbumGrid(
            state = PagedState(items = state.value, total = state.value.size),
            onAlbum = onAlbum,
            onLoadMore = {},
            modifier = modifier,
            header = {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        name,
                        style = MaterialTheme.typography.headlineSmall,
                        modifier = Modifier.semantics { heading() },
                    )
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { onPlayAll(Placement.PLAY_NOW) }) {
                            Icon(Icons.Outlined.PlayArrow, contentDescription = null)
                            Text(stringResource(R.string.action_play_all), Modifier.padding(start = 4.dp))
                        }
                        OutlinedButton(onClick = { onPlayAll(Placement.ADD_TO_QUEUE) }) {
                            Text(stringResource(R.string.action_add_to_queue))
                        }
                    }
                }
            },
        )
    }
}

@Composable
fun SearchResultsList(
    state: SearchUiState,
    onRetry: () -> Unit,
    onArtist: (Artist) -> Unit,
    onAlbum: (Album) -> Unit,
    onTrack: (Track) -> Unit,
    modifier: Modifier = Modifier,
) {
    val results = state.results
    when {
        state.error != null -> ErrorState(state.error, onRetry, modifier)
        state.loading && results == null -> LoadingState(modifier)
        results == null -> EmptyState(stringResource(R.string.search_hint), modifier)
        results.artists.isEmpty() && results.albums.isEmpty() && results.tracks.isEmpty() ->
            EmptyState(stringResource(R.string.search_nothing, state.query.trim()), modifier)
        else -> LazyColumn(modifier.fillMaxSize()) {
            if (results.artists.isNotEmpty()) {
                item { SectionHeader(stringResource(R.string.section_artists)) }
                items(results.artists, key = { "ar${it.id}" }) { a ->
                    ListItem(
                        headlineContent = { Text(a.name) },
                        modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_artist)) { onArtist(a) },
                    )
                }
            }
            if (results.albums.isNotEmpty()) {
                item { SectionHeader(stringResource(R.string.section_albums)) }
                items(results.albums, key = { "al${it.id}" }) { a ->
                    ListItem(
                        headlineContent = { Text(a.title) },
                        modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_album)) { onAlbum(a) },
                    )
                }
            }
            if (results.tracks.isNotEmpty()) {
                item { SectionHeader(stringResource(R.string.section_tracks)) }
                items(results.tracks, key = { "tr${it.id}" }) { t ->
                    TrackRow(t, onClick = { onTrack(t) }, showArt = true)
                }
            }
        }
    }
}

@Composable
fun SectionHeader(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = Modifier
            .padding(start = 16.dp, top = 16.dp, bottom = 4.dp)
            .semantics { heading() },
    )
}
