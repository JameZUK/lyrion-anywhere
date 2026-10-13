package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.ui.platform.testTag
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.ArrowUpward
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DragHandle
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Favorite
import androidx.compose.material.icons.outlined.FilterAlt
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Radio
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material.icons.outlined.WarningAmber
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.Checkbox
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.PinnedMenu
import io.github.jamezuk.lyrionanywhere.core.ShelfKind
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationSpec
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.home.HomeLayouts

/** Home's edit mode (docs/plans/wave1-design.md §7): reorder, hide, add and remove shelves. */
data class HomeEditActions(
    val onMove: (from: Int, to: Int) -> Unit,
    val onHidden: (index: Int, hidden: Boolean) -> Unit,
    val onRemove: (index: Int) -> Unit,
    val onAdd: (HomeShelf) -> Unit,
    val onReset: () -> Unit,
    val onSave: () -> Unit,
    val onCancel: () -> Unit,
)

/**
 * Home: the user's own shelves, in their order (saved on the server, the same on every
 * device). Without a saved layout it shows the default: recently played, stations, new
 * music, most played and on this day. The player card sits on top while something plays.
 */
@Composable
fun HomeScreen(
    feed: HomeFeed,
    actions: HomeActions,
    modifier: Modifier = Modifier,
    playerCard: @Composable () -> Unit = {},
    edit: HomeEditActions? = null,
) {
    val draft = feed.draft
    if (draft != null && edit != null) {
        HomeEditor(draft, feed, edit, actions.genreChoices, saving = feed.saving, modifier = modifier)
        return
    }
    val layout = feed.layout
    var creating by rememberSaveable { mutableStateOf(false) }
    LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
        item { playerCard() }
        if (layout == null) {
            item { NewMusicShelf(feed, actions) }
            item { RandomShelf(feed, actions) }
            item { FavouritesShelf(feed, actions) }
            item { PlaylistsShelf(feed, actions) }
        } else {
            for (shelf in layout.shelves.filterNot { it.hidden }) {
                item(key = "${shelf.kind}-${shelf.id}") {
                    Shelf(shelf, feed, actions, onNewStation = { creating = true })
                }
            }
            if (edit != null && feed.layoutLoaded) {
                item {
                    Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.Center) {
                        OutlinedButton(onClick = actions.onEdit, modifier = Modifier.testTag("home-edit")) {
                            Icon(Icons.Outlined.Edit, contentDescription = null, modifier = Modifier.size(18.dp))
                            Text(stringResource(R.string.home_edit), Modifier.padding(start = 8.dp))
                        }
                    }
                }
            } else if (edit != null && feed.layoutError != null) {
                // The default is shown, but it isn't the user's: no editing until it loads.
                item {
                    Column(
                        Modifier.fillMaxWidth().padding(16.dp).testTag("home-layout-failed"),
                        horizontalAlignment = Alignment.CenterHorizontally,
                    ) {
                        Text(
                            stringResource(R.string.home_layout_failed),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        TextButton(onClick = actions.onRetry) { Text(stringResource(R.string.action_retry)) }
                    }
                }
            }
        }
    }
    if (creating) {
        NewStationDialog(
            genreChoices = actions.genreChoices,
            onCreate = { name, spec ->
                creating = false
                actions.onCreateStation(name, spec)
            },
            onDismiss = { creating = false },
        )
    }
}

@Composable
private fun Shelf(shelf: HomeShelf, feed: HomeFeed, actions: HomeActions, onNewStation: () -> Unit) {
    when (shelf.kind) {
        ShelfKind.RECENT, ShelfKind.MOST, ShelfKind.ON_THIS_DAY -> {
            val title = stringResource(shelfTitle(shelf.kind))
            Carousel(
                title = title,
                state = feed.history[shelf.kind] ?: Loadable.Loading,
                onSeeAll = { actions.onSeeAll(null) },
                onRetry = actions.onRetry,
                hideWhenEmpty = shelf.kind == ShelfKind.ON_THIS_DAY,
                showSeeAll = false,
            ) { a -> AlbumCard(a, actions) }
        }
        ShelfKind.STATIONS -> StationsShelf(feed, actions, onNewStation)
        ShelfKind.NEW -> NewMusicShelf(feed, actions)
        ShelfKind.RANDOM -> RandomShelf(feed, actions)
        ShelfKind.FAVOURITES -> FavouritesShelf(feed, actions)
        ShelfKind.PLAYLISTS -> PlaylistsShelf(feed, actions)
        ShelfKind.STATION -> {
            val st = (feed.stations as? Loadable.Ready)?.value?.firstOrNull { it.id == shelf.id } ?: return
            Column(Modifier.fillMaxWidth().padding(top = 8.dp)) {
                ShelfHeading(st.name)
                Box(Modifier.padding(horizontal = 16.dp)) { StationCard(st, feed.activeStation == st.id, actions) }
            }
        }
        ShelfKind.MENU -> {
            val menu = feed.menus.firstOrNull { it.id == shelf.id } ?: return
            ListItem(
                leadingContent = { Icon(Icons.Outlined.PushPin, contentDescription = null) },
                headlineContent = { Text(menu.name) },
                supportingContent = menu.broken?.let { { Text(it) } },
                modifier = Modifier.clickable(onClickLabel = stringResource(R.string.menu_open)) { actions.onMenu(menu) },
            )
        }
        ShelfKind.GENRE -> {
            val id = shelf.id ?: return
            val name = feed.genreNames[id] ?: stringResource(R.string.home_shelf_genre)
            Carousel(
                title = name,
                state = feed.genres[id] ?: Loadable.Loading,
                onSeeAll = { actions.onGenre(id, name) },
                onRetry = actions.onRetry,
                hideWhenEmpty = true,
            ) { a -> AlbumCard(a, actions) }
        }
        ShelfKind.FILTER -> FilterShelf(shelf, feed, actions)
    }
}

/**
 * A smart filter's shelf (Wave 2 §5.6): its first albums, Play and Shuffle (its first 50),
 * and See all for its page. A deleted filter's shelf is left out.
 */
@Composable
private fun FilterShelf(shelf: HomeShelf, feed: HomeFeed, actions: HomeActions) {
    val id = shelf.id ?: return
    val filter = feed.filters.firstOrNull { it.id == id }
    if (filter == null && feed.filtersLoaded) return
    val name = filter?.name ?: stringResource(R.string.home_shelf_filter)
    val state = feed.filterAlbums[id] ?: Loadable.Loading
    val playable = (state as? Loadable.Ready)?.value?.isNotEmpty() == true && !feed.filterPlaying
    Box(Modifier.testTag("shelf-filter-$id")) {
        Carousel(
            title = name,
            state = state,
            onSeeAll = { actions.onFilter(id, name) },
            onRetry = actions.onRetry,
            empty = stringResource(R.string.filter_no_albums),
            extra = {
                if (playable) {
                    IconButton(onClick = { actions.onPlayFilter(id, name, false) }, modifier = Modifier.testTag("shelf-filter-play-$id")) {
                        Icon(Icons.Outlined.PlayArrow, stringResource(R.string.filter_play, name))
                    }
                    IconButton(onClick = { actions.onPlayFilter(id, name, true) }, modifier = Modifier.testTag("shelf-filter-shuffle-$id")) {
                        Icon(Icons.Outlined.Shuffle, stringResource(R.string.filter_shuffle, name))
                    }
                }
            },
        ) { a -> AlbumCard(a, actions) }
    }
}

@Composable
private fun ShelfHeading(title: String) {
    Text(
        title,
        style = MaterialTheme.typography.titleLarge,
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp).semantics { heading() },
    )
}

@Composable
private fun NewMusicShelf(feed: HomeFeed, actions: HomeActions) = Carousel(
    title = stringResource(R.string.home_recent),
    state = feed.recent,
    onSeeAll = { actions.onSeeAll(null) },
    onRetry = actions.onRetry,
) { a -> AlbumCard(a, actions) }

@Composable
private fun RandomShelf(feed: HomeFeed, actions: HomeActions) = Carousel(
    title = stringResource(R.string.home_random),
    state = feed.random,
    onSeeAll = { actions.onSeeAll(LibraryDestination.RANDOM) },
    onRetry = actions.onRetry,
    extra = {
        IconButton(onClick = actions.onReshuffle) {
            Icon(Icons.Outlined.Shuffle, stringResource(R.string.shuffle_again))
        }
    },
) { a -> AlbumCard(a, actions) }

@Composable
private fun FavouritesShelf(feed: HomeFeed, actions: HomeActions) = Carousel(
    title = stringResource(R.string.lib_favourites),
    state = feed.favourites,
    onSeeAll = { actions.onSeeAll(LibraryDestination.FAVOURITES) },
    onRetry = actions.onRetry,
    hideWhenEmpty = true,
) { f ->
    SmallCard(f.name, f.artwork, Icons.Outlined.Favorite) { actions.onFavourite(f) }
}

@Composable
private fun PlaylistsShelf(feed: HomeFeed, actions: HomeActions) = Carousel(
    title = stringResource(R.string.lib_playlists),
    state = feed.playlists,
    onSeeAll = { actions.onSeeAll(LibraryDestination.PLAYLISTS) },
    onRetry = actions.onRetry,
    hideWhenEmpty = true,
) { p ->
    SmallCard(p.name, null, Icons.AutoMirrored.Outlined.QueueMusic) { actions.onPlaylist(p) }
}

@Composable
private fun StationsShelf(feed: HomeFeed, actions: HomeActions, onNewStation: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(top = 8.dp)) {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                stringResource(R.string.home_stations),
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
            TextButton(onClick = onNewStation) {
                Icon(Icons.Outlined.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(stringResource(R.string.station_new), Modifier.padding(start = 4.dp))
            }
        }
        when (val st = feed.stations) {
            Loadable.Loading -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) {
                androidx.compose.material3.CircularProgressIndicator(Modifier.size(24.dp))
            }
            is Loadable.Failed -> Text(
                io.github.jamezuk.lyrionanywhere.ui.components.errorMessage(st.error),
                Modifier.padding(horizontal = 16.dp),
            )
            is Loadable.Ready -> if (st.value.isEmpty()) {
                Text(stringResource(R.string.stations_none), Modifier.padding(horizontal = 16.dp))
            } else {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    items(st.value.size) { i ->
                        val s = st.value[i]
                        Box(Modifier.width(220.dp)) { StationCard(s, feed.activeStation == s.id, actions) }
                    }
                }
            }
        }
    }
}

/** What a station plays, in a few words. */
@Composable
fun stationKind(spec: StationSpec): String = when (spec) {
    is StationSpec.Genre -> stringResource(R.string.station_kind_genre)
    is StationSpec.Decade -> stringResource(R.string.station_kind_decade, spec.decade.toInt())
    is StationSpec.Forgotten -> stringResource(R.string.station_kind_forgotten)
    is StationSpec.Fresh -> stringResource(R.string.station_kind_fresh)
    is StationSpec.DeepCuts -> stringResource(R.string.station_kind_deep_cuts)
    is StationSpec.Menu -> stringResource(R.string.station_kind_menu)
}

@Composable
private fun StationCard(s: Station, playing: Boolean, actions: HomeActions) {
    var confirmDelete by remember { mutableStateOf(false) }
    val start = stringResource(R.string.station_play, s.name)
    Card(
        Modifier.fillMaxWidth().heightIn(min = 96.dp)
            .clickable(role = Role.Button, onClickLabel = start) { actions.onStation(s) },
    ) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(
                if (s.broken != null) Icons.Outlined.WarningAmber else Icons.Outlined.Radio,
                contentDescription = null,
                modifier = Modifier.size(32.dp),
            )
            Column(Modifier.weight(1f).padding(start = 12.dp)) {
                Text(s.name, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                Text(
                    s.broken ?: if (playing) stringResource(R.string.station_playing) else stationKind(s.spec),
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            IconButton(onClick = { confirmDelete = true }) {
                Icon(Icons.Outlined.Delete, stringResource(R.string.station_delete, s.name))
            }
        }
    }
    if (confirmDelete) {
        AlertDialog(
            onDismissRequest = { confirmDelete = false },
            title = { Text(stringResource(R.string.station_delete_title, s.name)) },
            confirmButton = {
                TextButton(onClick = {
                    confirmDelete = false
                    actions.onDeleteStation(s)
                }) { Text(stringResource(R.string.action_delete)) }
            },
            dismissButton = { TextButton(onClick = { confirmDelete = false }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
}

/** The title of a shelf kind (as listed in the editor). */
fun shelfTitle(kind: ShelfKind): Int = when (kind) {
    ShelfKind.RECENT -> R.string.home_shelf_recent
    ShelfKind.STATIONS -> R.string.home_stations
    ShelfKind.NEW -> R.string.home_recent
    ShelfKind.MOST -> R.string.home_shelf_most
    ShelfKind.ON_THIS_DAY -> R.string.home_shelf_on_this_day
    ShelfKind.RANDOM -> R.string.home_random
    ShelfKind.FAVOURITES -> R.string.lib_favourites
    ShelfKind.PLAYLISTS -> R.string.lib_playlists
    ShelfKind.STATION -> R.string.home_shelf_station
    ShelfKind.MENU -> R.string.home_shelf_menu
    ShelfKind.GENRE -> R.string.home_shelf_genre
    ShelfKind.FILTER -> R.string.home_shelf_filter
}

@Composable
private fun shelfName(shelf: HomeShelf, feed: HomeFeed): String {
    val generic = stringResource(shelfTitle(shelf.kind))
    return when (shelf.kind) {
        ShelfKind.STATION -> (feed.stations as? Loadable.Ready)?.value?.firstOrNull { it.id == shelf.id }?.name
            ?.let { stringResource(R.string.home_shelf_named, generic, it) } ?: generic
        ShelfKind.MENU -> feed.menus.firstOrNull { it.id == shelf.id }?.name
            ?.let { stringResource(R.string.home_shelf_named, generic, it) } ?: generic
        ShelfKind.GENRE -> feed.genreNames[shelf.id]?.let { stringResource(R.string.home_shelf_named, generic, it) } ?: generic
        ShelfKind.FILTER -> feed.filters.firstOrNull { it.id == shelf.id }?.name
            ?.let { stringResource(R.string.home_shelf_named, generic, it) } ?: generic
        else -> generic
    }
}

/**
 * Edit mode: each shelf with a drag handle, Move up / Move down, Hide or Show and (for
 * stations, menus and genres) Remove; Add a shelf, Reset and Save. Every move is also an
 * accessibility action, so it works with TalkBack and without dragging.
 */
@Composable
fun HomeEditor(
    draft: HomeLayout,
    feed: HomeFeed,
    edit: HomeEditActions,
    genreChoices: suspend () -> List<Genre>,
    saving: Boolean,
    modifier: Modifier = Modifier,
) {
    var adding by rememberSaveable { mutableStateOf(false) }
    var askDiscard by rememberSaveable { mutableStateOf(false) }
    val changed = draft != (feed.layout ?: HomeLayouts.DEFAULT)
    // Back (and Cancel) leave edit mode, asking first when there are changes to lose.
    val leave = { if (changed) askDiscard = true else edit.onCancel() }
    BackHandler(enabled = !askDiscard) { leave() }
    if (askDiscard) {
        AlertDialog(
            onDismissRequest = { askDiscard = false },
            title = { Text(stringResource(R.string.home_discard_title)) },
            text = { Text(stringResource(R.string.home_discard_body)) },
            confirmButton = {
                TextButton(
                    onClick = {
                        askDiscard = false
                        edit.onCancel()
                    },
                    modifier = Modifier.testTag("home-discard"),
                ) { Text(stringResource(R.string.action_discard)) }
            },
            dismissButton = {
                TextButton(onClick = { askDiscard = false }) { Text(stringResource(R.string.action_keep_editing)) }
            },
        )
    }
    val rowHeight = with(LocalDensity.current) { 64.dp.toPx() }
    Column(modifier.fillMaxSize()) {
        Text(
            stringResource(R.string.home_edit_explained),
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(16.dp),
        )
        LazyColumn(Modifier.weight(1f)) {
            itemsIndexed(draft.shelves, key = { _, s -> "${s.kind}-${s.id}" }) { i, shelf ->
                EditRow(i, draft.shelves.size, shelf, shelfName(shelf, feed), rowHeight, edit)
                HorizontalDivider()
            }
            item {
                TextButton(onClick = { adding = true }, modifier = Modifier.padding(8.dp)) {
                    Icon(Icons.Outlined.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                    Text(stringResource(R.string.home_add_shelf), Modifier.padding(start = 8.dp))
                }
            }
        }
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = edit.onReset) { Text(stringResource(R.string.home_reset)) }
            Box(Modifier.weight(1f))
            OutlinedButton(onClick = leave) { Text(stringResource(R.string.action_cancel)) }
            Button(onClick = edit.onSave, enabled = !saving) { Text(stringResource(R.string.action_save)) }
        }
    }
    if (adding) {
        AddShelfDialog(draft, feed, genreChoices, onAdd = {
            adding = false
            edit.onAdd(it)
        }, onDismiss = { adding = false })
    }
}

@Composable
private fun EditRow(index: Int, count: Int, shelf: HomeShelf, name: String, rowHeight: Float, edit: HomeEditActions) {
    val current by rememberUpdatedState(index)
    var dragged by remember { mutableFloatStateOf(0f) }
    val up = stringResource(R.string.home_move_up, name)
    val down = stringResource(R.string.home_move_down, name)
    val actions = buildList {
        if (index > 0) add(CustomAccessibilityAction(up) { edit.onMove(index, index - 1); true })
        if (index < count - 1) add(CustomAccessibilityAction(down) { edit.onMove(index, index + 1); true })
    }
    ListItem(
        leadingContent = {
            Icon(
                Icons.Outlined.DragHandle,
                contentDescription = stringResource(R.string.home_drag, name),
                modifier = Modifier.pointerInput(shelf.kind, shelf.id) {
                    detectVerticalDragGestures(
                        onDragEnd = { dragged = 0f },
                        onDragCancel = { dragged = 0f },
                    ) { change, dy ->
                        change.consume()
                        dragged += dy
                        val i = current
                        if (dragged > rowHeight / 2 && i < count - 1) {
                            edit.onMove(i, i + 1)
                            dragged -= rowHeight
                        } else if (dragged < -rowHeight / 2 && i > 0) {
                            edit.onMove(i, i - 1)
                            dragged += rowHeight
                        }
                    }
                },
            )
        },
        headlineContent = {
            Text(
                name,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                color = if (shelf.hidden) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            )
        },
        supportingContent = if (shelf.hidden) {
            { Text(stringResource(R.string.home_hidden)) }
        } else {
            null
        },
        trailingContent = {
            Row {
                IconButton(onClick = { edit.onMove(index, index - 1) }, enabled = index > 0) {
                    Icon(Icons.Outlined.ArrowUpward, up)
                }
                IconButton(onClick = { edit.onMove(index, index + 1) }, enabled = index < count - 1) {
                    Icon(Icons.Outlined.ArrowDownward, down)
                }
                IconButton(onClick = { edit.onHidden(index, !shelf.hidden) }) {
                    Icon(
                        if (shelf.hidden) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                        stringResource(if (shelf.hidden) R.string.home_show else R.string.home_hide, name),
                    )
                }
                if (HomeLayouts.takesId(shelf.kind)) {
                    IconButton(onClick = { edit.onRemove(index) }) {
                        Icon(Icons.Outlined.Delete, stringResource(R.string.home_remove, name))
                    }
                }
            }
        },
        modifier = Modifier.semantics { customActions = actions },
    )
}

@Composable
private fun AddShelfDialog(
    draft: HomeLayout,
    feed: HomeFeed,
    genreChoices: suspend () -> List<Genre>,
    onAdd: (HomeShelf) -> Unit,
    onDismiss: () -> Unit,
) {
    val genres = remember { mutableStateListOf<Genre>() }
    var pickGenre by remember { mutableStateOf(false) }
    LaunchedEffect(pickGenre) { if (pickGenre && genres.isEmpty()) genres.addAll(genreChoices()) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.home_add_shelf)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                if (pickGenre) {
                    for (g in genres.filterNot { HomeLayouts.has(draft, ShelfKind.GENRE, it.id) }) {
                        ListItem(
                            headlineContent = { Text(g.name) },
                            modifier = Modifier.clickable { onAdd(HomeShelf(ShelfKind.GENRE, g.id, false)) },
                        )
                    }
                    return@Column
                }
                for (kind in HomeLayouts.addableSingles(draft)) {
                    ListItem(
                        headlineContent = { Text(stringResource(shelfTitle(kind))) },
                        modifier = Modifier.clickable { onAdd(HomeShelf(kind, null, false)) },
                    )
                }
                val stations = (feed.stations as? Loadable.Ready)?.value.orEmpty()
                    .filterNot { HomeLayouts.has(draft, ShelfKind.STATION, it.id) }
                for (s in stations) {
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.Radio, contentDescription = null) },
                        headlineContent = { Text(s.name) },
                        supportingContent = { Text(stringResource(R.string.home_shelf_station)) },
                        modifier = Modifier.clickable { onAdd(HomeShelf(ShelfKind.STATION, s.id, false)) },
                    )
                }
                for (m in feed.menus.filterNot { HomeLayouts.has(draft, ShelfKind.MENU, it.id) }) {
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.PushPin, contentDescription = null) },
                        headlineContent = { Text(m.name) },
                        supportingContent = { Text(stringResource(R.string.home_shelf_menu)) },
                        modifier = Modifier.clickable { onAdd(HomeShelf(ShelfKind.MENU, m.id, false)) },
                    )
                }
                for (f in feed.filters.filterNot { HomeLayouts.has(draft, ShelfKind.FILTER, it.id) }) {
                    ListItem(
                        leadingContent = { Icon(Icons.Outlined.FilterAlt, contentDescription = null) },
                        headlineContent = { Text(f.name) },
                        supportingContent = { Text(stringResource(R.string.home_shelf_filter)) },
                        modifier = Modifier.clickable { onAdd(HomeShelf(ShelfKind.FILTER, f.id, false)) }
                            .testTag("home-add-filter-${f.id}"),
                    )
                }
                ListItem(
                    headlineContent = { Text(stringResource(R.string.home_add_genre)) },
                    modifier = Modifier.clickable { pickGenre = true },
                )
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}

/** The decades offered for decade stations. */
val DECADES = (1950..2020 step 10).toList()

private enum class NewKind { GENRE, DECADE, FORGOTTEN, FRESH, DEEP_CUTS }

/** A new station: genres, a decade, or one of the built-in kinds. */
@Composable
fun NewStationDialog(
    genreChoices: suspend () -> List<Genre>,
    onCreate: (String, StationSpec) -> Unit,
    onDismiss: () -> Unit,
) {
    var kind by rememberSaveable { mutableStateOf(NewKind.GENRE) }
    var name by rememberSaveable { mutableStateOf("") }
    var decade by rememberSaveable { mutableIntStateOf(1980) }
    var mine by rememberSaveable { mutableStateOf(false) }
    val chosen = remember { mutableStateListOf<Genre>() }
    val genres = remember { mutableStateListOf<Genre>() }
    LaunchedEffect(Unit) { genres.addAll(genreChoices()) }
    val defaultName = when (kind) {
        NewKind.GENRE -> chosen.joinToString(" & ") { it.name }
        NewKind.DECADE -> stringResource(R.string.station_kind_decade, decade)
        NewKind.FORGOTTEN -> stringResource(R.string.station_kind_forgotten)
        NewKind.FRESH -> stringResource(R.string.station_kind_fresh)
        NewKind.DEEP_CUTS -> stringResource(R.string.station_kind_deep_cuts)
    }
    val spec: StationSpec? = when (kind) {
        NewKind.GENRE -> if (chosen.isEmpty()) null else StationSpec.Genre(chosen.map { it.id })
        NewKind.DECADE -> StationSpec.Decade(decade.toUInt(), mine)
        NewKind.FORGOTTEN -> StationSpec.Forgotten
        NewKind.FRESH -> StationSpec.Fresh
        NewKind.DEEP_CUTS -> StationSpec.DeepCuts
    }
    val finalName = name.trim().ifEmpty { defaultName }.take(64)
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.station_new)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                val kinds = listOf(
                    NewKind.GENRE to R.string.station_kind_genre,
                    NewKind.DECADE to R.string.station_kind_decade_choice,
                    NewKind.FORGOTTEN to R.string.station_kind_forgotten,
                    NewKind.FRESH to R.string.station_kind_fresh,
                    NewKind.DEEP_CUTS to R.string.station_kind_deep_cuts,
                )
                for ((k, label) in kinds) {
                    Row(
                        Modifier.fillMaxWidth().clickable(role = Role.RadioButton) { kind = k },
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = kind == k, onClick = null)
                        Text(stringResource(label), Modifier.padding(start = 8.dp))
                    }
                }
                when (kind) {
                    NewKind.GENRE -> FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for (g in genres) {
                            val on = chosen.any { it.id == g.id }
                            FilterChip(
                                selected = on,
                                onClick = {
                                    if (on) chosen.removeAll { it.id == g.id } else if (chosen.size < 5) chosen.add(g)
                                },
                                label = { Text(g.name) },
                            )
                        }
                    }
                    NewKind.DECADE -> {
                        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            for (d in DECADES) {
                                FilterChip(selected = decade == d, onClick = { decade = d }, label = { Text("${d}s") })
                            }
                        }
                        Row(
                            Modifier.clickable(role = Role.Checkbox) { mine = !mine },
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Checkbox(checked = mine, onCheckedChange = null)
                            Text(stringResource(R.string.station_decade_mine), Modifier.padding(start = 8.dp))
                        }
                    }
                    else -> {}
                }
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it.take(64) },
                    label = { Text(stringResource(R.string.station_name)) },
                    placeholder = { Text(defaultName) },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { spec?.let { onCreate(finalName, it) } },
                enabled = spec != null && finalName.isNotBlank(),
            ) { Text(stringResource(R.string.action_create)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}
