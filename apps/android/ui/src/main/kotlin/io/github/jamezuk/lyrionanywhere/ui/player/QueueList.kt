package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.PlaylistAdd
import androidx.compose.material.icons.automirrored.outlined.PlaylistPlay
import androidx.compose.material.icons.outlined.ClearAll
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DownloadDone
import androidx.compose.material.icons.outlined.DragHandle
import androidx.compose.material.icons.outlined.OfflinePin
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.HapticSteps
import io.github.jamezuk.lyrionanywhere.ui.components.OverflowRow
import io.github.jamezuk.lyrionanywhere.ui.components.RowAction
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import io.github.jamezuk.lyrionanywhere.ui.library.TrackRow
import kotlinx.coroutines.launch

/** What the queue can do. */
data class QueueActions(
    val onJump: (Int) -> Unit,
    val onRemove: (Int) -> Unit = {},
    val onMove: (Int, Int) -> Unit = { _, _ -> },
    val onClear: () -> Unit = {},
    /** The list reached its end and the queue has more: read the next page. */
    val onLoadMore: () -> Unit = {},
    /** Move the entry at an index to just after the current track (Wave 2 §3.4). */
    val onPlayNext: (Int) -> Unit = {},
)

/** Which row is being dragged, and how far (shared by the rows of one list). */
@Stable
class QueueDrag {
    var from by mutableIntStateOf(-1)
    var offset by mutableFloatStateOf(0f)
    var rowPx by mutableFloatStateOf(0f)
}

/**
 * The queue's header: one quiet line ("12 tracks on Lounge") and ⋮ with Save as playlist,
 * Clear upcoming (the tracks after the current one) and Clear queue, each clear asking first.
 * For a long queue, Clear upcoming offers Clear all instead. (The page's own title says
 * "Queue", so the header doesn't.)
 */
@Composable
fun QueueHeader(
    count: Int,
    playerName: String,
    onClear: () -> Unit,
    /** Tracks after the current one (null: not known, so not offered). */
    upcoming: Int? = null,
    onClearUpcoming: () -> Unit = {},
    /** "Save as playlist" (admins, on a gateway that has it); null: not offered. */
    onSaveAsPlaylist: (() -> Unit)? = null,
    /** False while the server can't be reached: the queue can't be changed (greyed out). */
    enabled: Boolean = true,
    /** The downloads queue (it says so, and has no actions). */
    downloads: Boolean = false,
) {
    var confirm by rememberSaveable { mutableStateOf(false) }
    var confirmUpcoming by rememberSaveable { mutableStateOf(false) }
    val actions = if (downloads) emptyList() else buildList {
        if (onSaveAsPlaylist != null && count > 0) {
            add(RowAction(stringResource(R.string.save_queue_action), Icons.AutoMirrored.Outlined.PlaylistAdd, onSaveAsPlaylist, enabled, tag = "save-queue") {})
        }
        if (upcoming != null && upcoming > 0) {
            add(RowAction(stringResource(R.string.queue_clear_upcoming), Icons.Outlined.ClearAll, { confirmUpcoming = true }, enabled, tag = "clear-upcoming") {})
        }
        if (count > 0) {
            add(RowAction(stringResource(R.string.queue_clear), Icons.Outlined.Delete, { confirm = true }, enabled, tag = "clear-queue") {})
        }
    }
    val summary = if (downloads) {
        LocalResources.current.getQuantityString(R.plurals.queue_summary_downloads, count, count)
    } else if (count > 0) {
        LocalResources.current.getQuantityString(R.plurals.queue_summary, count, count, playerName)
    } else {
        playerName
    }
    // Every action is in the menu: the header never wraps, whatever the width or text size.
    OverflowRow(
        actions = emptyList(),
        menuItems = actions,
        menuLabel = stringResource(R.string.queue_more),
        menuTag = "queue-actions",
        modifier = Modifier.fillMaxWidth().padding(end = 4.dp),
        lead = {
            Text(
                summary,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
                modifier = Modifier.padding(start = 16.dp, top = 8.dp, bottom = 8.dp).testTag("queue-title"),
            )
        },
    )
    if (confirm) {
        ConfirmDialog(
            title = stringResource(R.string.queue_clear_confirm_title),
            text = stringResource(R.string.queue_clear_confirm, playerName),
            confirm = stringResource(R.string.action_clear),
            onConfirm = {
                confirm = false
                onClear()
            },
            onDismiss = { confirm = false },
        )
    }
    if (confirmUpcoming && upcoming != null) {
        val long = upcoming > PlayerViewModel.CLEAR_UPCOMING_MAX
        ConfirmDialog(
            title = stringResource(R.string.queue_clear_upcoming_title),
            text = if (long) {
                LocalResources.current.getQuantityString(R.plurals.queue_clear_upcoming_long, upcoming, upcoming, playerName)
            } else {
                LocalResources.current.getQuantityString(R.plurals.queue_clear_upcoming_confirm, upcoming, upcoming, playerName)
            },
            confirm = stringResource(if (long) R.string.action_clear_all else R.string.queue_clear_upcoming),
            onConfirm = {
                confirmUpcoming = false
                if (long) onClear() else onClearUpcoming()
            },
            onDismiss = { confirmUpcoming = false },
        )
    }
}

@Composable
private fun ConfirmDialog(title: String, text: String, confirm: String, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        // Scrolls when large text makes it taller than the dialog.
        text = { Text(text, Modifier.verticalScroll(rememberScrollState())) },
        confirmButton = { TextButton(onClick = onConfirm) { Text(confirm) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}

/**
 * The queue's rows: tap to play; swipe right to play next, left to remove (with Undo);
 * long-press the handle and drag to reorder. Each row's menu, and TalkBack's actions on it,
 * offer the same (Play next, Move up, Move down, Remove); with a keyboard, Alt+Up and
 * Alt+Down move the focused row and Delete removes it.
 *
 * With [sections] the queue shows Now playing, Up next and Then; without, it is flat.
 * (Whether a track plays straight on from the one before is in its Track info, not here.)
 */
fun LazyListScope.queueItems(
    entries: List<QueueEntry>,
    currentIndex: Int?,
    drag: QueueDrag,
    actions: QueueActions,
    hasMore: Boolean = false,
    sections: QueueSections? = null,
    /** Where each track is on the phone (its mark). */
    marks: (Track) -> QueueMark = { QueueMark.NONE },
    /** Which controls work (while the server is away: rows not on the phone are dimmed). */
    gate: ControlGate = ControlGate.ONLINE,
    /** False: a queue that can't be edited (the downloads queue), rows only play. */
    editable: Boolean = true,
) {
    if (entries.isEmpty() && !hasMore) {
        item(key = "queue-empty") { Text(stringResource(R.string.queue_empty), Modifier.padding(16.dp)) }
    }
    itemsIndexed(entries, key = { _, e -> e.key }) { i, e ->
        Box(Modifier.fillMaxWidth()) {
            androidx.compose.foundation.layout.Column {
                when {
                    sections == null -> {}
                    i == sections.current -> SectionHeader(stringResource(R.string.now_playing), Modifier.testTag("section-now"))
                    i == sections.upNext.first -> SectionHeader(stringResource(R.string.up_next), Modifier.testTag("section-up-next"))
                    i == sections.thenFrom -> SectionHeader(stringResource(R.string.section_then), Modifier.testTag("section-then"))
                }
                QueueRow(i, e, entries.size, currentIndex, drag, actions, marks(e.track), gate, editable)
            }
        }
    }
    if (hasMore) {
        // Shown once the list is scrolled to its end: it asks for the next page (again
        // whenever the list read changes, as a read skipped during an edit is retried).
        item(key = "queue-more") {
            val onLoadMore by rememberUpdatedState(actions.onLoadMore)
            LaunchedEffect(entries) { onLoadMore() }
            Box(Modifier.fillMaxWidth().padding(16.dp).testTag("queue-more"), contentAlignment = Alignment.Center) {
                CircularProgressIndicator(Modifier.size(24.dp))
            }
        }
    }
}

/**
 * The mark a queue row shows: downloaded (the one offline mark), or kept on the phone (only
 * while your music server is out of reach: otherwise the cache goes unmentioned).
 */
internal fun shownMark(mark: QueueMark, gate: ControlGate): QueueMark =
    if (mark == QueueMark.ON_DEVICE && gate.online) QueueMark.NONE else mark

/** A queue row's mark: an icon with its own label, for TalkBack too. */
@Composable
internal fun QueueMarkIcon(mark: QueueMark, index: Int) {
    when (mark) {
        QueueMark.NONE -> return
        QueueMark.DOWNLOADED -> io.github.jamezuk.lyrionanywhere.ui.components.OfflineMark(Modifier.testTag("queue-mark-$index"))
        QueueMark.ON_DEVICE -> io.github.jamezuk.lyrionanywhere.ui.components.CacheMark(Modifier.testTag("queue-mark-$index"))
    }
}

@Composable
private fun QueueRow(
    index: Int,
    entry: QueueEntry,
    size: Int,
    currentIndex: Int?,
    drag: QueueDrag,
    actions: QueueActions,
    mark: QueueMark = QueueMark.NONE,
    gate: ControlGate = ControlGate.ONLINE,
    canEdit: Boolean = true,
) {
    // While the server is away: no edits, and only rows on the phone can be played.
    val editable = gate.other && canEdit
    val playable = gate.canJump(mark)
    val haptics = LocalHapticFeedback.current
    val dismiss = rememberSwipeToDismissBoxState()
    val scope = rememberCoroutineScope()
    val dragging = drag.from == index
    val canPlayNext = currentIndex != null && QueueEdit.playNextTarget(index, currentIndex) != null
    val playNext = stringResource(R.string.action_play_next)
    val moveUp = stringResource(R.string.queue_move_up)
    val moveDown = stringResource(R.string.queue_move_down)
    val remove = stringResource(R.string.queue_remove)
    var menu by remember { mutableStateOf(false) }
    SwipeToDismissBox(
        state = dismiss,
        enableDismissFromStartToEnd = canPlayNext && editable,
        enableDismissFromEndToStart = editable,
        onDismiss = {
            when (it) {
                SwipeToDismissBoxValue.EndToStart -> actions.onRemove(index)
                SwipeToDismissBoxValue.StartToEnd -> actions.onPlayNext(index)
                else -> {}
            }
            // The row's place follows the queue: it must never stay swiped away (a refused
            // removal puts the same entry back; play next moves it).
            scope.launch { dismiss.snapTo(SwipeToDismissBoxValue.Settled) }
        },
        backgroundContent = {
            val right = dismiss.dismissDirection == SwipeToDismissBoxValue.StartToEnd
            Box(
                Modifier
                    .fillMaxSize()
                    .background(if (right) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.errorContainer)
                    .padding(horizontal = 24.dp),
                contentAlignment = if (right) Alignment.CenterStart else Alignment.CenterEnd,
            ) {
                if (right) {
                    Icon(Icons.AutoMirrored.Outlined.PlaylistPlay, contentDescription = null, tint = MaterialTheme.colorScheme.onPrimaryContainer)
                } else {
                    Icon(Icons.Outlined.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.onErrorContainer)
                }
            }
        },
        modifier = Modifier
            .testTag("queue-row-$index")
            .zIndex(if (dragging) 1f else 0f)
            .graphicsLayer { translationY = if (dragging) drag.offset else 0f }
            .onSizeChanged { drag.rowPx = it.height.toFloat() },
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            modifier = Modifier.background(MaterialTheme.colorScheme.background),
        ) {
            Box(Modifier.weight(1f)) {
                // The actions sit on the row TalkBack focuses (the one that plays the track).
                TrackRow(
                    entry.track,
                    onClick = { actions.onJump(index) },
                    highlighted = index == currentIndex,
                    showArt = true,
                    onMore = { menu = true }.takeIf { editable },
                    enabled = playable,
                    badge = if (shownMark(mark, gate) != QueueMark.NONE) {
                        { QueueMarkIcon(shownMark(mark, gate), index) }
                    } else {
                        null
                    },
                    modifier = Modifier
                        .testTag("queue-track-$index")
                        .onPreviewKeyEvent { e -> editable && e.type == KeyEventType.KeyDown && queueKey(e, index, size, actions) }
                        .semantics {
                            customActions = if (!editable) {
                                emptyList()
                            } else {
                                buildList {
                                    if (canPlayNext) add(CustomAccessibilityAction(playNext) { actions.onPlayNext(index); true })
                                    if (index > 0) add(CustomAccessibilityAction(moveUp) { actions.onMove(index, index - 1); true })
                                    if (index < size - 1) add(CustomAccessibilityAction(moveDown) { actions.onMove(index, index + 1); true })
                                    add(CustomAccessibilityAction(remove) { actions.onRemove(index); true })
                                }
                            }
                        },
                )
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    if (canPlayNext) {
                        DropdownMenuItem(text = { Text(playNext) }, onClick = {
                            menu = false
                            actions.onPlayNext(index)
                        })
                    }
                    if (index > 0) {
                        DropdownMenuItem(text = { Text(moveUp) }, onClick = {
                            menu = false
                            actions.onMove(index, index - 1)
                        })
                    }
                    if (index < size - 1) {
                        DropdownMenuItem(text = { Text(moveDown) }, onClick = {
                            menu = false
                            actions.onMove(index, index + 1)
                        })
                    }
                    DropdownMenuItem(text = { Text(remove) }, onClick = {
                        menu = false
                        actions.onRemove(index)
                    })
                }
            }
            // Touch only: screen-reader and keyboard users have the row's actions instead.
            if (editable) Icon(
                Icons.Outlined.DragHandle,
                contentDescription = null,
                modifier = Modifier
                    .testTag("queue-drag-$index")
                    .padding(horizontal = 12.dp, vertical = 16.dp)
                    .pointerInput(index, size) {
                        detectDragGesturesAfterLongPress(
                            onDragStart = {
                                haptics.performHapticFeedback(HapticSteps.pickUp)
                                drag.from = index
                                drag.offset = 0f
                            },
                            onDragEnd = {
                                val to = QueueEdit.dropIndex(index, drag.offset, drag.rowPx, size)
                                haptics.performHapticFeedback(HapticSteps.drop)
                                drag.from = -1
                                drag.offset = 0f
                                if (to != index) actions.onMove(index, to)
                            },
                            onDragCancel = {
                                drag.from = -1
                                drag.offset = 0f
                            },
                        ) { change, amount ->
                            change.consume()
                            drag.offset += amount.y
                        }
                    },
            )
        }
    }
}

/** A queue row's keys: Alt+Up/Alt+Down move it, Delete or Backspace removes it. */
internal fun queueKey(e: KeyEvent, index: Int, size: Int, actions: QueueActions): Boolean = when {
    e.isAltPressed && e.key == Key.DirectionUp -> {
        if (index > 0) actions.onMove(index, index - 1)
        true
    }
    e.isAltPressed && e.key == Key.DirectionDown -> {
        if (index < size - 1) actions.onMove(index, index + 1)
        true
    }
    e.key == Key.Delete || e.key == Key.Backspace -> {
        actions.onRemove(index)
        true
    }
    else -> false
}
