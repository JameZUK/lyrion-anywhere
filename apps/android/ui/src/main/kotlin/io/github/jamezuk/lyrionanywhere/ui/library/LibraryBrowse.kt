package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.rememberLazyGridState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Sort
import androidx.compose.material.icons.automirrored.outlined.ViewList
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.GridView
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.utf16CodePoint
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.IndexEntry
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.data.LibraryAlbumOrder
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.menus.More
import io.github.jamezuk.lyrionanywhere.ui.menus.MoreRequest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/** A chip's label. */
@Composable
fun libraryChipLabel(c: LibraryChip): String = stringResource(
    when (c) {
        LibraryChip.ALBUMS -> R.string.lib_albums
        LibraryChip.ARTISTS -> R.string.lib_artists
        LibraryChip.PLAYLISTS -> R.string.lib_playlists
        LibraryChip.FAVOURITES -> R.string.lib_favourites
        LibraryChip.FILTERS -> R.string.lib_filters
        LibraryChip.DOWNLOADED -> R.string.chip_downloaded
    },
)

/** The Library's type chips (Spotify style): tap one to show only that; tap it again for everything. */
@Composable
fun LibraryChips(selected: LibraryChip?, onToggle: (LibraryChip) -> Unit, modifier: Modifier = Modifier) {
    val group = stringResource(R.string.library_types)
    Row(
        modifier
            .fillMaxWidth()
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 12.dp)
            .semantics { contentDescription = group }
            .testTag("library-chips"),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        LibraryChip.entries.forEach { c ->
            FilterChip(
                selected = c == selected,
                onClick = { onToggle(c) },
                label = { Text(libraryChipLabel(c)) },
                modifier = Modifier.testTag("chip-${c.name.lowercase()}"),
            )
        }
    }
}

@Composable
fun librarySortLabel(order: LibraryAlbumOrder): String = stringResource(
    when (order) {
        LibraryAlbumOrder.NEWEST -> R.string.sort_recently_added
        LibraryAlbumOrder.RECENTLY_PLAYED -> R.string.sort_recently_played
        LibraryAlbumOrder.TITLE -> R.string.sort_a_to_z
        LibraryAlbumOrder.ARTIST -> R.string.sort_by_artist_short
        LibraryAlbumOrder.YEAR -> R.string.sort_by_year_short
    },
)

/** The albums' order and view: "Sort: A–Z" (a menu) and grid or list. */
@Composable
fun AlbumBrowseBar(sort: LibraryAlbumOrder, grid: Boolean, onSort: (LibraryAlbumOrder) -> Unit, onGrid: (Boolean) -> Unit) {
    var open by remember { mutableStateOf(false) }
    Row(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f)) {
            TextButton(onClick = { open = true }, modifier = Modifier.testTag("album-sort")) {
                Icon(Icons.AutoMirrored.Outlined.Sort, contentDescription = null)
                Text(
                    stringResource(R.string.sort_label, librarySortLabel(sort)),
                    Modifier.padding(start = 8.dp),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                LibraryAlbumOrder.OFFERED.forEach { s ->
                    DropdownMenuItem(
                        text = { Text(librarySortLabel(s)) },
                        onClick = {
                            open = false
                            onSort(s)
                        },
                        trailingIcon = if (s == sort) {
                            { Icon(Icons.Outlined.Check, contentDescription = null) }
                        } else {
                            null
                        },
                        modifier = Modifier.semantics { selected = s == sort }.testTag("album-sort-${s.name.lowercase()}"),
                    )
                }
            }
        }
        // Says what it does next (a toggle's label mustn't depend on colour or state alone).
        IconButton(onClick = { onGrid(!grid) }, modifier = Modifier.testTag("album-view")) {
            Icon(
                if (grid) Icons.AutoMirrored.Outlined.ViewList else Icons.Outlined.GridView,
                contentDescription = stringResource(if (grid) R.string.view_as_list else R.string.view_as_grid),
            )
        }
    }
}

/** Read the pages the list is showing (again after a failure is cleared). */
@Composable
private fun EnsureVisible(first: () -> Int, last: () -> Int, retryKey: Any?, onEnsure: (Int) -> Unit) {
    val ensure by rememberUpdatedState(onEnsure)
    LaunchedEffect(retryKey) {
        snapshotFlow { first() to last() }.distinctUntilChanged().collect { (a, b) ->
            ensure(a)
            ensure(b)
        }
    }
}

/**
 * The Library's albums: a grid or a list in the chosen order, read where they are looked at,
 * with the A–Z scroller in title order. List rows swipe right to play next and left to add to
 * the queue.
 */
@Composable
fun AlbumBrowser(
    state: SparseState<Album>,
    sort: LibraryAlbumOrder,
    grid: Boolean,
    onSort: (LibraryAlbumOrder) -> Unit,
    onGrid: (Boolean) -> Unit,
    onEnsure: (Int) -> Unit,
    onRetry: () -> Unit,
    onAlbum: (Album) -> Unit,
    onMore: (MoreRequest) -> Unit,
    modifier: Modifier = Modifier,
    onQueue: ((Album, Placement) -> Unit)? = null,
) {
    Column(modifier.fillMaxSize()) {
        AlbumBrowseBar(sort, grid, onSort, onGrid)
        when {
            state.failedEmpty -> ErrorState(state.error!!, onRetry)
            state.total == null -> LoadingState()
            state.count == 0 -> EmptyState(
                stringResource(if (sort == LibraryAlbumOrder.RECENTLY_PLAYED) R.string.no_recently_played else R.string.no_albums),
            )
            else -> Box(Modifier.fillMaxSize()) {
                // The A–Z scroller only in title order (never for recently played).
                val index = if (sort == LibraryAlbumOrder.TITLE) state.index else emptyList()
                if (grid) {
                    val g = rememberLazyGridState()
                    EnsureVisible(
                        first = { g.firstVisibleItemIndex },
                        last = { g.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                        retryKey = state.error == null,
                        onEnsure = onEnsure,
                    )
                    LazyVerticalGrid(
                        columns = GridCells.Adaptive(150.dp),
                        state = g,
                        contentPadding = PaddingValues(start = 12.dp, top = 12.dp, bottom = 12.dp, end = if (index.isEmpty()) 12.dp else 44.dp),
                        horizontalArrangement = Arrangement.spacedBy(12.dp),
                        verticalArrangement = Arrangement.spacedBy(16.dp),
                        modifier = Modifier.fillMaxSize().testTag("album-grid"),
                    ) {
                        items(state.count, key = { it }) { i ->
                            val a = state[i]
                            if (a == null) {
                                PlaceholderTile()
                            } else {
                                AlbumTile(a, onClick = { onAlbum(a) }, onMore = { onMore(More.album(a)) })
                            }
                        }
                    }
                    AlphabetScroller(index, state.count, firstVisible = { g.firstVisibleItemIndex }, onJump = { g.scrollToItem(it) })
                } else {
                    val l = rememberLazyListState()
                    EnsureVisible(
                        first = { l.firstVisibleItemIndex },
                        last = { l.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                        retryKey = state.error == null,
                        onEnsure = onEnsure,
                    )
                    LazyColumn(
                        state = l,
                        contentPadding = PaddingValues(end = if (index.isEmpty()) 0.dp else 32.dp),
                        modifier = Modifier.fillMaxSize().testTag("album-list"),
                    ) {
                        items(state.count, key = { it }) { i ->
                            val a = state[i]
                            if (a == null) {
                                PlaceholderRow()
                            } else {
                                BrowseSwipe(onQueue?.let { q -> { p: Placement -> q(a, p) } }, Modifier.testTag("album-row-$i")) { m ->
                                    AlbumRow(a, onClick = { onAlbum(a) }, onMore = { onMore(More.album(a)) }, modifier = m)
                                }
                            }
                        }
                    }
                    AlphabetScroller(index, state.count, firstVisible = { l.firstVisibleItemIndex }, onJump = { l.scrollToItem(it) })
                }
            }
        }
    }
}

/** An album as a list row: art, title and artist; More on its button. */
@Composable
fun AlbumRow(album: Album, onClick: () -> Unit, onMore: (() -> Unit)?, modifier: Modifier = Modifier, trailing: (@Composable () -> Unit)? = null) {
    val title = album.title.ifEmpty { stringResource(R.string.unknown_album) }
    ListItem(
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { ArtworkImage(album.artwork, 120, contentDescription = null, modifier = Modifier.size(48.dp), corner = 4.dp) },
        headlineContent = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = listOfNotNull(album.artist, album.year?.toString()).joinToString(" · ").ifEmpty { null }?.let {
            { Text(it, maxLines = 1, overflow = TextOverflow.Ellipsis) }
        },
        trailingContent = {
            Row(verticalAlignment = Alignment.CenterVertically) {
                trailing?.invoke()
                if (onMore != null) {
                    IconButton(onClick = onMore) { Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, title)) }
                }
            }
        },
        modifier = modifier.clickable(onClickLabel = stringResource(R.string.open_album), onClick = onClick),
    )
}

@Composable
private fun PlaceholderTile() {
    val loading = stringResource(R.string.row_loading)
    Box(
        Modifier
            .fillMaxWidth()
            .aspectRatio(1f)
            .background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(8.dp))
            .semantics { contentDescription = loading },
    )
}

@Composable
private fun PlaceholderRow() {
    ListItem(
        headlineContent = { Text(stringResource(R.string.row_loading), color = MaterialTheme.colorScheme.onSurfaceVariant) },
        leadingContent = { Box(Modifier.size(48.dp).background(MaterialTheme.colorScheme.surfaceVariant, RoundedCornerShape(4.dp))) },
    )
}

/** The Library's artists, by role, read where they are looked at, with the A–Z scroller. */
@Composable
fun ArtistBrowser(
    state: SparseState<Artist>,
    views: List<ArtistView>,
    view: ArtistView,
    onView: (ArtistView) -> Unit,
    onEnsure: (Int) -> Unit,
    onRetry: () -> Unit,
    onArtist: (Artist) -> Unit,
    onMore: (MoreRequest) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier.fillMaxSize()) {
        ArtistViewChips(views, view, onView)
        when {
            state.failedEmpty -> ErrorState(state.error!!, onRetry)
            state.total == null -> LoadingState()
            state.count == 0 -> EmptyState(stringResource(R.string.no_artists))
            else -> Box(Modifier.fillMaxSize()) {
                val l = rememberLazyListState()
                EnsureVisible(
                    first = { l.firstVisibleItemIndex },
                    last = { l.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                    retryKey = state.error == null,
                    onEnsure = onEnsure,
                )
                LazyColumn(
                    state = l,
                    contentPadding = PaddingValues(end = if (state.index.isEmpty()) 0.dp else 32.dp),
                    modifier = Modifier.fillMaxSize().testTag("artist-list"),
                ) {
                    items(state.count, key = { it }) { i ->
                        val a = state[i]
                        if (a == null) {
                            PlaceholderRow()
                        } else {
                            val name = a.name.ifEmpty { stringResource(R.string.unknown_artist) }
                            ListItem(
                                headlineContent = { Text(name) },
                                trailingContent = {
                                    IconButton(onClick = { onMore(More.artist(a)) }) {
                                        Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, name))
                                    }
                                },
                                modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_artist)) { onArtist(a) },
                            )
                        }
                    }
                }
                AlphabetScroller(state.index, state.count, firstVisible = { l.firstVisibleItemIndex }, onJump = { l.scrollToItem(it) })
            }
        }
    }
}

/**
 * The A–Z fast scroller (Wave 2 §3.2), from LMS's index: drag or tap along it to jump to a
 * letter, with a large letter shown while doing so. TalkBack offers it as an adjustable
 * control ("Jump to letter", its state the current letter: swipe up or down to change it);
 * with a keyboard, Up and Down step through the letters and a typed letter jumps to it.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun androidx.compose.foundation.layout.BoxScope.AlphabetScroller(
    index: List<IndexEntry>,
    total: Int,
    firstVisible: () -> Int,
    onJump: suspend (Int) -> Unit,
) {
    if (index.size < 2 || total <= 0) return
    val scope = rememberCoroutineScope()
    var height by remember { mutableIntStateOf(0) }
    var dragging by remember { mutableStateOf<Int?>(null) }
    val current = dragging ?: AtoZ.currentAt(index, firstVisible())
    val label = stringResource(R.string.az_scroller)
    fun jump(i: Int) {
        val e = index.getOrNull(i) ?: return
        scope.launch { onJump(AtoZ.target(e, total)) }
    }
    Column(
        Modifier
            .align(Alignment.CenterEnd)
            .fillMaxHeight()
            .width(32.dp)
            .padding(vertical = 8.dp)
            .onSizeChanged { height = it.height }
            .testTag("az-scroller")
            .pointerInput(index, total) {
                awaitEachGesture {
                    val down = awaitFirstDown()
                    var at = AtoZ.atY(index, down.position.y, height.toFloat())
                    dragging = at.takeIf { it >= 0 }
                    jump(at)
                    down.consume()
                    while (true) {
                        val e = awaitPointerEvent()
                        val c = e.changes.firstOrNull { it.id == down.id } ?: break
                        if (!c.pressed) break
                        c.consume()
                        val now = AtoZ.atY(index, c.position.y, height.toFloat())
                        if (now != at && now >= 0) {
                            at = now
                            dragging = now
                            jump(now)
                        }
                    }
                    dragging = null
                }
            }
            .focusable()
            .onKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onKeyEvent false
                val cur = AtoZ.currentAt(index, firstVisible()).coerceAtLeast(0)
                when (e.key) {
                    Key.DirectionUp -> {
                        jump((cur - 1).coerceAtLeast(0))
                        true
                    }
                    Key.DirectionDown -> {
                        jump((cur + 1).coerceAtMost(index.size - 1))
                        true
                    }
                    else -> {
                        val c = e.utf16CodePoint.takeIf { it > 0 }?.toChar()
                        val i = c?.let { AtoZ.forChar(index, it) } ?: -1
                        if (i >= 0) jump(i)
                        i >= 0
                    }
                }
            }
            .semantics {
                contentDescription = label
                stateDescription = index.getOrNull(current)?.key.orEmpty()
                progressBarRangeInfo = ProgressBarRangeInfo(current.coerceAtLeast(0).toFloat(), 0f..(index.size - 1).toFloat(), index.size - 2)
                setProgress { v ->
                    val i = Math.round(v).coerceIn(0, index.size - 1)
                    jump(i)
                    true
                }
            },
        verticalArrangement = Arrangement.SpaceEvenly,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        // Every letter when they fit; otherwise every other one (the touch still finds them all).
        val step = if (index.size > 30) 2 else 1
        index.forEachIndexed { i, e ->
            if (i % step == 0) {
                Text(
                    e.key.take(1),
                    style = MaterialTheme.typography.labelSmall,
                    fontSize = 11.sp,
                    textAlign = TextAlign.Center,
                    color = if (i == current) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                )
            }
        }
    }
    val shown = dragging
    if (shown != null) {
        Surface(
            shape = RoundedCornerShape(16.dp),
            color = MaterialTheme.colorScheme.primaryContainer,
            tonalElevation = 6.dp,
            modifier = Modifier.align(Alignment.Center).size(88.dp).testTag("az-bubble"),
        ) {
            Box(contentAlignment = Alignment.Center) {
                Text(
                    index.getOrNull(shown)?.key.orEmpty(),
                    style = MaterialTheme.typography.displayMedium,
                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                    maxLines = 1,
                )
            }
        }
    }
}
