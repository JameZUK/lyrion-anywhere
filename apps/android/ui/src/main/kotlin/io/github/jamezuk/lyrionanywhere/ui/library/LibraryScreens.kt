package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyGridScope
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.PlaylistAdd
import androidx.compose.material.icons.automirrored.outlined.Sort
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
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
import io.github.jamezuk.lyrionanywhere.ui.menus.More
import io.github.jamezuk.lyrionanywhere.ui.menus.MoreRequest
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtTinted

private const val GRID_ART_PX = 300
private const val LIST_ART_PX = 120

/** Load the next page when the list nears its end. */
@Composable
internal fun LoadMoreWhenNear(lastVisible: () -> Int, total: () -> Int, onLoadMore: () -> Unit) {
    val near by remember { derivedStateOf { total() > 0 && lastVisible() >= total() - 20 } }
    LaunchedEffect(near) { if (near) onLoadMore() }
}

@Composable
internal fun PageFooter(state: PagedState<*>, onRetry: () -> Unit) {
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

/** Album orders offered on album lists (random has its own page). */
val ALBUM_SORTS = listOf(AlbumSort.TITLE, AlbumSort.ARTIST, AlbumSort.YEAR, AlbumSort.NEWEST)

@Composable
fun AlbumSortMenu(sort: AlbumSort, onSort: (AlbumSort) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) {
            Icon(Icons.AutoMirrored.Outlined.Sort, contentDescription = stringResource(R.string.sort_albums))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            ALBUM_SORTS.forEach { s ->
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

/** A grid of albums, paged as it scrolls, with loading, empty and error states. */
@Composable
fun AlbumGrid(
    state: PagedState<Album>,
    onAlbum: (Album) -> Unit,
    onLoadMore: () -> Unit,
    modifier: Modifier = Modifier,
    onMore: ((MoreRequest) -> Unit)? = null,
    empty: String? = null,
    header: (@Composable () -> Unit)? = null,
) {
    when {
        state.failedEmpty && header == null -> ErrorState(state.error!!, onRetry = onLoadMore, modifier = modifier)
        state.items.isEmpty() && state.loading && header == null -> LoadingState(modifier)
        state.items.isEmpty() && state.endReached && header == null ->
            EmptyState(empty ?: stringResource(R.string.no_albums), modifier)
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
                albumItems(state.items, onAlbum, onMore)
                if (header != null && state.items.isEmpty() && state.endReached) {
                    item(span = { GridItemSpan(maxLineSpan) }) { Text(empty ?: stringResource(R.string.no_albums)) }
                }
                if (!state.endReached || state.error != null) {
                    item(span = { GridItemSpan(maxLineSpan) }) { PageFooter(state, onLoadMore) }
                }
            }
        }
    }
}

internal fun LazyGridScope.albumItems(albums: List<Album>, onAlbum: (Album) -> Unit, onMore: ((MoreRequest) -> Unit)?, prefix: String = "") {
    itemsIndexed(albums, key = { i, a -> "$prefix${a.id}-$i" }) { _, album ->
        AlbumTile(album, onClick = { onAlbum(album) }, onMore = onMore?.let { m -> { m(More.album(album)) } })
    }
}

/** An album: tap to open; long-press (or the accessibility action) for "More…". */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun AlbumTile(album: Album, onClick: () -> Unit, onMore: (() -> Unit)? = null, showArtist: Boolean = true) {
    val moreLabel = stringResource(R.string.action_more)
    Column(
        Modifier
            .combinedClickable(
                onClickLabel = stringResource(R.string.open_album),
                onLongClickLabel = if (onMore != null) moreLabel else null,
                onLongClick = onMore,
                onClick = onClick,
            )
            .semantics {
                if (onMore != null) customActions = listOf(CustomAccessibilityAction(moreLabel) { onMore(); true })
            }
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
        val sub = if (showArtist) album.artist else album.year?.toString()
        sub?.let {
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
    onMore: ((MoreRequest) -> Unit)? = null,
    header: (@Composable () -> Unit)? = null,
) {
    when {
        state.failedEmpty && header == null -> ErrorState(state.error!!, onRetry = onLoadMore, modifier = modifier)
        state.items.isEmpty() && state.loading && header == null -> LoadingState(modifier)
        state.items.isEmpty() && state.endReached && header == null -> EmptyState(stringResource(R.string.no_artists), modifier)
        else -> {
            val list = rememberLazyListState()
            LoadMoreWhenNear(
                lastVisible = { list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                total = { state.items.size },
                onLoadMore = onLoadMore,
            )
            LazyColumn(state = list, modifier = modifier.fillMaxSize()) {
                if (header != null) item { header() }
                itemsIndexed(state.items, key = { i, a -> "${a.id}-$i" }) { _, artist ->
                    val name = artist.name.ifEmpty { stringResource(R.string.unknown_artist) }
                    ListItem(
                        headlineContent = { Text(name) },
                        trailingContent = onMore?.let { m ->
                            {
                                IconButton(onClick = { m(More.artist(artist)) }) {
                                    Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, name))
                                }
                            }
                        },
                        modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_artist)) { onArtist(artist) },
                    )
                }
                if (header != null && state.items.isEmpty() && state.endReached) {
                    item { Text(stringResource(R.string.no_artists), Modifier.padding(16.dp)) }
                }
                if (!state.endReached || state.error != null) {
                    item { PageFooter(state, onLoadMore) }
                }
            }
        }
    }
}

/** Play, Shuffle, Add to queue and More, for an album, artist, playlist or genre. */
@Composable
fun PlayButtons(
    onPlay: (Placement) -> Unit,
    onShuffle: (() -> Unit)?,
    onMore: (() -> Unit)?,
    modifier: Modifier = Modifier,
    playLabel: Int = R.string.action_play,
    /** "Download" (offline), when downloads are offered here. */
    onDownload: (() -> Unit)? = null,
    downloadMark: io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMark = io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMark.NONE,
) {
    FlowRow(
        modifier,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Button(onClick = { onPlay(Placement.PLAY_NOW) }) {
            Icon(Icons.Outlined.PlayArrow, contentDescription = null)
            Text(stringResource(playLabel), Modifier.padding(start = 4.dp))
        }
        if (onShuffle != null) {
            FilledTonalButton(onClick = onShuffle) {
                Icon(Icons.Outlined.Shuffle, contentDescription = null)
                Text(stringResource(R.string.shuffle), Modifier.padding(start = 4.dp))
            }
        }
        IconButton(onClick = { onPlay(Placement.ADD_TO_QUEUE) }) {
            Icon(Icons.AutoMirrored.Outlined.PlaylistAdd, contentDescription = stringResource(R.string.action_add_to_queue))
        }
        if (onDownload != null) {
            io.github.jamezuk.lyrionanywhere.ui.offline.DownloadButton(downloadMark, onDownload)
        }
        if (onMore != null) {
            IconButton(onClick = onMore) {
                Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.action_more))
            }
        }
    }
}

/** A page header tinted by the artwork's colours: art, kind, title, lines, buttons. */
@Composable
fun TintedHeader(
    artwork: String?,
    kind: String,
    title: String,
    lines: List<String>,
    buttons: @Composable () -> Unit,
    onLine: ((Int) -> Unit)? = null,
) {
    val scheme = MaterialTheme.colorScheme
    Column(
        Modifier
            .fillMaxWidth()
            .background(Brush.verticalGradient(listOf(scheme.primaryContainer, scheme.background)))
            .padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        ArtworkImage(
            artwork,
            600,
            contentDescription = stringResource(R.string.album_cover),
            modifier = Modifier.widthIn(max = 320.dp).fillMaxWidth(0.72f),
            corner = 12.dp,
        )
        Text(
            kind.uppercase(),
            style = MaterialTheme.typography.labelMedium,
            color = scheme.onPrimaryContainer,
            modifier = Modifier.padding(top = 8.dp),
        )
        Text(
            title,
            style = MaterialTheme.typography.headlineSmall,
            color = scheme.onBackground,
            modifier = Modifier.semantics { heading() },
        )
        lines.forEachIndexed { i, line ->
            val click = onLine?.takeIf { i == 0 }
            Text(
                line,
                style = if (i == 0) MaterialTheme.typography.titleMedium else MaterialTheme.typography.bodyMedium,
                color = if (i == 0) scheme.primary else scheme.onSurfaceVariant,
                modifier = if (click != null) {
                    Modifier.clickable(onClickLabel = stringResource(R.string.open_artist)) { click(i) }.padding(vertical = 12.dp)
                } else {
                    Modifier
                },
            )
        }
        buttons()
    }
}

/** What the album page can do. */
data class AlbumActions(
    val onRetry: () -> Unit,
    val onPlay: (Placement, Int?) -> Unit,
    val onShuffle: () -> Unit,
    val onPlayTrack: (Track) -> Unit,
    val onArtist: (Long, String) -> Unit,
    val onMore: (MoreRequest) -> Unit,
    /** Download the album (offline); null where downloads aren't offered. */
    val onDownload: (() -> Unit)? = null,
    /** Which of the album's tracks are on the device. */
    val downloaded: Set<Long> = emptySet(),
    /** "Start radio" and "More like this" (A6), when the server offers them. */
    val discovery: io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryUiState =
        io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryUiState(),
    val onStartRadio: () -> Unit = {},
    val onSimilar: (io.github.jamezuk.lyrionanywhere.core.MenuItem) -> Unit = {},
)

/** The album page: large art, details, Play/Shuffle/Add/More, and tracks by disc. */
@Composable
fun AlbumScreen(state: Loadable<AlbumDetail>, actions: AlbumActions, modifier: Modifier = Modifier) {
    when (state) {
        Loadable.Loading -> LoadingState(modifier)
        is Loadable.Failed -> ErrorState(state.error, actions.onRetry, modifier)
        is Loadable.Ready -> {
            val d = state.value
            val album = d.album
            ArtTinted(album?.artwork) {
                LazyColumn(
                    modifier.fillMaxSize().background(MaterialTheme.colorScheme.background),
                    contentPadding = PaddingValues(bottom = 16.dp),
                ) {
                    item {
                        val details = listOfNotNull(
                            d.year?.toString(),
                            d.genres.take(2).joinToString(", ").ifEmpty { null },
                            pluralTracks(d.tracks.size),
                            formatDuration(d.totalSecs).takeIf { d.totalSecs > 0 },
                        ).joinToString(" · ")
                        val title = album?.title?.ifEmpty { null } ?: stringResource(R.string.unknown_album)
                        TintedHeader(
                            artwork = album?.artwork,
                            kind = stringResource(R.string.title_album),
                            title = title,
                            lines = listOfNotNull(album?.artist, details, d.formats.joinToString(", ").ifEmpty { null }),
                            onLine = if (album?.artist != null && album.artistId != null) {
                                { actions.onArtist(album.artistId!!, album.artist!!) }
                            } else {
                                null
                            },
                            buttons = {
                                PlayButtons(
                                    onPlay = { actions.onPlay(it, null) },
                                    onShuffle = actions.onShuffle,
                                    onMore = album?.let { a -> { actions.onMore(More.album(a)) } },
                                    modifier = Modifier.padding(top = 8.dp),
                                    onDownload = actions.onDownload,
                                    downloadMark = io.github.jamezuk.lyrionanywhere.ui.offline.downloadMark(
                                        d.tracks.mapNotNull { it.id },
                                        actions.downloaded,
                                    ),
                                )
                            },
                        )
                        io.github.jamezuk.lyrionanywhere.ui.discovery.StartRadioButton(actions.discovery, actions.onStartRadio)
                        HorizontalDivider()
                    }
                    if (d.tracks.isEmpty()) {
                        item { EmptyState(stringResource(R.string.no_tracks), Modifier.padding(24.dp)) }
                    }
                    val discs = d.discs
                    discs.forEach { disc ->
                        if (disc.number != null) {
                            item(key = "disc-${disc.number}") {
                                SectionHeader(stringResource(R.string.disc_number, disc.number))
                            }
                        }
                        items(disc.tracks, key = { t -> "${disc.number}-${t.id}-${t.trackNumber}-${t.title}" }) { track ->
                            TrackRow(
                                track,
                                onClick = { actions.onPlayTrack(track) },
                                showArtist = d.variousArtists,
                                onMore = More.track(track)?.let { r -> { actions.onMore(r) } },
                            )
                        }
                    }
                    if (actions.discovery.similar.isNotEmpty()) {
                        item(key = "more-like-this") {
                            io.github.jamezuk.lyrionanywhere.ui.discovery.MoreLikeThisShelf(
                                actions.discovery,
                                actions.onSimilar,
                                Modifier.padding(top = 16.dp),
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun pluralTracks(n: Int): String =
    androidx.compose.ui.platform.LocalResources.current.getQuantityString(R.plurals.track_count, n, n)

@Composable
fun TrackRow(
    track: Track,
    onClick: () -> Unit,
    highlighted: Boolean = false,
    showArt: Boolean = false,
    showArtist: Boolean = true,
    onMore: (() -> Unit)? = null,
) {
    val playLabel = stringResource(R.string.play_track)
    val title = track.title.ifEmpty { stringResource(R.string.unknown_track) }
    ListItem(
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
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
                title,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                color = if (highlighted) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurface,
            )
        },
        supportingContent = track.artist?.takeIf { showArtist }?.let { { Text(it, maxLines = 1, overflow = TextOverflow.Ellipsis) } },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                track.durationSecs?.let { Text(formatDuration(it)) }
                if (onMore != null) {
                    IconButton(onClick = onMore) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, title)) }
                }
            }
        },
        modifier = Modifier.clickable(onClickLabel = playLabel, onClick = onClick),
    )
}

/** What the artist page can do. */
data class ArtistActions(
    val onRetry: () -> Unit,
    val onAlbum: (Album) -> Unit,
    val onPlay: (Placement) -> Unit,
    val onShuffle: () -> Unit,
    val onMore: (MoreRequest) -> Unit,
)

@Composable
fun sectionLabel(s: ArtistSection): String = stringResource(
    when (s) {
        ArtistSection.ALBUMS -> R.string.section_albums
        ArtistSection.APPEARS_ON -> R.string.section_appears_on
        ArtistSection.COMPILATIONS -> R.string.section_compilations
    },
)

/** The artist page: header art, Play all/Shuffle/More, and albums grouped by role. */
@Composable
fun ArtistScreen(
    id: Long,
    name: String,
    state: Loadable<ArtistDetail>,
    actions: ArtistActions,
    modifier: Modifier = Modifier,
) {
    when (state) {
        Loadable.Loading -> LoadingState(modifier)
        is Loadable.Failed -> ErrorState(state.error, actions.onRetry, modifier)
        is Loadable.Ready -> {
            val d = state.value
            ArtTinted(d.headerArt) {
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(150.dp),
                    contentPadding = PaddingValues(bottom = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                    modifier = modifier.fillMaxSize().background(MaterialTheme.colorScheme.background),
                ) {
                    item(span = { GridItemSpan(maxLineSpan) }) {
                        TintedHeader(
                            artwork = d.headerArt,
                            kind = stringResource(R.string.kind_artist),
                            title = name,
                            lines = listOf(
                                androidx.compose.ui.platform.LocalResources.current
                                    .getQuantityString(R.plurals.album_count, d.albums.size, d.albums.size),
                            ),
                            buttons = {
                                PlayButtons(
                                    onPlay = actions.onPlay,
                                    onShuffle = actions.onShuffle,
                                    onMore = { actions.onMore(More.artist(Artist(id, name))) },
                                    playLabel = R.string.action_play_all,
                                    modifier = Modifier.padding(top = 8.dp),
                                )
                            },
                        )
                    }
                    if (d.albums.isEmpty()) {
                        item(span = { GridItemSpan(maxLineSpan) }) { EmptyState(stringResource(R.string.no_albums)) }
                    }
                    d.sections.forEach { (section, albums) ->
                        if (d.sections.size > 1) {
                            item(span = { GridItemSpan(maxLineSpan) }, key = "s-$section") { SectionHeader(sectionLabel(section)) }
                        }
                        itemsIndexed(albums, key = { i, a -> "$section-${a.id}-$i" }) { _, album ->
                            Box(Modifier.padding(horizontal = 6.dp)) {
                                AlbumTile(
                                    album,
                                    onClick = { actions.onAlbum(album) },
                                    onMore = { actions.onMore(More.album(album)) },
                                    showArtist = section != ArtistSection.ALBUMS,
                                )
                            }
                        }
                    }
                }
            }
        }
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
    onMore: ((MoreRequest) -> Unit)? = null,
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
                    TrackRow(
                        t,
                        onClick = { onTrack(t) },
                        showArt = true,
                        onMore = onMore?.let { m -> More.track(t)?.let { r -> { m(r) } } },
                    )
                }
            }
        }
    }
}

@Composable
fun SectionHeader(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.primary,
        modifier = modifier
            .padding(start = 16.dp, top = 16.dp, bottom = 4.dp)
            .semantics { heading() },
    )
}
