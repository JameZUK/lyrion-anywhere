package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGesturesAfterLongPress
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.ClearAll
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DragHandle
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import kotlinx.coroutines.launch
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
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
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import io.github.jamezuk.lyrionanywhere.ui.library.TrackRow

/** What the queue can do. */
data class QueueActions(
    val onJump: (Int) -> Unit,
    val onRemove: (Int) -> Unit = {},
    val onMove: (Int, Int) -> Unit = { _, _ -> },
    val onClear: () -> Unit = {},
    /** The list reached its end and the queue has more: read the next page. */
    val onLoadMore: () -> Unit = {},
)

/** Which row is being dragged, and how far (shared by the rows of one list). */
@Stable
class QueueDrag {
    var from by mutableIntStateOf(-1)
    var offset by mutableFloatStateOf(0f)
    var rowPx by mutableFloatStateOf(0f)
}

/** The queue's heading, with "Clear queue" (asks first). */
@Composable
fun QueueHeader(count: Int, playerName: String, onClear: () -> Unit) {
    var confirm by rememberSaveable { mutableStateOf(false) }
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Box(Modifier.weight(1f)) { SectionHeader(stringResource(R.string.section_queue)) }
        if (count > 0) {
            TextButton(onClick = { confirm = true }) {
                Icon(Icons.Outlined.ClearAll, contentDescription = null)
                Text(stringResource(R.string.queue_clear), Modifier.padding(start = 4.dp))
            }
        }
    }
    if (confirm) {
        AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text(stringResource(R.string.queue_clear_confirm_title)) },
            text = {
                // Scrolls when large text makes it taller than the dialog.
                Text(stringResource(R.string.queue_clear_confirm, playerName), Modifier.verticalScroll(rememberScrollState()))
            },
            confirmButton = {
                TextButton(onClick = {
                    confirm = false
                    onClear()
                }) { Text(stringResource(R.string.action_clear)) }
            },
            dismissButton = { TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
}

/**
 * The queue's rows: tap to play, swipe left to remove, long-press the handle and drag to
 * reorder. TalkBack users get Move up, Move down and Remove as actions on each row; with a
 * keyboard, Alt+Up and Alt+Down move the focused row and Delete removes it.
 */
fun LazyListScope.queueItems(
    entries: List<QueueEntry>,
    currentIndex: Int?,
    drag: QueueDrag,
    actions: QueueActions,
    hasMore: Boolean = false,
) {
    if (entries.isEmpty() && !hasMore) {
        item(key = "queue-empty") { Text(stringResource(R.string.queue_empty), Modifier.padding(16.dp)) }
    }
    itemsIndexed(entries, key = { _, e -> e.key }) { i, e ->
        QueueRow(i, e, entries.size, i == currentIndex, drag, actions)
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

@Composable
private fun QueueRow(
    index: Int,
    entry: QueueEntry,
    size: Int,
    current: Boolean,
    drag: QueueDrag,
    actions: QueueActions,
) {
    val haptics = LocalHapticFeedback.current
    val dismiss = rememberSwipeToDismissBoxState()
    val scope = rememberCoroutineScope()
    val dragging = drag.from == index
    val moveUp = stringResource(R.string.queue_move_up)
    val moveDown = stringResource(R.string.queue_move_down)
    val remove = stringResource(R.string.queue_remove)
    SwipeToDismissBox(
        state = dismiss,
        enableDismissFromStartToEnd = false,
        onDismiss = {
            if (it == SwipeToDismissBoxValue.EndToStart) actions.onRemove(index)
            // A removal that LMS refuses puts the same entry back: it must not stay swiped away.
            scope.launch { dismiss.snapTo(SwipeToDismissBoxValue.Settled) }
        },
        backgroundContent = {
            Box(
                Modifier.fillMaxSize().background(MaterialTheme.colorScheme.errorContainer).padding(horizontal = 24.dp),
                contentAlignment = Alignment.CenterEnd,
            ) {
                Icon(Icons.Outlined.Delete, contentDescription = null, tint = MaterialTheme.colorScheme.onErrorContainer)
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
                    highlighted = current,
                    showArt = true,
                    modifier = Modifier
                        .testTag("queue-track-$index")
                        .onPreviewKeyEvent { e -> e.type == KeyEventType.KeyDown && queueKey(e, index, size, actions) }
                        .semantics {
                            customActions = buildList {
                                if (index > 0) add(CustomAccessibilityAction(moveUp) { actions.onMove(index, index - 1); true })
                                if (index < size - 1) add(CustomAccessibilityAction(moveDown) { actions.onMove(index, index + 1); true })
                                add(CustomAccessibilityAction(remove) { actions.onRemove(index); true })
                            }
                        },
                )
            }
            // Touch only: screen-reader and keyboard users have the row's actions instead.
            Icon(
                Icons.Outlined.DragHandle,
                contentDescription = null,
                modifier = Modifier
                    .testTag("queue-drag-$index")
                    .padding(horizontal = 12.dp, vertical = 16.dp)
                    .pointerInput(index, size) {
                        detectDragGesturesAfterLongPress(
                            onDragStart = {
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                drag.from = index
                                drag.offset = 0f
                            },
                            onDragEnd = {
                                val to = QueueEdit.dropIndex(index, drag.offset, drag.rowPx, size)
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
