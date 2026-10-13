package io.github.jamezuk.lyrionanywhere.ui.library

import android.content.res.Resources
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.FilterAlt
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import io.github.jamezuk.lyrionanywhere.core.FilterRule
import io.github.jamezuk.lyrionanywhere.core.SmartFilter
import io.github.jamezuk.lyrionanywhere.data.FilterLimits
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage
import io.github.jamezuk.lyrionanywhere.ui.components.errorText
import io.github.jamezuk.lyrionanywhere.ui.menus.MoreRequest

/** A message for the user (a snackbar), usable outside composition. */
fun filterMessageText(res: Resources, m: FilterMessage): String = when (m) {
    FilterMessage.NoPlayer -> res.getString(R.string.station_no_player)
    is FilterMessage.Playing -> res.getString(if (m.shuffle) R.string.filter_shuffling else R.string.filter_playing, m.name)
    is FilterMessage.NoAlbums -> res.getString(R.string.filter_nothing_to_play, m.name)
    is FilterMessage.Saved -> res.getString(if (m.added) R.string.filter_added else R.string.filter_saved, m.name)
    is FilterMessage.Deleted -> res.getString(R.string.filter_deleted_named, m.name)
    is FilterMessage.Failed -> errorText(res, m.error)
}

@Composable
private fun ruleKindLabel(k: RuleKind): String = stringResource(
    when (k) {
        RuleKind.GENRE -> R.string.filter_rule_genre
        RuleKind.DECADE -> R.string.filter_rule_decade
        RuleKind.ARTIST -> R.string.filter_rule_artist
        RuleKind.COMPILATION -> R.string.filter_rule_compilation
    },
)

/** One rule, in a few words ("3 genres", "1990s", "An artist (composer)"). */
@Composable
fun ruleSummary(r: FilterRule): String = when (r) {
    is FilterRule.Genre -> pluralStringResource(R.plurals.filter_summary_genres, r.genres.size, r.genres.size)
    is FilterRule.Decade -> stringResource(R.string.filter_decade, r.decade.toInt())
    is FilterRule.Artist -> r.role?.let { stringResource(R.string.filter_summary_artist_role, roleLabel(it).lowercase()) }
        ?: stringResource(R.string.filter_summary_artist)
    is FilterRule.Compilation -> stringResource(if (r.compilation) R.string.filter_compilation_only else R.string.filter_compilation_none)
}

@Composable
fun filterSummary(f: SmartFilter): String = f.rules.map { ruleSummary(it) }.joinToString(" · ")

@Composable
fun filterSortLabel(s: AlbumSort): String = stringResource(
    when (s) {
        AlbumSort.TITLE -> R.string.sort_a_to_z
        AlbumSort.ARTIST -> R.string.sort_by_artist_short
        AlbumSort.YEAR -> R.string.sort_by_year_short
        AlbumSort.NEWEST -> R.string.sort_recently_added
        AlbumSort.RANDOM -> R.string.filter_sort_random
    },
)

/** What the filters list can do. */
data class FiltersActions(
    val onOpen: (SmartFilter) -> Unit = {},
    val onNew: () -> Unit = {},
    val onEdit: (SmartFilter) -> Unit = {},
    val onPlay: (SmartFilter, shuffle: Boolean) -> Unit = { _, _ -> },
    val onDelete: (SmartFilter) -> Unit = {},
    val onRetry: () -> Unit = {},
)

/**
 * The *Filters* chip in Library (Wave 2 §5.6): each filter opens its albums, plays or
 * shuffles them, and is edited or deleted (after asking); New filter opens the rule builder.
 */
@Composable
fun FiltersScreen(ui: FiltersUi, actions: FiltersActions, modifier: Modifier = Modifier) {
    var deleting by remember { mutableStateOf<SmartFilter?>(null) }
    LazyColumn(modifier.fillMaxSize().testTag("filters"), contentPadding = PaddingValues(bottom = 16.dp)) {
        item {
            Row(
                Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    stringResource(R.string.filters_title),
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.weight(1f).semantics { heading() },
                )
                Button(
                    onClick = actions.onNew,
                    enabled = ui.filters is Loadable.Ready && !ui.full,
                    modifier = Modifier.testTag("filter-new"),
                ) {
                    Icon(Icons.Outlined.Add, contentDescription = null, modifier = Modifier.size(18.dp))
                    Text(stringResource(R.string.filter_new), Modifier.padding(start = 8.dp))
                }
            }
        }
        item {
            Text(
                if (ui.full) {
                    stringResource(R.string.filters_explained) + " " + stringResource(R.string.filters_full, FilterLimits.MAX_FILTERS)
                } else {
                    stringResource(R.string.filters_explained)
                },
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }
        when (val f = ui.filters) {
            Loadable.Loading -> item { LoadingState(Modifier.heightIn(min = 160.dp)) }
            is Loadable.Failed -> item { ErrorState(f.error, actions.onRetry, Modifier.heightIn(min = 200.dp)) }
            is Loadable.Ready -> if (f.value.isEmpty()) {
                item {
                    Text(
                        stringResource(R.string.filters_none),
                        Modifier.padding(16.dp).testTag("filters-none"),
                        style = MaterialTheme.typography.bodyLarge,
                    )
                }
            } else {
                items(f.value, key = { it.id }) { filter ->
                    FilterRow(filter, ui.playing, actions, onDelete = { deleting = filter })
                    HorizontalDivider()
                }
            }
        }
    }
    deleting?.let { f ->
        ConfirmDeleteFilter(f, onConfirm = {
            deleting = null
            actions.onDelete(f)
        }, onDismiss = { deleting = null })
    }
}

@Composable
private fun FilterRow(f: SmartFilter, playing: Boolean, actions: FiltersActions, onDelete: () -> Unit) {
    var menu by remember { mutableStateOf(false) }
    ListItem(
        leadingContent = { Icon(Icons.Outlined.FilterAlt, contentDescription = null) },
        headlineContent = { Text(f.name, maxLines = 2, overflow = TextOverflow.Ellipsis) },
        supportingContent = { Text(filterSummary(f), maxLines = 2, overflow = TextOverflow.Ellipsis) },
        trailingContent = {
            Row {
                IconButton(onClick = { actions.onPlay(f, false) }, enabled = !playing, modifier = Modifier.testTag("filter-play-${f.id}")) {
                    Icon(Icons.Outlined.PlayArrow, stringResource(R.string.filter_play, f.name))
                }
                IconButton(onClick = { actions.onPlay(f, true) }, enabled = !playing, modifier = Modifier.testTag("filter-shuffle-${f.id}")) {
                    Icon(Icons.Outlined.Shuffle, stringResource(R.string.filter_shuffle, f.name))
                }
                Box {
                    IconButton(onClick = { menu = true }, modifier = Modifier.testTag("filter-more-${f.id}")) {
                        Icon(Icons.Outlined.MoreVert, stringResource(R.string.filter_more, f.name))
                    }
                    DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.filter_action_edit)) },
                            leadingIcon = { Icon(Icons.Outlined.Edit, contentDescription = null) },
                            onClick = {
                                menu = false
                                actions.onEdit(f)
                            },
                            modifier = Modifier.testTag("filter-edit-${f.id}"),
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.filter_action_delete)) },
                            leadingIcon = { Icon(Icons.Outlined.Delete, contentDescription = null) },
                            onClick = {
                                menu = false
                                onDelete()
                            },
                            modifier = Modifier.testTag("filter-delete-${f.id}"),
                        )
                    }
                }
            }
        },
        modifier = Modifier.clickable(onClickLabel = stringResource(R.string.filter_open)) { actions.onOpen(f) }
            .testTag("filter-${f.id}"),
    )
}

@Composable
fun ConfirmDeleteFilter(f: SmartFilter, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.filter_delete_title, f.name)) },
        text = { Text(stringResource(R.string.filter_delete_body)) },
        confirmButton = {
            TextButton(onClick = onConfirm, modifier = Modifier.testTag("filter-delete-confirm")) {
                Text(stringResource(R.string.action_delete))
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}

/** What a filter's page can do. */
data class FilterAlbumsActions(
    val onAlbum: (Album) -> Unit = {},
    val onLoadMore: () -> Unit = {},
    val onMore: ((MoreRequest) -> Unit)? = null,
    val onPlay: (shuffle: Boolean) -> Unit = {},
    val onEdit: (SmartFilter) -> Unit = {},
    val onDelete: () -> Unit = {},
    val onRetry: () -> Unit = {},
)

/** One smart filter: its rules and its albums, a page at a time; play, shuffle, edit, delete. */
@Composable
fun FilterAlbumsScreen(ui: FilterAlbumsUi, actions: FilterAlbumsActions, modifier: Modifier = Modifier) {
    var deleting by remember { mutableStateOf(false) }
    when (val f = ui.filter) {
        Loadable.Loading -> LoadingState(modifier)
        is Loadable.Failed -> ErrorState(f.error, actions.onRetry, modifier)
        is Loadable.Ready -> {
            val filter = f.value
            if (filter == null) {
                EmptyState(stringResource(R.string.filter_deleted), modifier.testTag("filter-gone"))
                return
            }
            AlbumGrid(
                ui.albums,
                onAlbum = actions.onAlbum,
                onLoadMore = actions.onLoadMore,
                onMore = actions.onMore,
                empty = stringResource(R.string.filter_no_albums),
                modifier = modifier,
                header = { FilterHeader(filter, ui, actions, onDelete = { deleting = true }) },
            )
            if (deleting) {
                ConfirmDeleteFilter(filter, onConfirm = {
                    deleting = false
                    actions.onDelete()
                }, onDismiss = { deleting = false })
            }
        }
    }
}

@Composable
private fun FilterHeader(f: SmartFilter, ui: FilterAlbumsUi, actions: FilterAlbumsActions, onDelete: () -> Unit) {
    Column(Modifier.fillMaxWidth().padding(bottom = 8.dp)) {
        Text(f.name, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
        Text(
            filterSummary(f),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(top = 4.dp),
        )
        val canPlay = !ui.playing && ui.albums.items.isNotEmpty()
        FlowRow(Modifier.padding(top = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = { actions.onPlay(false) }, enabled = canPlay, modifier = Modifier.testTag("filter-page-play")) {
                Icon(Icons.Outlined.PlayArrow, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(stringResource(R.string.filter_action_play), Modifier.padding(start = 8.dp))
            }
            OutlinedButton(onClick = { actions.onPlay(true) }, enabled = canPlay, modifier = Modifier.testTag("filter-page-shuffle")) {
                Icon(Icons.Outlined.Shuffle, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(stringResource(R.string.filter_action_shuffle), Modifier.padding(start = 8.dp))
            }
            OutlinedButton(onClick = { actions.onEdit(f) }, modifier = Modifier.testTag("filter-page-edit")) {
                Icon(Icons.Outlined.Edit, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(stringResource(R.string.filter_action_edit), Modifier.padding(start = 8.dp))
            }
            OutlinedButton(onClick = onDelete, modifier = Modifier.testTag("filter-page-delete")) {
                Icon(Icons.Outlined.Delete, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(stringResource(R.string.filter_action_delete), Modifier.padding(start = 8.dp))
            }
        }
    }
}

/** What the rule builder can do (a [FilterEditorViewModel]'s methods). */
data class FilterEditorActions(
    val onName: (String) -> Unit = {},
    val onAddRule: (RuleKind) -> Unit = {},
    val onRemoveRule: (RuleKind) -> Unit = {},
    val onGenre: (Long) -> Unit = {},
    val onRetryGenres: () -> Unit = {},
    val onDecade: (Int) -> Unit = {},
    val onArtistQuery: (String) -> Unit = {},
    val onArtist: (Artist) -> Unit = {},
    val onChangeArtist: () -> Unit = {},
    val onRole: (ArtistRole?) -> Unit = {},
    val onCompilation: (Boolean) -> Unit = {},
    val onSort: (AlbumSort) -> Unit = {},
    val onLimit: (String) -> Unit = {},
    val onSave: () -> Unit = {},
    val onCancel: () -> Unit = {},
) {
    companion object {
        fun of(vm: FilterEditorViewModel, onCancel: () -> Unit) = FilterEditorActions(
            onName = vm::setName,
            onAddRule = vm::addRule,
            onRemoveRule = vm::removeRule,
            onGenre = vm::toggleGenre,
            onRetryGenres = vm::retryGenres,
            onDecade = vm::setDecade,
            onArtistQuery = vm::setArtistQuery,
            onArtist = vm::chooseArtist,
            onChangeArtist = vm::clearArtist,
            onRole = vm::setRole,
            onCompilation = vm::setCompilation,
            onSort = vm::setSort,
            onLimit = vm::setLimit,
            onSave = vm::save,
            onCancel = onCancel,
        )
    }
}

/** The rule builder, full screen over what opened it. */
@Composable
fun FilterEditorDialog(ui: FilterEditorUi, actions: FilterEditorActions) {
    Dialog(
        onDismissRequest = actions.onCancel,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Surface(Modifier.fillMaxSize()) { FilterEditor(ui, actions) }
    }
}

/**
 * The rule builder (Wave 2 §5.6): a name, up to one rule of each kind (all must match), the
 * order and how many albums. Why it can't be saved is announced as soon as Save is pressed.
 */
@Composable
fun FilterEditor(ui: FilterEditorUi, actions: FilterEditorActions, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxSize().imePadding().testTag("filter-editor")) {
        Row(Modifier.fillMaxWidth().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = actions.onCancel) { Icon(Icons.Outlined.Close, stringResource(R.string.filter_close)) }
            Text(
                stringResource(if (ui.editing != null) R.string.filter_editor_edit else R.string.filter_editor_new),
                style = MaterialTheme.typography.titleLarge,
                modifier = Modifier.weight(1f).semantics { heading() },
            )
        }
        Column(
            Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            OutlinedTextField(
                value = ui.name,
                onValueChange = actions.onName,
                label = { Text(stringResource(R.string.filter_name)) },
                singleLine = true,
                modifier = Modifier.fillMaxWidth().testTag("filter-name"),
            )
            SectionTitle(stringResource(R.string.filter_rules))
            Text(
                stringResource(if (ui.rules.isEmpty()) R.string.filter_rules_none else R.string.filter_rules_explained),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            for (k in ui.rules) RuleCard(k, ui, actions)
            AddRule(ui, actions)
            SectionTitle(stringResource(R.string.filter_order))
            FlowRow(Modifier.selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (s in FILTER_SORTS) {
                    FilterChip(
                        selected = ui.sort == s,
                        onClick = { actions.onSort(s) },
                        label = { Text(filterSortLabel(s)) },
                        modifier = Modifier.testTag("filter-sort-${s.name.lowercase()}"),
                    )
                }
            }
            OutlinedTextField(
                value = ui.limit,
                onValueChange = actions.onLimit,
                label = { Text(stringResource(R.string.filter_limit)) },
                supportingText = { Text(stringResource(R.string.filter_limit_hint)) },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number),
                modifier = Modifier.fillMaxWidth().testTag("filter-limit"),
            )
        }
        val error = ui.problem ?: ui.failure?.let { errorMessage(it) }
        if (error != null) {
            Text(
                error,
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp)
                    .semantics { liveRegion = LiveRegionMode.Assertive }
                    .testTag("filter-error"),
            )
        }
        Row(Modifier.fillMaxWidth().padding(16.dp), horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End)) {
            OutlinedButton(onClick = actions.onCancel) { Text(stringResource(R.string.action_cancel)) }
            Button(onClick = actions.onSave, enabled = !ui.saving, modifier = Modifier.testTag("filter-save")) {
                Text(stringResource(R.string.action_save))
            }
        }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 4.dp).semantics { heading() })
}

@Composable
private fun AddRule(ui: FilterEditorUi, actions: FilterEditorActions) {
    if (ui.addable.isEmpty()) {
        if (ui.rules.size >= FilterLimits.MAX_RULES) Text(stringResource(R.string.filter_rules_full))
        return
    }
    Text(stringResource(R.string.filter_rule_add), style = MaterialTheme.typography.labelLarge)
    FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        for (k in ui.addable) {
            val label = ruleKindLabel(k)
            val spoken = stringResource(R.string.filter_rule_add_named, label)
            AssistChip(
                onClick = { actions.onAddRule(k) },
                label = { Text(label) },
                leadingIcon = { Icon(Icons.Outlined.Add, contentDescription = null, modifier = Modifier.size(18.dp)) },
                modifier = Modifier.semantics { contentDescription = spoken }.testTag("filter-add-${k.name.lowercase()}"),
            )
        }
    }
}

@Composable
private fun RuleCard(k: RuleKind, ui: FilterEditorUi, actions: FilterEditorActions) {
    val label = ruleKindLabel(k)
    Card(Modifier.fillMaxWidth().testTag("filter-rule-${k.name.lowercase()}")) {
        Column(Modifier.padding(start = 12.dp, end = 4.dp, bottom = 12.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(label, style = MaterialTheme.typography.titleSmall, modifier = Modifier.weight(1f).semantics { heading() })
                IconButton(onClick = { actions.onRemoveRule(k) }, modifier = Modifier.testTag("filter-remove-${k.name.lowercase()}")) {
                    Icon(Icons.Outlined.Close, stringResource(R.string.filter_rule_remove, label.lowercase()))
                }
            }
            Column(Modifier.padding(end = 8.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                when (k) {
                    RuleKind.GENRE -> GenreRule(ui, actions)
                    RuleKind.DECADE -> FlowRow(Modifier.selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        for (d in FilterLimits.DECADES) {
                            FilterChip(
                                selected = ui.decade == d,
                                onClick = { actions.onDecade(d) },
                                label = { Text(stringResource(R.string.filter_decade, d)) },
                                modifier = Modifier.testTag("filter-decade-$d"),
                            )
                        }
                    }
                    RuleKind.ARTIST -> ArtistRule(ui, actions)
                    RuleKind.COMPILATION -> Column(Modifier.selectableGroup()) {
                        for ((on, text) in listOf(true to R.string.filter_compilation_only, false to R.string.filter_compilation_none)) {
                            Row(
                                Modifier.fillMaxWidth().heightIn(min = 48.dp)
                                    .selectable(selected = ui.compilation == on, role = Role.RadioButton) { actions.onCompilation(on) }
                                    .testTag("filter-compilation-$on"),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                RadioButton(selected = ui.compilation == on, onClick = null)
                                Text(stringResource(text), Modifier.padding(start = 8.dp))
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun GenreRule(ui: FilterEditorUi, actions: FilterEditorActions) {
    Text(
        stringResource(R.string.filter_genres_chosen, ui.genres.size, FilterLimits.MAX_GENRES),
        style = MaterialTheme.typography.bodySmall,
        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite }.testTag("filter-genre-count"),
    )
    when (val g = ui.genreChoices) {
        null, Loadable.Loading -> Row(verticalAlignment = Alignment.CenterVertically) {
            CircularProgressIndicator(Modifier.size(18.dp))
            Text(stringResource(R.string.filter_genres_loading), Modifier.padding(start = 8.dp))
        }
        is Loadable.Failed -> Row(verticalAlignment = Alignment.CenterVertically) {
            Text(errorMessage(g.error), Modifier.weight(1f))
            TextButton(onClick = actions.onRetryGenres) { Text(stringResource(R.string.action_retry)) }
        }
        is Loadable.Ready -> if (g.value.isEmpty()) {
            Text(stringResource(R.string.filter_genres_none))
        } else {
            val full = ui.genres.size >= FilterLimits.MAX_GENRES
            FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (genre in g.value) {
                    val on = genre.id in ui.genres
                    FilterChip(
                        selected = on,
                        onClick = { actions.onGenre(genre.id) },
                        enabled = on || !full,
                        label = { Text(genre.name) },
                        modifier = Modifier.testTag("filter-genre-${genre.id}"),
                    )
                }
            }
        }
    }
}

@Composable
private fun ArtistRule(ui: FilterEditorUi, actions: FilterEditorActions) {
    if (ui.artistId != null) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            val name = ui.artistName?.takeIf { it.isNotBlank() } ?: stringResource(R.string.filter_artist_chosen)
            Text(stringResource(R.string.filter_artist_is, name), Modifier.weight(1f).testTag("filter-artist-chosen"))
            TextButton(onClick = actions.onChangeArtist, modifier = Modifier.testTag("filter-artist-change")) {
                Text(stringResource(R.string.filter_artist_change))
            }
        }
    } else {
        OutlinedTextField(
            value = ui.artistQuery,
            onValueChange = actions.onArtistQuery,
            label = { Text(stringResource(R.string.filter_artist_find)) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth().testTag("filter-artist-search"),
        )
        val status = when {
            ui.searching -> stringResource(R.string.filter_artist_searching)
            ui.artistQuery.trim().length >= 2 && ui.artistResults.isEmpty() -> stringResource(R.string.filter_artist_none)
            else -> ""
        }
        Text(
            status,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
        )
        for (a in ui.artistResults) {
            val pick = stringResource(R.string.filter_artist_pick, a.name)
            ListItem(
                headlineContent = { Text(a.name) },
                modifier = Modifier.clickable(onClickLabel = pick, role = Role.Button) { actions.onArtist(a) }
                    .testTag("filter-artist-${a.id}"),
            )
        }
    }
    Text(stringResource(R.string.filter_role), style = MaterialTheme.typography.labelLarge)
    FlowRow(Modifier.selectableGroup(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        FilterChip(
            selected = ui.role == null,
            onClick = { actions.onRole(null) },
            label = { Text(stringResource(R.string.filter_role_any)) },
            modifier = Modifier.testTag("filter-role-any"),
        )
        for (r in ROLE_ORDER) {
            FilterChip(
                selected = ui.role == r,
                onClick = { actions.onRole(r) },
                label = { Text(roleLabel(r)) },
                modifier = Modifier.testTag("filter-role-${r.name.lowercase()}"),
            )
        }
    }
}

/** The browse roles, as LMS lists them. */
private val ROLE_ORDER = listOf(
    ArtistRole.ALBUM_ARTIST,
    ArtistRole.ARTIST,
    ArtistRole.TRACK_ARTIST,
    ArtistRole.COMPOSER,
    ArtistRole.CONDUCTOR,
    ArtistRole.BAND,
)

/**
 * The rule builder with its own view model ([key] names one opening, so each starts fresh):
 * [onSaved] once it is saved (then it closes), [onClose] to leave without saving.
 */
@Composable
fun FilterEditorHost(
    key: String,
    make: () -> FilterEditorViewModel,
    onSaved: (SmartFilter) -> Unit,
    onClose: () -> Unit,
) {
    val vm = viewModel(key = key) { make() }
    val ui by vm.state.collectAsStateWithLifecycle()
    val saved = ui.saved
    LaunchedEffect(saved) { if (saved != null) onSaved(saved) }
    if (saved == null) FilterEditorDialog(ui, FilterEditorActions.of(vm, onClose))
}
