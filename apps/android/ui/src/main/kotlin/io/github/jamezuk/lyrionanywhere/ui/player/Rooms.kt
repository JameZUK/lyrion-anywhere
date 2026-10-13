package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material.icons.outlined.Alarm
import androidx.compose.material.icons.outlined.Bedtime
import androidx.compose.material.icons.outlined.DragIndicator
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.PowerSettingsNew
import androidx.compose.material.icons.outlined.SpeakerGroup
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.HapticSteps
import io.github.jamezuk.lyrionanywhere.ui.alarms.currentLocale
import io.github.jamezuk.lyrionanywhere.ui.alarms.dayName
import io.github.jamezuk.lyrionanywhere.ui.alarms.is24Hour
import io.github.jamezuk.lyrionanywhere.ui.alarms.timeText
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState

/** What the Rooms view can do. */
data class RoomsActions(
    /** Control this room (it becomes the selected player). */
    val onControl: (String) -> Unit,
    val onTogglePlay: (String) -> Unit,
    val onVolume: (String, Int) -> Unit,
    val onPower: (String, Boolean) -> Unit,
    /** The first room joins the second's group. */
    val onSync: (String, String) -> Unit,
    val onUnsync: (String) -> Unit,
    val onRetry: () -> Unit = {},
    /** Move what the controlled player plays to this room (Transfer); null: not offered. */
    val onTransfer: ((String) -> Unit)? = null,
    /** Open a room's alarms (id, name); null: not offered. */
    val onAlarms: ((String, String) -> Unit)? = null,
)

/** Dropping a dragged card: which card the finger is over. Plain Kotlin, tested. */
object RoomDrop {
    /** The room under [point] other than [dragging], or null. */
    fun target(point: Offset, cards: Map<String, Rect>, dragging: String): String? =
        cards.entries.firstOrNull { (id, r) -> id != dragging && r.contains(point) }?.key
}

/**
 * Every room at a glance (Wave 2 §4.3): a card per player with art, title and artist,
 * play/pause, volume, power, sleep remaining and the next alarm. A list on phones, a grid on
 * wider windows. Tap a card to control that room; long-press to move playback there
 * (Transfer); drag a card by its handle onto another to sync them. Every drag has its
 * alternative: "Sync with…" in the card's menu, and a TalkBack action for each other room.
 */
@Composable
fun RoomsScreen(
    state: RoomsUiState,
    actions: RoomsActions,
    modifier: Modifier = Modifier,
    /** The controlled player (its card says so; no Transfer to itself). */
    selectedId: String? = null,
    transfer: TransferState = TransferState.Idle,
    onDismissTransfer: () -> Unit = {},
) {
    when {
        state.loading && state.rooms.isEmpty() -> return LoadingState(modifier)
        state.error != null && state.rooms.isEmpty() -> return ErrorState(state.error, actions.onRetry, modifier)
        state.rooms.isEmpty() -> return EmptyState(stringResource(R.string.rooms_empty), modifier)
    }
    val haptics = LocalHapticFeedback.current
    val cards = remember { mutableStateMapOf<String, Rect>() }
    val handles = remember { mutableMapOf<String, Offset>() }
    var dragging by remember { mutableStateOf<String?>(null) }
    var dragAt by remember { mutableStateOf(Offset.Zero) }
    var dragBy by remember { mutableStateOf(Offset.Zero) }
    val over = dragging?.let { RoomDrop.target(dragAt, cards, it) }
    var syncing by rememberSaveable { mutableStateOf<String?>(null) }
    Column(modifier.fillMaxSize()) {
        TransferStatus(transfer, onDismissTransfer)
        LazyVerticalGrid(
            columns = GridCells.Adaptive(minSize = 320.dp),
            contentPadding = PaddingValues(12.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.fillMaxSize().testTag("rooms"),
        ) {
            if (state.more) {
                item(span = { GridItemSpan(maxLineSpan) }) {
                    Text(
                        pluralStringResource(R.plurals.rooms_more, state.rooms.size, state.rooms.size),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            items(state.rooms, key = { it.id }) { room ->
                val isDragged = dragging == room.id
                RoomCard(
                    room = room,
                    state = state,
                    actions = actions,
                    selected = room.id == selectedId,
                    canTransfer = actions.onTransfer != null && selectedId != null && room.id != selectedId &&
                        transfer !is TransferState.Running,
                    dropTarget = over == room.id,
                    onSyncWith = { syncing = room.id },
                    modifier = Modifier
                        .zIndex(if (isDragged) 1f else 0f)
                        .graphicsLayer {
                            if (isDragged) {
                                translationX = dragBy.x
                                translationY = dragBy.y
                                alpha = 0.9f
                            }
                        }
                        .onGloballyPositioned { if (!isDragged) cards[room.id] = it.boundsInRoot() },
                    handle = Modifier
                        .onGloballyPositioned { if (!isDragged) handles[room.id] = it.boundsInRoot().topLeft }
                        .pointerInput(room.id) {
                            detectDragGestures(
                                onDragStart = { start ->
                                    haptics.performHapticFeedback(HapticSteps.pickUp)
                                    dragging = room.id
                                    dragBy = Offset.Zero
                                    // Where the finger is, in the same coordinates as the cards.
                                    dragAt = (handles[room.id] ?: Offset.Zero) + start
                                },
                                onDragEnd = {
                                    val target = RoomDrop.target(dragAt, cards, room.id)
                                    haptics.performHapticFeedback(HapticSteps.drop)
                                    dragging = null
                                    dragBy = Offset.Zero
                                    if (target != null) actions.onSync(room.id, target)
                                },
                                onDragCancel = {
                                    dragging = null
                                    dragBy = Offset.Zero
                                },
                            ) { change, amount ->
                                change.consume()
                                dragBy += amount
                                dragAt += amount
                            }
                        },
                )
            }
        }
    }
    syncing?.let { id ->
        val room = state.room(id)
        if (room == null) {
            syncing = null
            return@let
        }
        SyncWithDialog(room, state.rooms.filter { it.id != id }, onPick = { target ->
            actions.onSync(id, target)
            syncing = null
        }, onDismiss = { syncing = null })
    }
}

/** "Sync Kitchen with…": the other rooms, one tap each (the alternative to dragging). */
@Composable
private fun SyncWithDialog(room: Room, others: List<Room>, onPick: (String) -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.room_sync_title, room.name)) },
        text = {
            Column(Modifier.testTag("sync-with")) {
                others.forEach { o ->
                    TextButton(onClick = { onPick(o.id) }, modifier = Modifier.fillMaxWidth().testTag("sync-with-${o.id}")) {
                        Text(o.name, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.fillMaxWidth())
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}

/**
 * One room's card: one accessible group, its controls named with the room ("Kitchen
 * volume"). The header (art, title, artist) controls the room on a tap and moves playback
 * there on a long-press, and carries the TalkBack actions that stand in for dragging.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun RoomCard(
    room: Room,
    state: RoomsUiState,
    actions: RoomsActions,
    selected: Boolean,
    canTransfer: Boolean,
    dropTarget: Boolean,
    onSyncWith: () -> Unit,
    modifier: Modifier,
    handle: Modifier,
) {
    val name = room.name
    val status = room.status
    val track = status?.current
    val group = state.groupOf(room.id)
    val others = state.rooms.filter { it.id != room.id }
    val latestActions by rememberUpdatedState(actions)
    val syncActions = others.map { o ->
        CustomAccessibilityAction(stringResource(R.string.room_sync_named, name, o.name)) { latestActions.onSync(room.id, o.id); true }
    }
    val unsyncLabel = stringResource(R.string.room_unsync_named, name)
    val transferLabel = stringResource(R.string.room_transfer, name)
    val cardActions = syncActions +
        (if (group.isNotEmpty()) listOf(CustomAccessibilityAction(unsyncLabel) { latestActions.onUnsync(room.id); true }) else emptyList()) +
        (if (canTransfer) listOf(CustomAccessibilityAction(transferLabel) { latestActions.onTransfer?.invoke(room.id); true }) else emptyList())
    Card(
        modifier = modifier
            .fillMaxWidth()
            .testTag("room-${room.id}")
            .semantics { isTraversalGroup = true },
        border = when {
            dropTarget -> BorderStroke(3.dp, MaterialTheme.colorScheme.primary)
            selected -> BorderStroke(1.dp, MaterialTheme.colorScheme.tertiary)
            else -> null
        },
        colors = CardDefaults.cardColors(),
    ) {
        Column(Modifier.padding(bottom = 8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Row(
                    Modifier
                        .weight(1f)
                        .heightIn(min = 72.dp)
                        .combinedClickable(
                            onClickLabel = stringResource(R.string.room_control, name),
                            onLongClickLabel = if (canTransfer) transferLabel else null,
                            onLongClick = if (canTransfer) ({ actions.onTransfer?.invoke(room.id) }) else null,
                            onClick = { actions.onControl(room.id) },
                        )
                        .semantics { customActions = cardActions }
                        .padding(start = 12.dp, top = 12.dp, bottom = 8.dp)
                        .testTag("room-head-${room.id}"),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    ArtworkImage(track?.artwork, 120, contentDescription = null, modifier = Modifier.size(56.dp), corner = 6.dp)
                    Column(Modifier.weight(1f)) {
                        Text(
                            name,
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.semantics { heading() },
                        )
                        Text(
                            roomLine(room),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (group.isNotEmpty()) {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Icon(Icons.Outlined.SpeakerGroup, contentDescription = null, modifier = Modifier.size(16.dp))
                                Text(
                                    stringResource(R.string.room_grouped, group.joinToString(" + ") { it.name }),
                                    style = MaterialTheme.typography.labelMedium,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                    modifier = Modifier.padding(start = 4.dp),
                                )
                            }
                        }
                    }
                }
                // Drag onto another card to sync. Its alternatives are the menu's "Sync
                // with…" and the TalkBack actions above, so screen readers skip it.
                Box(
                    handle
                        .size(48.dp)
                        .testTag("room-drag-${room.id}")
                        .clearAndSetSemantics {},
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Outlined.DragIndicator, contentDescription = null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                RoomMenu(room, group.isNotEmpty(), canTransfer, actions, onSyncWith, others.isNotEmpty())
            }
            RoomControls(room, actions)
            RoomExtras(room)
        }
    }
}

@Composable
private fun roomLine(room: Room): String {
    val s = room.status
    val t = s?.current
    return when {
        room.error != null && s == null -> stringResource(R.string.room_unreadable)
        s == null -> stringResource(R.string.loading)
        !s.connected -> stringResource(R.string.player_disconnected)
        !s.powered -> stringResource(R.string.player_off)
        t != null -> listOfNotNull(t.title.ifEmpty { null }, t.artist).joinToString(" · ")
        else -> stringResource(R.string.room_nothing)
    }
}

/** Play/pause, power and volume, each named with the room. */
@Composable
private fun RoomControls(room: Room, actions: RoomsActions) {
    val s = room.status ?: return
    val name = room.name
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(horizontal = 4.dp)) {
        IconButton(onClick = { actions.onTogglePlay(room.id) }, enabled = s.connected, modifier = Modifier.testTag("room-play-${room.id}")) {
            Icon(
                if (room.playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow,
                contentDescription = stringResource(if (room.playing) R.string.room_pause else R.string.room_play, name),
            )
        }
        val powerState = stringResource(if (s.powered) R.string.toggle_on else R.string.mode_off)
        IconToggleButton(
            checked = s.powered,
            onCheckedChange = { actions.onPower(room.id, it) },
            enabled = s.connected,
            modifier = Modifier.testTag("room-power-${room.id}").semantics { stateDescription = powerState },
        ) {
            Icon(Icons.Outlined.PowerSettingsNew, contentDescription = stringResource(R.string.power_named, name))
        }
        Icon(
            Icons.AutoMirrored.Outlined.VolumeUp,
            contentDescription = null,
            modifier = Modifier.padding(start = 4.dp).size(20.dp).alpha(0.7f),
        )
        var dragging by remember { mutableStateOf(false) }
        var value by remember { mutableFloatStateOf(0f) }
        val shown = if (dragging) value else s.volume.toFloat()
        val label = stringResource(R.string.room_volume, name)
        val haptics = LocalHapticFeedback.current
        Slider(
            value = shown,
            onValueChange = {
                if (HapticSteps.crossed(if (dragging) value else shown, it, HapticSteps.VOLUME)) haptics.performHapticFeedback(HapticSteps.tick)
                dragging = true
                value = it
            },
            onValueChangeFinished = {
                dragging = false
                actions.onVolume(room.id, value.toInt())
            },
            valueRange = 0f..100f,
            enabled = s.connected,
            modifier = Modifier
                .weight(1f)
                .padding(end = 12.dp)
                .testTag("room-volume-${room.id}")
                .semantics {
                    contentDescription = label
                    stateDescription = "${shown.toInt()}%"
                },
        )
    }
}

/** Sleep remaining and the next alarm, when there are any. */
@Composable
private fun RoomExtras(room: Room) {
    val sleep = SleepTimer.left(room.status)
    val alarm = room.alarm
    if (sleep == null && alarm == null) return
    Row(
        Modifier.fillMaxWidth().padding(horizontal = 16.dp),
        horizontalArrangement = Arrangement.spacedBy(16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (sleep != null) {
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("room-sleep-${room.id}")) {
                Icon(Icons.Outlined.Bedtime, contentDescription = null, modifier = Modifier.size(16.dp))
                Text(
                    stringResource(R.string.room_sleep, sleepLeftText(sleep)),
                    style = MaterialTheme.typography.labelMedium,
                    modifier = Modifier.padding(start = 4.dp),
                )
            }
        }
        if (alarm != null) {
            val locale = currentLocale()
            val text = "${dayName(alarm.day, locale)} ${timeText(alarm.timeSecs.toUInt(), is24Hour(), locale)}"
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.testTag("room-alarm-${room.id}")) {
                Icon(Icons.Outlined.Alarm, contentDescription = null, modifier = Modifier.size(16.dp))
                Text(
                    stringResource(R.string.room_alarm, text),
                    style = MaterialTheme.typography.labelMedium,
                    modifier = Modifier.padding(start = 4.dp),
                )
            }
        }
    }
}

/** The card's menu: Sync with…, Leave the group, Move playback here, Alarms. */
@Composable
private fun RoomMenu(
    room: Room,
    grouped: Boolean,
    canTransfer: Boolean,
    actions: RoomsActions,
    onSyncWith: () -> Unit,
    hasOthers: Boolean,
) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }, modifier = Modifier.testTag("room-menu-${room.id}")) {
            Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.room_more, room.name))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            if (hasOthers) {
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.room_sync_with)) },
                    onClick = {
                        open = false
                        onSyncWith()
                    },
                    modifier = Modifier.testTag("room-sync-${room.id}"),
                )
            }
            if (grouped) {
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.room_unsync)) },
                    onClick = {
                        open = false
                        actions.onUnsync(room.id)
                    },
                    modifier = Modifier.testTag("room-unsync-${room.id}"),
                )
            }
            if (canTransfer) {
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.room_move_here)) },
                    onClick = {
                        open = false
                        actions.onTransfer?.invoke(room.id)
                    },
                    modifier = Modifier.testTag("room-transfer-${room.id}"),
                )
            }
            actions.onAlarms?.let { alarms ->
                DropdownMenuItem(
                    text = { Text(stringResource(R.string.alarms)) },
                    onClick = {
                        open = false
                        alarms(room.id, room.name)
                    },
                )
            }
        }
    }
}

/** Rooms in "Play on…": one button to see every room at a glance. */
@Composable
fun AllRoomsButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    TextButton(onClick = onClick, modifier = modifier.testTag("all-rooms")) {
        Icon(Icons.Outlined.SpeakerGroup, contentDescription = null, modifier = Modifier.size(18.dp))
        Text(stringResource(R.string.rooms_all), Modifier.padding(start = 8.dp))
    }
}
