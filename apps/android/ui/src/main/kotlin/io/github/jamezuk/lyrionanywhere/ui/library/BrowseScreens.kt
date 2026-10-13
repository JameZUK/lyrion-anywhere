package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.PlaylistAdd
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.outlined.Album
import androidx.compose.material.icons.outlined.CalendarMonth
import androidx.compose.material.icons.outlined.DownloadDone
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Favorite
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.LibraryMusic
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.NewReleases
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Radio
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material.icons.outlined.Style
import androidx.compose.material.icons.outlined.ViewModule
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.SecondaryTabRow
import androidx.compose.material3.Tab
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.Favourite
import io.github.jamezuk.lyrionanywhere.core.FavouritePage
import io.github.jamezuk.lyrionanywhere.core.FolderEntry
import io.github.jamezuk.lyrionanywhere.core.FolderKind
import io.github.jamezuk.lyrionanywhere.core.Genre
import io.github.jamezuk.lyrionanywhere.core.PinnedMenu
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.Station
import io.github.jamezuk.lyrionanywhere.core.StationSpec
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.VirtualLibrary
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage
import io.github.jamezuk.lyrionanywhere.ui.menus.More
import io.github.jamezuk.lyrionanywhere.ui.menus.MoreRequest

/** Where the Library page leads. */
enum class LibraryDestination(val label: Int, val icon: ImageVector) {
    ALBUMS(R.string.lib_albums, Icons.Outlined.Album),
    ARTISTS(R.string.lib_artists, Icons.Outlined.Person),
    GENRES(R.string.lib_genres, Icons.Outlined.Style),
    YEARS(R.string.lib_years, Icons.Outlined.CalendarMonth),
    COMPILATIONS(R.string.lib_compilations, Icons.Outlined.ViewModule),
    RANDOM(R.string.lib_random, Icons.Outlined.Shuffle),
    FOLDER(R.string.lib_folder, Icons.Outlined.Folder),
    PLAYLISTS(R.string.lib_playlists, Icons.AutoMirrored.Outlined.QueueMusic),
    FAVOURITES(R.string.lib_favourites, Icons.Outlined.Favorite),
    APPS(R.string.lib_apps, Icons.Outlined.Radio),

    /** Music downloaded to this phone (offline). */
    DOWNLOADS(R.string.title_downloads, Icons.Outlined.DownloadDone),
}

private val BROWSE = listOf(
    LibraryDestination.ALBUMS,
    LibraryDestination.ARTISTS,
    LibraryDestination.GENRES,
    LibraryDestination.YEARS,
    LibraryDestination.COMPILATIONS,
    LibraryDestination.RANDOM,
    LibraryDestination.FOLDER,
)
private val MORE = listOf(
    LibraryDestination.PLAYLISTS,
    LibraryDestination.FAVOURITES,
    LibraryDestination.APPS,
    LibraryDestination.DOWNLOADS,
)

/**
 * The Library page, as the web app's: New Music first, then LMS's "My Music", then the
 * app's other collections; a virtual-library picker only when LMS has libraries.
 */
@Composable
fun LibraryScreen(
    libraries: List<VirtualLibrary>,
    chosenLibrary: String?,
    onChooseLibrary: (String?) -> Unit,
    onNewMusic: () -> Unit,
    onOpen: (LibraryDestination) -> Unit,
    modifier: Modifier = Modifier,
    /** The type chip chosen (Wave 2 §3.2); null: the overview below. */
    chip: io.github.jamezuk.lyrionanywhere.data.LibraryChip? = null,
    onChip: ((io.github.jamezuk.lyrionanywhere.data.LibraryChip) -> Unit)? = null,
    /** What a chip shows. */
    chipContent: @Composable (io.github.jamezuk.lyrionanywhere.data.LibraryChip) -> Unit = {},
) {
    if (chip != null && onChip != null) {
        Column(modifier.fillMaxSize()) {
            LibraryChips(chip, onChip, Modifier.padding(vertical = 4.dp))
            Box(Modifier.weight(1f)) { chipContent(chip) }
        }
        return
    }
    LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
        if (onChip != null) {
            item { LibraryChips(null, onChip, Modifier.padding(vertical = 4.dp)) }
        }
        if (libraries.isNotEmpty()) {
            item { LibraryPicker(libraries, chosenLibrary, onChooseLibrary) }
        }
        item {
            Card(
                onClick = onNewMusic,
                colors = CardDefaults.cardColors(
                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                    contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                ),
                modifier = Modifier.fillMaxWidth().padding(16.dp),
            ) {
                Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Outlined.NewReleases, contentDescription = null, modifier = Modifier.size(32.dp))
                    Column(Modifier.weight(1f).padding(horizontal = 16.dp)) {
                        Text(stringResource(R.string.new_music), style = MaterialTheme.typography.titleLarge)
                        Text(stringResource(R.string.new_music_detail), style = MaterialTheme.typography.bodyMedium)
                    }
                    Icon(Icons.Outlined.ChevronRight, contentDescription = null)
                }
            }
        }
        items(BROWSE) { d -> DestinationRow(d, onOpen) }
        item { SectionHeader(stringResource(R.string.section_more)) }
        items(MORE) { d -> DestinationRow(d, onOpen) }
    }
}

@Composable
private fun DestinationRow(d: LibraryDestination, onOpen: (LibraryDestination) -> Unit) {
    ListItem(
        leadingContent = { Icon(d.icon, contentDescription = null) },
        headlineContent = { Text(stringResource(d.label)) },
        trailingContent = { Icon(Icons.Outlined.ChevronRight, contentDescription = null) },
        modifier = Modifier.clickable { onOpen(d) },
    )
}

@Composable
private fun LibraryPicker(libraries: List<VirtualLibrary>, chosen: String?, onChoose: (String?) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val name = libraries.firstOrNull { it.id == chosen }?.name ?: stringResource(R.string.whole_library)
    Box(Modifier.padding(horizontal = 16.dp, vertical = 8.dp)) {
        ListItem(
            leadingContent = { Icon(Icons.Outlined.LibraryMusic, contentDescription = null) },
            overlineContent = { Text(stringResource(R.string.library_picker)) },
            headlineContent = { Text(name) },
            modifier = Modifier.clickable(onClickLabel = stringResource(R.string.library_choose)) { open = true },
        )
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(text = { Text(stringResource(R.string.whole_library)) }, onClick = {
                open = false
                onChoose(null)
            })
            libraries.forEach { l ->
                DropdownMenuItem(text = { Text(l.name) }, onClick = {
                    open = false
                    onChoose(l.id)
                })
            }
        }
    }
}

@Composable
fun artistViewLabel(v: ArtistView): String = stringResource(
    when (v) {
        ArtistView.ALBUM_ARTISTS -> R.string.artists_album_artists
        ArtistView.ALL -> R.string.artists_all
        ArtistView.COMPOSERS -> R.string.artists_composers
        ArtistView.CONDUCTORS -> R.string.artists_conductors
        ArtistView.BANDS -> R.string.artists_bands
    },
)

/** The artist views as chips: only those the library has (as the web app). */
@Composable
fun ArtistViewChips(views: List<ArtistView>, selected: ArtistView, onSelect: (ArtistView) -> Unit) {
    if (views.size < 2) return
    Row(
        Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(horizontal = 12.dp),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        views.forEach { v ->
            FilterChip(selected = v == selected, onClick = { onSelect(v) }, label = { Text(artistViewLabel(v)) })
        }
    }
}

/** A paged list with loading, empty and error states. */
@Composable
fun <T> PagedList(
    state: PagedState<T>,
    empty: String,
    onLoadMore: () -> Unit,
    modifier: Modifier = Modifier,
    key: (Int, T) -> Any,
    header: (@Composable () -> Unit)? = null,
    row: @Composable (T) -> Unit,
) {
    when {
        state.failedEmpty && header == null -> ErrorState(state.error!!, onLoadMore, modifier)
        state.items.isEmpty() && state.loading && header == null -> LoadingState(modifier)
        state.items.isEmpty() && state.endReached && header == null -> EmptyState(empty, modifier)
        else -> {
            val list = rememberLazyListState()
            LoadMoreWhenNear(
                lastVisible = { list.layoutInfo.visibleItemsInfo.lastOrNull()?.index ?: 0 },
                total = { state.items.size },
                onLoadMore = onLoadMore,
            )
            LazyColumn(state = list, modifier = modifier.fillMaxSize()) {
                if (header != null) item { header() }
                itemsIndexed(state.items, key = key) { _, x -> row(x) }
                if (header != null && state.items.isEmpty() && state.endReached && state.error == null) {
                    item { Text(empty, Modifier.padding(16.dp)) }
                }
                if (header != null && state.failedEmpty) {
                    item { Text(errorMessage(state.error!!), Modifier.padding(16.dp)) }
                }
                if (!state.endReached || state.error != null) item { PageFooter(state, onLoadMore) }
            }
        }
    }
}

/** Genres, with a search box. */
@Composable
fun GenresScreen(
    state: PagedState<Genre>,
    query: String,
    onQuery: (String) -> Unit,
    onGenre: (Genre) -> Unit,
    onLoadMore: () -> Unit,
    onMore: (MoreRequest) -> Unit,
    modifier: Modifier = Modifier,
) {
    val focusManager = LocalFocusManager.current
    Column(modifier) {
        OutlinedTextField(
            value = query,
            onValueChange = onQuery,
            label = { Text(stringResource(R.string.genres_search)) },
            singleLine = true,
            leadingIcon = { Icon(Icons.Outlined.Search, contentDescription = null) },
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            // The list filters as you type: Search just closes the keyboard.
            keyboardActions = KeyboardActions(onSearch = { focusManager.clearFocus() }),
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
        )
        PagedList(state, stringResource(R.string.no_genres), onLoadMore, key = { i, g -> "${g.id}-$i" }) { g ->
            ListItem(
                headlineContent = { Text(g.name) },
                trailingContent = {
                    IconButton(onClick = { onMore(More.genre(g.id, g.name)) }) {
                        Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, g.name))
                    }
                },
                modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_genre)) { onGenre(g) },
            )
        }
    }
}

/** A genre or year page's top: title, Play all / Add / More, and (for genres) the tabs. */
@Composable
fun CollectionHeader(
    title: String,
    onPlay: (Placement) -> Unit,
    onMore: () -> Unit,
    tabs: List<String> = emptyList(),
    tab: Int = 0,
    onTab: (Int) -> Unit = {},
) {
    Column(Modifier.fillMaxWidth()) {
        Text(
            title,
            style = MaterialTheme.typography.headlineSmall,
            modifier = Modifier.padding(start = 16.dp, top = 8.dp).semantics { heading() },
        )
        PlayButtons(
            onPlay = onPlay,
            onShuffle = null,
            onMore = onMore,
            playLabel = R.string.action_play_all,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
        if (tabs.isNotEmpty()) {
            SecondaryTabRow(selectedTabIndex = tab) {
                tabs.forEachIndexed { i, t -> Tab(selected = i == tab, onClick = { onTab(i) }, text = { Text(t) }) }
            }
        }
    }
}

@Composable
fun YearsScreen(
    state: PagedState<UInt>,
    onYear: (UInt) -> Unit,
    onLoadMore: () -> Unit,
    onMore: (MoreRequest) -> Unit,
    modifier: Modifier = Modifier,
) {
    val noYear = stringResource(R.string.no_year)
    PagedList(state, stringResource(R.string.no_years), onLoadMore, modifier, key = { i, y -> "$y-$i" }) { y ->
        val label = if (y == 0u) noYear else y.toString()
        ListItem(
            headlineContent = { Text(label) },
            trailingContent = {
                IconButton(onClick = { onMore(More.year(y, label)) }) {
                    Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, label))
                }
            },
            modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_year)) { onYear(y) },
        )
    }
}

/** A folder of the music folder: folders open, tracks play; both can be played or added. */
@Composable
fun FolderScreen(
    state: PagedState<FolderEntry>,
    isTop: Boolean,
    onFolder: (FolderEntry) -> Unit,
    onPlay: (FolderEntry, Placement) -> Unit,
    onPlayAll: ((Placement) -> Unit)?,
    onLoadMore: () -> Unit,
    onMore: (MoreRequest) -> Unit,
    modifier: Modifier = Modifier,
) {
    val untitled = stringResource(R.string.untitled)
    PagedList(
        state,
        stringResource(R.string.folder_empty),
        onLoadMore,
        modifier,
        key = { i, f -> "${f.id}-$i" },
        header = if (!isTop && onPlayAll != null && state.items.any { it.kind != FolderKind.OTHER }) {
            { PlayButtons(onPlay = onPlayAll, onShuffle = null, onMore = null, playLabel = R.string.action_play_all, modifier = Modifier.padding(16.dp)) }
        } else {
            null
        },
    ) { f ->
        val name = f.name.ifEmpty { untitled }
        val folder = f.kind == FolderKind.FOLDER
        ListItem(
            leadingContent = {
                if (f.kind == FolderKind.TRACK) {
                    ArtworkImage(f.artwork, 120, contentDescription = null, modifier = Modifier.size(40.dp), corner = 4.dp)
                } else {
                    Icon(if (folder) Icons.Outlined.Folder else Icons.AutoMirrored.Outlined.QueueMusic, contentDescription = null)
                }
            },
            headlineContent = { Text(name, maxLines = 2, overflow = TextOverflow.Ellipsis) },
            supportingContent = f.duration?.let { { Text(it) } },
            trailingContent = {
                Row {
                    if (folder) {
                        IconButton(onClick = { onPlay(f, Placement.PLAY_NOW) }) {
                            Icon(Icons.Outlined.PlayArrow, stringResource(R.string.play_named, name))
                        }
                    }
                    if (f.kind != FolderKind.OTHER) {
                        IconButton(onClick = { onPlay(f, Placement.ADD_TO_QUEUE) }) {
                            Icon(Icons.AutoMirrored.Outlined.PlaylistAdd, stringResource(R.string.add_named, name))
                        }
                        IconButton(onClick = { onMore(More.folder(f)) }) {
                            Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, name))
                        }
                    }
                }
            },
            modifier = Modifier.clickable(
                enabled = f.kind != FolderKind.OTHER,
                onClickLabel = stringResource(if (folder) R.string.open_folder else R.string.action_play),
            ) { if (folder) onFolder(f) else onPlay(f, Placement.PLAY_NOW) },
        )
    }
}

@Composable
fun PlaylistsScreen(
    state: PagedState<SavedPlaylist>,
    onPlaylist: (SavedPlaylist) -> Unit,
    onLoadMore: () -> Unit,
    onMore: (MoreRequest) -> Unit,
    modifier: Modifier = Modifier,
) {
    PagedList(state, stringResource(R.string.no_playlists), onLoadMore, modifier, key = { i, p -> "${p.id}-$i" }) { p ->
        ListItem(
            leadingContent = { Icon(Icons.AutoMirrored.Outlined.QueueMusic, contentDescription = null) },
            headlineContent = { Text(p.name) },
            trailingContent = {
                IconButton(onClick = { onMore(More.playlist(p)) }) {
                    Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, p.name))
                }
            },
            modifier = Modifier.clickable(onClickLabel = stringResource(R.string.open_playlist)) { onPlaylist(p) },
        )
    }
}

/** A saved playlist: Play/Shuffle/Add/More, then its tracks. */
@Composable
fun PlaylistScreen(
    state: Loadable<List<Track>>,
    onRetry: () -> Unit,
    onPlay: (Placement, Int?) -> Unit,
    onShuffle: () -> Unit,
    onMorePlaylist: () -> Unit,
    onMore: (MoreRequest) -> Unit,
    modifier: Modifier = Modifier,
    /** Download the playlist (offline); null where downloads aren't offered. */
    onDownload: (() -> Unit)? = null,
    downloaded: Set<Long> = emptySet(),
    /** Play a track next or add it to the queue (its row's swipes); null: no swipes. */
    onQueueTrack: ((Track, Placement) -> Unit)? = null,
) {
    when (state) {
        Loadable.Loading -> LoadingState(modifier)
        is Loadable.Failed -> ErrorState(state.error, onRetry, modifier)
        is Loadable.Ready -> LazyColumn(modifier.fillMaxSize()) {
            item {
                PlayButtons(
                    onPlay = { onPlay(it, null) },
                    onShuffle = onShuffle,
                    onMore = onMorePlaylist,
                    modifier = Modifier.padding(16.dp),
                    onDownload = onDownload,
                    downloadMark = io.github.jamezuk.lyrionanywhere.ui.offline.downloadMark(
                        state.value.mapNotNull { it.id },
                        downloaded,
                    ),
                )
            }
            if (state.value.isEmpty()) item { Text(stringResource(R.string.playlist_empty), Modifier.padding(16.dp)) }
            itemsIndexed(state.value, key = { i, t -> "$i-${t.id}" }) { i, t ->
                val queue = onQueueTrack?.takeIf { t.id != null }
                BrowseSwipe(queue?.let { q -> { p: Placement -> q(t, p) } }) { m ->
                    TrackRow(
                        t,
                        onClick = { onPlay(Placement.PLAY_NOW, i) },
                        showArt = true,
                        onMore = More.track(t)?.let { r -> { onMore(r) } },
                        modifier = m,
                    )
                }
            }
        }
    }
}

/** Favourites: folders open; anything else plays (now, next or at the end). */
@Composable
fun FavouritesScreen(
    state: Loadable<FavouritePage>,
    onRetry: () -> Unit,
    onFolder: (Favourite) -> Unit,
    onPlay: (Favourite, Placement) -> Unit,
    modifier: Modifier = Modifier,
) {
    when (state) {
        Loadable.Loading -> LoadingState(modifier)
        is Loadable.Failed -> ErrorState(state.error, onRetry, modifier)
        is Loadable.Ready -> {
            val items = state.value.items
            if (items.isEmpty()) {
                EmptyState(stringResource(R.string.no_favourites), modifier)
                return
            }
            LazyColumn(modifier.fillMaxSize()) {
                items(items, key = { it.id }) { f ->
                    ListItem(
                        leadingContent = {
                            ArtworkImage(f.artwork, 120, contentDescription = null, modifier = Modifier.size(40.dp), corner = 4.dp)
                        },
                        headlineContent = { Text(f.name, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                        trailingContent = if (f.isFolder) {
                            { Icon(Icons.Outlined.ChevronRight, contentDescription = null) }
                        } else {
                            {
                                Row {
                                    IconButton(onClick = { onPlay(f, Placement.PLAY_NEXT) }) {
                                        Icon(Icons.AutoMirrored.Outlined.QueueMusic, stringResource(R.string.menu_play_next, f.name))
                                    }
                                    IconButton(onClick = { onPlay(f, Placement.ADD_TO_QUEUE) }) {
                                        Icon(Icons.AutoMirrored.Outlined.PlaylistAdd, stringResource(R.string.add_named, f.name))
                                    }
                                }
                            }
                        },
                        modifier = Modifier.clickable(
                            onClickLabel = stringResource(if (f.isFolder) R.string.open_folder else R.string.action_play),
                        ) { if (f.isFolder) onFolder(f) else onPlay(f, Placement.PLAY_NOW) },
                    )
                }
            }
        }
    }
}

/** What Home can do. */
data class HomeActions(
    val onAlbum: (Album) -> Unit,
    val onMore: (MoreRequest) -> Unit,
    val onSeeAll: (LibraryDestination?) -> Unit,
    val onFavourite: (Favourite) -> Unit,
    val onPlaylist: (SavedPlaylist) -> Unit,
    val onReshuffle: () -> Unit,
    val onRetry: () -> Unit,
    /** Start a station on the current player. */
    val onStation: (Station) -> Unit = {},
    val onCreateStation: (String, StationSpec) -> Unit = { _, _ -> },
    val onDeleteStation: (Station) -> Unit = {},
    /** Open a pinned menu. */
    val onMenu: (PinnedMenu) -> Unit = {},
    /** All albums of a genre shelf. */
    val onGenre: (Long, String) -> Unit = { _, _ -> },
    /** Enter edit mode. */
    val onEdit: () -> Unit = {},
    /** Genres to choose from (new stations, genre shelves). */
    val genreChoices: suspend () -> List<Genre> = { emptyList() },
)

/**
 * A carousel card's width for this screen: about two and a half cards across a phone, more
 * on wider screens, never tiny or huge (pure, so it is unit-tested).
 */
internal fun carouselCardWidth(screenWidthDp: Int, small: Boolean = false): Float {
    val across = (screenWidthDp - 32f) / 2.4f - 12f
    val w = if (small) across * 0.8f else across
    return if (small) w.coerceIn(96f, 150f) else w.coerceIn(112f, 190f)
}

@Composable
private fun cardWidth(small: Boolean = false): Dp {
    // The window's own width (split screen, foldables), not the whole display's.
    val widthDp = with(LocalDensity.current) { LocalWindowInfo.current.containerSize.width.toDp().value.toInt() }
    return carouselCardWidth(widthDp, small).dp
}

@Composable
internal fun AlbumCard(a: Album, actions: HomeActions) {
    Box(Modifier.width(cardWidth())) {
        AlbumTile(a, onClick = { actions.onAlbum(a) }, onMore = { actions.onMore(More.album(a)) })
    }
}

@Composable
internal fun SmallCard(name: String, artwork: String?, icon: ImageVector, onClick: () -> Unit) {
    Column(
        Modifier.width(cardWidth(small = true)).clickable(role = Role.Button, onClick = onClick),
    ) {
        if (artwork != null) {
            ArtworkImage(artwork, 240, contentDescription = null, modifier = Modifier.fillMaxWidth())
        } else {
            Card(Modifier.fillMaxWidth().aspectRatio(1f)) {
                Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                    Icon(icon, contentDescription = null, modifier = Modifier.size(40.dp))
                }
            }
        }
        Text(name, style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(top = 4.dp))
    }
}

/** A titled horizontal row, with "See all" and its own loading and error states. */
@Composable
fun <T> Carousel(
    title: String,
    state: Loadable<List<T>>,
    onSeeAll: () -> Unit,
    onRetry: () -> Unit,
    hideWhenEmpty: Boolean = false,
    extra: @Composable () -> Unit = {},
    showSeeAll: Boolean = true,
    card: @Composable (T) -> Unit,
) {
    if (hideWhenEmpty && (state as? Loadable.Ready)?.value?.isEmpty() == true) return
    Column(Modifier.fillMaxWidth().padding(top = 8.dp)) {
        Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            Text(
                title,
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
            extra()
            if (showSeeAll) {
                val seeAll = stringResource(R.string.see_all_named, title)
                TextButton(onClick = onSeeAll, modifier = Modifier.semantics { contentDescription = seeAll }) {
                    Text(stringResource(R.string.see_all))
                }
            }
        }
        when (state) {
            Loadable.Loading -> Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) {
                androidx.compose.material3.CircularProgressIndicator(Modifier.size(24.dp))
            }
            is Loadable.Failed -> Row(Modifier.padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    errorMessage(state.error),
                    Modifier.weight(1f).semantics { liveRegion = LiveRegionMode.Polite },
                    style = MaterialTheme.typography.bodyMedium,
                )
                OutlinedButton(onClick = onRetry) { Text(stringResource(R.string.action_retry)) }
            }
            is Loadable.Ready -> if (state.value.isEmpty()) {
                Text(stringResource(R.string.nothing_here), Modifier.padding(horizontal = 16.dp))
            } else {
                LazyRow(
                    contentPadding = PaddingValues(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    items(state.value.size) { i -> card(state.value[i]) }
                }
            }
        }
    }
}
