package io.github.jamezuk.lyrionanywhere.ui.menus

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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.PlaylistAdd
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.Card
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import io.github.jamezuk.lyrionanywhere.core.MenuItem
import io.github.jamezuk.lyrionanywhere.core.MenuItemKind
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Suitability
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage

/** The rows of the menu at the top of [navigator]'s stack. */
private fun LazyListScope.menuRows(frame: MenuFrame, navigator: MenuNavigator, inputs: MutableMap<Int, String>) {
    itemsIndexed(frame.items, key = { i, _ -> "${frame.id}-$i" }) { i, item ->
        MenuRow(item, inputs[i] ?: "", { inputs[i] = it.take(200) }, navigator)
    }
    if (frame.items.isEmpty() && !frame.loading && frame.error == null) {
        item { Text(stringResource(R.string.menu_empty), Modifier.padding(16.dp)) }
    }
    if (frame.error != null && frame.items.isNotEmpty()) {
        item {
            Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(errorMessage(frame.error), textAlign = TextAlign.Center)
                TextButton(onClick = navigator::retry) { Text(stringResource(R.string.action_retry)) }
            }
        }
    }
    if (frame.canLoadMore) {
        item {
            OutlinedButton(onClick = navigator::loadMore, modifier = Modifier.padding(16.dp)) {
                val left = frame.total - frame.items.size
                Text(androidx.compose.ui.platform.LocalResources.current.getQuantityString(R.plurals.menu_show_more, left, left))
            }
        }
    }
}

@Composable
private fun MenuRow(item: MenuItem, input: String, onInput: (String) -> Unit, navigator: MenuNavigator) {
    val transparent = ListItemDefaults.colors(containerColor = Color.Transparent)
    when (item.kind) {
        MenuItemKind.TEXT -> Text(
            item.text,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        )
        MenuItemKind.SEARCH -> Row(
            Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OutlinedTextField(
                value = input,
                onValueChange = onInput,
                label = { Text(item.text.ifEmpty { stringResource(R.string.menu_search) }) },
                singleLine = true,
                leadingIcon = { Icon(Icons.Outlined.Search, contentDescription = null) },
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { navigator.search(item, input) }),
                modifier = Modifier.weight(1f),
            )
            IconButton(onClick = { navigator.search(item, input) }, enabled = input.isNotBlank()) {
                Icon(Icons.Outlined.Search, contentDescription = stringResource(R.string.menu_search_go, item.text))
            }
        }
        else -> {
            val label = item.text.ifEmpty { stringResource(R.string.unknown_track) }
            ListItem(
                colors = transparent,
                leadingContent = item.icon?.let { { ArtworkImage(it, 120, contentDescription = null, modifier = Modifier.size(40.dp), corner = 4.dp) } },
                headlineContent = { Text(label, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                trailingContent = {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        val next = item.playNext
                        val add = item.add
                        val more = item.more
                        if (item.kind == MenuItemKind.PLAYABLE || item.kind == MenuItemKind.FOLDER) {
                            if (next != null) {
                                IconButton(onClick = { navigator.act(item, next, PLAY_NEXT) }) {
                                    Icon(Icons.AutoMirrored.Outlined.QueueMusic, stringResource(R.string.menu_play_next, label))
                                }
                            }
                            if (add != null) {
                                IconButton(onClick = { navigator.act(item, add, ADD) }) {
                                    Icon(Icons.AutoMirrored.Outlined.PlaylistAdd, stringResource(R.string.menu_add, label))
                                }
                            }
                        }
                        if (more != null) {
                            IconButton(onClick = { navigator.act(item, more, MORE) }) {
                                Icon(Icons.Outlined.MoreVert, stringResource(R.string.menu_more, label))
                            }
                        }
                        when (item.kind) {
                            MenuItemKind.FOLDER -> Icon(Icons.Outlined.ChevronRight, contentDescription = null)
                            MenuItemKind.ACTION -> Icon(Icons.Outlined.PlayArrow, contentDescription = null)
                            else -> {}
                        }
                    }
                },
                modifier = Modifier.clickable(
                    onClickLabel = stringResource(
                        when (item.kind) {
                            MenuItemKind.PLAYABLE -> R.string.action_play
                            MenuItemKind.FOLDER -> R.string.menu_open
                            else -> R.string.menu_run
                        },
                    ),
                ) { navigator.choose(item) },
            )
        }
    }
}

private const val PLAY_NEXT = "Play next"
private const val ADD = "Add to queue"
private const val MORE = "More"

/** A full-screen LMS menu (an app, a plugin), with its back stack. */
@Composable
fun MenuBrowser(navigator: MenuNavigator, modifier: Modifier = Modifier) {
    val stack by navigator.stack.collectAsStateWithLifecycle()
    val frame = stack.lastOrNull()
    val inputs = remember(frame?.id) { mutableStateMapOf<Int, String>() }
    when {
        frame == null -> LoadingState(modifier)
        frame.items.isEmpty() && frame.loading -> LoadingState(modifier)
        frame.items.isEmpty() && frame.error != null -> ErrorState(frame.error, navigator::retry, modifier)
        else -> LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
            if (stack.size > 1) {
                item {
                    ListItem(
                        leadingContent = {
                            IconButton(onClick = { navigator.back() }) {
                                Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.action_back))
                            }
                        },
                        headlineContent = {
                            Text(frame.title, style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                        },
                    )
                }
            }
            menuRows(frame, navigator, inputs)
        }
    }
}

/** What a "More…" sheet is for. */
data class SheetTarget(
    val title: String,
    val subtitle: String? = null,
    val artwork: String? = null,
    /** Quick play actions for it (none for things LMS can't enqueue by id). */
    val canPlay: Boolean = true,
)

/**
 * "More…" for a track, album, artist (…): quick Play now / Play next / Add to queue, then
 * LMS's own context menu (including entries plugins add, such as "more like this").
 */
@Composable
fun ItemSheetContent(
    target: SheetTarget,
    navigator: MenuNavigator,
    onPlay: (Placement) -> Unit,
    onClose: () -> Unit,
    /** Download it (offline); null where downloads aren't offered for this item. */
    onDownload: (() -> Unit)? = null,
    /** "Start radio" and "More like this" for it (A6), when the server offers them. */
    discovery: io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryUiState? = null,
    onStartRadio: () -> Unit = {},
    onSimilar: (MenuItem) -> Unit = {},
) {
    val stack by navigator.stack.collectAsStateWithLifecycle()
    val frame = stack.lastOrNull()
    val inputs = remember(frame?.id) { mutableStateMapOf<Int, String>() }
    LazyColumn(Modifier.fillMaxWidth(), contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            ListItem(
                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                leadingContent = when {
                    stack.size > 1 -> {
                        { IconButton(onClick = { navigator.back() }) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.action_back)) } }
                    }
                    target.artwork != null -> {
                        { ArtworkImage(target.artwork, 120, contentDescription = null, modifier = Modifier.size(48.dp), corner = 6.dp) }
                    }
                    else -> null
                },
                headlineContent = {
                    Text(
                        if (stack.size > 1) frame?.title.orEmpty() else target.title,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.semantics { heading() },
                    )
                },
                supportingContent = target.subtitle?.takeIf { stack.size <= 1 }?.let { { Text(it, maxLines = 1) } },
                trailingContent = {
                    IconButton(onClick = onClose) { Icon(Icons.Outlined.Close, stringResource(R.string.action_close)) }
                },
            )
        }
        if (stack.size <= 1 && target.canPlay) {
            item {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilledTonalButton(onClick = { onPlay(Placement.PLAY_NOW) }, modifier = Modifier.weight(1f)) {
                        Text(stringResource(R.string.action_play_now), maxLines = 1)
                    }
                    OutlinedButton(onClick = { onPlay(Placement.PLAY_NEXT) }, modifier = Modifier.weight(1f)) {
                        Text(stringResource(R.string.action_play_next), maxLines = 1)
                    }
                    OutlinedButton(onClick = { onPlay(Placement.ADD_TO_QUEUE) }, modifier = Modifier.weight(1f)) {
                        Text(stringResource(R.string.action_add_short), maxLines = 1)
                    }
                }
                if (onDownload != null) {
                    OutlinedButton(
                        onClick = onDownload,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                    ) {
                        Icon(Icons.Outlined.Download, contentDescription = null)
                        Text(stringResource(R.string.action_download), Modifier.padding(start = 8.dp), maxLines = 1)
                    }
                }
                if (discovery != null) {
                    io.github.jamezuk.lyrionanywhere.ui.discovery.StartRadioButton(discovery, onStartRadio)
                    io.github.jamezuk.lyrionanywhere.ui.discovery.MoreLikeThisShelf(discovery, onSimilar)
                }
                HorizontalDivider(Modifier.padding(top = 8.dp))
            }
        }
        when {
            frame == null || (frame.items.isEmpty() && frame.loading) -> item {
                Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { LoadingState(Modifier.size(48.dp)) }
            }
            frame.items.isEmpty() && frame.error != null -> item {
                Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(errorMessage(frame.error), textAlign = TextAlign.Center)
                    TextButton(onClick = navigator::retry) { Text(stringResource(R.string.action_retry)) }
                }
            }
            else -> menuRows(frame, navigator, inputs)
        }
    }
}

/** Apps & Radio: tiles, with unusable ones tucked away (see [AppsViewModel]). */
@Composable
fun AppsScreen(
    state: AppsUiState,
    onOpen: (AppTile) -> Unit,
    onRetry: () -> Unit,
    onToggleShowAll: () -> Unit,
    modifier: Modifier = Modifier,
) {
    when {
        state.loading -> LoadingState(modifier)
        state.error != null -> ErrorState(state.error, onRetry, modifier)
        state.empty -> EmptyState(stringResource(R.string.apps_none), modifier)
        else -> LazyVerticalGrid(
            columns = GridCells.Adaptive(140.dp),
            contentPadding = PaddingValues(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            modifier = modifier.fillMaxSize(),
        ) {
            if (state.noPlayer) {
                item(span = { GridItemSpan(maxLineSpan) }) { Text(stringResource(R.string.apps_need_player)) }
            }
            for ((title, list) in listOf(
                R.string.apps_radio to state.radios,
                R.string.apps_apps to state.apps,
                R.string.apps_plugins to state.plugins,
            )) {
                val shown = state.shown(list)
                if (shown.isEmpty()) continue
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Text(stringResource(title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                }
                items(shown, key = { it.key }) { tile -> AppTileCard(tile, onClick = { onOpen(tile) }) }
            }
            if (state.hidden > 0 || state.showAll) {
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Column {
                        Text(
                            if (state.showAll) {
                                stringResource(R.string.apps_showing_all)
                            } else {
                                androidx.compose.ui.platform.LocalResources.current.getQuantityString(R.plurals.apps_hidden, state.hidden, state.hidden)
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                        TextButton(onClick = onToggleShowAll) {
                            Text(stringResource(if (state.showAll) R.string.apps_hide_unusable else R.string.apps_show_all))
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AppTileCard(tile: AppTile, onClick: () -> Unit) {
    val note = when (tile.fit) {
        is Suitability.NeedsSetup -> stringResource(R.string.apps_needs_setup)
        is Suitability.Failed, Suitability.Empty -> stringResource(R.string.apps_not_usable)
        else -> null
    }
    Card(onClick = onClick, modifier = Modifier.fillMaxWidth()) {
        Column(Modifier.padding(12.dp), horizontalAlignment = Alignment.CenterHorizontally) {
            ArtworkImage(tile.icon, 160, contentDescription = null, modifier = Modifier.size(64.dp), corner = 12.dp)
            Text(
                tile.name,
                style = MaterialTheme.typography.titleSmall,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(top = 8.dp),
            )
            if (note != null) {
                Text(note, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center)
            }
        }
    }
}
