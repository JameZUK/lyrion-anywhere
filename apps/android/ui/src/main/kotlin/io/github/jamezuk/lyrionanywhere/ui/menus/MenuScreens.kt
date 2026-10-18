package io.github.jamezuk.lyrionanywhere.ui.menus

import androidx.compose.foundation.clickable
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
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Radio
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
import androidx.activity.compose.BackHandler
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
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
        MenuRow(item, i, inputs[i] ?: "", { inputs[i] = it.take(200) }, navigator)
    }
    if (frame.items.isEmpty() && !frame.loading && frame.error == null) {
        item { Text(stringResource(R.string.menu_empty), Modifier.padding(16.dp)) }
    }
    if (frame.error != null && frame.items.isNotEmpty()) {
        item {
            Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text(
                    errorMessage(frame.error),
                    textAlign = TextAlign.Center,
                    modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                )
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
private fun MenuRow(item: MenuItem, index: Int, input: String, onInput: (String) -> Unit, navigator: MenuNavigator) {
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
                // One trailing control (Wave 3.5 rule 3): ⋮ with what the item offers (Play next,
                // Add to queue, Save as station or Pin to Home, More options), each also a
                // TalkBack action; a folder's chevron or an action's ▷ say what a tap does.
                trailingContent = {
                    val next = item.playNext
                    val add = item.add
                    val more = item.more
                    val playable = item.kind == MenuItemKind.PLAYABLE || item.kind == MenuItemKind.FOLDER
                    val save = navigator.canSave(item)
                    val entries = buildList<Triple<String, ImageVector, () -> Unit>> {
                        if (playable && next != null) {
                            add(Triple(stringResource(R.string.menu_play_next, label), Icons.AutoMirrored.Outlined.QueueMusic) { navigator.act(item, next, MenuVerb.PLAY_NEXT) })
                        }
                        if (playable && add != null) {
                            add(Triple(stringResource(R.string.menu_add, label), Icons.AutoMirrored.Outlined.PlaylistAdd) { navigator.act(item, add, MenuVerb.ADD) })
                        }
                        when (save) {
                            SaveKind.STATION -> add(Triple(stringResource(R.string.menu_save_station, label), Icons.Outlined.Radio) { navigator.save(item, index, SaveKind.STATION) })
                            SaveKind.PIN -> add(Triple(stringResource(R.string.menu_pin, label), Icons.Outlined.PushPin) { navigator.save(item, index, SaveKind.PIN) })
                            null -> {}
                        }
                        if (more != null) {
                            add(Triple(stringResource(R.string.menu_more, label), Icons.Outlined.MoreVert) { navigator.act(item, more, MenuVerb.MORE) })
                        }
                    }
                    when {
                        entries.size == 1 && more != null -> IconButton(onClick = entries.single().third) {
                            Icon(Icons.Outlined.MoreVert, entries.single().first)
                        }
                        entries.isNotEmpty() -> {
                            var open by remember { mutableStateOf(false) }
                            Box {
                                IconButton(onClick = { open = true }, modifier = Modifier.testTag("menu-row-more-$index")) {
                                    Icon(Icons.Outlined.MoreVert, stringResource(R.string.more_for, label))
                                }
                                DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
                                    entries.forEach { (text, icon, act) ->
                                        DropdownMenuItem(
                                            text = { Text(text) },
                                            leadingIcon = { Icon(icon, contentDescription = null) },
                                            onClick = {
                                                open = false
                                                act()
                                            },
                                        )
                                    }
                                }
                            }
                        }
                        item.kind == MenuItemKind.FOLDER -> Icon(Icons.Outlined.ChevronRight, contentDescription = null)
                        item.kind == MenuItemKind.ACTION -> Icon(Icons.Outlined.PlayArrow, contentDescription = null)
                        else -> {}
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

/** A menu action's name when its item has none. */
@Composable
fun menuVerbText(verb: MenuVerb): String = stringResource(verbRes(verb))

/** What a menu action is called, outside composition (a snackbar). */
fun menuLabelText(res: android.content.res.Resources, label: MenuLabel): String = when (label) {
    is MenuLabel.Named -> label.text
    is MenuLabel.Verb -> res.getString(verbRes(label.verb))
}

private fun verbRes(verb: MenuVerb): Int = when (verb) {
    MenuVerb.PLAY -> R.string.menu_verb_play
    MenuVerb.OPEN -> R.string.menu_verb_open
    MenuVerb.SEARCH -> R.string.menu_verb_search
    MenuVerb.PLAY_NEXT -> R.string.menu_verb_play_next
    MenuVerb.ADD -> R.string.menu_verb_add
    MenuVerb.MORE -> R.string.menu_verb_more
}

/** A frame's title: LMS's or the item's, else the action that opened it. */
@Composable
fun frameTitle(frame: MenuFrame): String = frame.title.ifEmpty { frame.titleVerb?.let { menuVerbText(it) }.orEmpty() }

/** A full-screen LMS menu (an app, a plugin), with its back stack. */
@Composable
fun MenuBrowser(navigator: MenuNavigator, modifier: Modifier = Modifier) {
    val stack by navigator.stack.collectAsStateWithLifecycle()
    val frame = stack.lastOrNull()
    val inputs = remember(frame?.id) { mutableStateMapOf<Int, String>() }
    // System back goes up a menu level first; at the first level it leaves as before.
    BackHandler(enabled = stack.size > 1) { navigator.back() }
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
                            Text(frameTitle(frame), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
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
    /** A track's credits (Wave 2 §3.1), each followable to that person's albums in that role. */
    credits: List<io.github.jamezuk.lyrionanywhere.core.Credit> = emptyList(),
    onCredit: (io.github.jamezuk.lyrionanywhere.core.Credit) -> Unit = {},
) {
    val stack by navigator.stack.collectAsStateWithLifecycle()
    val frame = stack.lastOrNull()
    val inputs = remember(frame?.id) { mutableStateMapOf<Int, String>() }
    // System back goes up a menu level first; at the first level it leaves as before.
    BackHandler(enabled = stack.size > 1) { navigator.back() }
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
                        if (stack.size > 1) frame?.let { frameTitle(it) }.orEmpty() else target.title,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.semantics { heading() },
                    )
                },
                supportingContent = target.subtitle?.takeIf { stack.size <= 1 }?.let {
                    { Text(it, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                },
                trailingContent = {
                    IconButton(onClick = onClose) { Icon(Icons.Outlined.Close, stringResource(R.string.action_close)) }
                },
            )
        }
        if (stack.size <= 1 && target.canPlay) {
            item {
                // Wraps onto more lines with large text, rather than squeezing the labels.
                FlowRow(
                    Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    FilledTonalButton(onClick = { onPlay(Placement.PLAY_NOW) }) {
                        Text(stringResource(R.string.action_play_now))
                    }
                    OutlinedButton(onClick = { onPlay(Placement.PLAY_NEXT) }) {
                        Text(stringResource(R.string.action_play_next))
                    }
                    OutlinedButton(onClick = { onPlay(Placement.ADD_TO_QUEUE) }) {
                        Text(stringResource(R.string.action_add_short))
                    }
                }
                if (onDownload != null) {
                    OutlinedButton(
                        onClick = onDownload,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp),
                    ) {
                        Icon(Icons.Outlined.Download, contentDescription = null)
                        Text(
                            stringResource(R.string.action_download),
                            Modifier.padding(start = 8.dp),
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                if (discovery != null) {
                    io.github.jamezuk.lyrionanywhere.ui.discovery.StartRadioButton(discovery, onStartRadio)
                    io.github.jamezuk.lyrionanywhere.ui.discovery.MoreLikeThisShelf(discovery, onSimilar)
                }
                HorizontalDivider(Modifier.padding(top = 8.dp))
            }
        }
        if (stack.size <= 1 && credits.isNotEmpty()) {
            item(key = "credits") {
                io.github.jamezuk.lyrionanywhere.ui.library.CreditsSection(credits, onCredit)
                HorizontalDivider(Modifier.padding(top = 8.dp))
            }
        }
        when {
            frame == null || (frame.items.isEmpty() && frame.loading) -> item {
                Box(Modifier.fillMaxWidth().padding(24.dp), contentAlignment = Alignment.Center) { LoadingState(Modifier.size(48.dp)) }
            }
            frame.items.isEmpty() && frame.error != null -> item {
                Column(Modifier.fillMaxWidth().padding(16.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    Text(
                        errorMessage(frame.error),
                        textAlign = TextAlign.Center,
                        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                    )
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
                // Keyed by section too, and once each, so a tile repeated anywhere can't crash the grid.
                val shown = state.shown(list).distinctBy { it.key }
                if (shown.isEmpty()) continue
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Text(stringResource(title), style = MaterialTheme.typography.titleMedium, modifier = Modifier.semantics { heading() })
                }
                items(shown, key = { "$title/${it.key}" }) { tile -> AppTileCard(tile, onClick = { onOpen(tile) }) }
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
