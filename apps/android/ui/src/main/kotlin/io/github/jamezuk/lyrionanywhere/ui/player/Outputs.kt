package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Input
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material3.Slider
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Alarm
import androidx.compose.material.icons.outlined.PowerSettingsNew
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.LinkOff
import androidx.compose.material3.IconToggleButton
import androidx.compose.ui.platform.testTag
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.Speaker
import androidx.compose.material.icons.outlined.SpeakerGroup
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.Choice
import io.github.jamezuk.lyrionanywhere.ui.components.ChoiceRow

/** The output button (speaker/cast style): opens "Play on…". */
@Composable
fun OutputButton(state: PlayerUiState, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val other = state.controllingOther
    val label = if (other) {
        stringResource(R.string.play_on_other_label, state.selected?.name ?: "")
    } else {
        stringResource(R.string.play_on_title)
    }
    IconButton(onClick = onClick, modifier = modifier) {
        Icon(
            if (other) Icons.Outlined.Speaker else Icons.Outlined.Devices,
            contentDescription = label,
            tint = if (other) MaterialTheme.colorScheme.tertiary else MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

/** What the "Play on…" sheet can do. */
data class PlayOnActions(
    val onControl: (String) -> Unit,
    val onBackToPhone: () -> Unit,
    /** "Move playback to <player>": move what the controlled player plays there (Transfer). */
    val onMoveHere: ((String) -> Unit)? = null,
    /** "Move playback here" on this phone's row (Transfer). */
    val onBringToPhone: (() -> Unit)? = null,
    /** Join this player to the controlled player's group (true), or take it out (false). */
    val onSync: ((String, Boolean) -> Unit)? = null,
    val onDismissTransfer: () -> Unit = {},
    /** Every room at a glance (Wave 2 §4.3); null: not offered. */
    val onAllRooms: (() -> Unit)? = null,
)

/** One entry of a player row's ⋮. */
private class RowItem(val label: String, val icon: androidx.compose.ui.graphics.vector.ImageVector, val enabled: Boolean, val tag: String, val run: () -> Unit)

/**
 * A player row's ⋮ (its one trailing control, Wave 3.5): the secondary actions (tapping the
 * row is the primary one, controlling it): Move playback to <player>, and, with Rooms folded
 * in, play or pause it, its alarms and its power. Shows nothing when there is nothing in it.
 */
@Composable
private fun PlayerRowMenu(name: String, items: List<RowItem>, tag: String) {
    if (items.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }, modifier = Modifier.testTag("$tag-more")) {
            Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.more_for, name))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            items.forEach { item ->
                DropdownMenuItem(
                    text = { Text(item.label) },
                    leadingIcon = { Icon(item.icon, contentDescription = null) },
                    enabled = item.enabled,
                    onClick = {
                        open = false
                        item.run()
                    },
                    modifier = Modifier.testTag("$tag-${item.tag}"),
                )
            }
        }
    }
}

/** A player's volume in Play on… (the controlled player, and any playing one). */
@Composable
private fun RowVolume(room: Room, onVolume: (String, Int) -> Unit) {
    val s = room.status ?: return
    var dragging by remember { mutableStateOf(false) }
    var value by remember { mutableFloatStateOf(0f) }
    val shown = if (dragging) value else s.volume.toFloat()
    val label = stringResource(R.string.room_volume, room.name)
    Row(verticalAlignment = Alignment.CenterVertically) {
        Icon(Icons.AutoMirrored.Outlined.VolumeUp, contentDescription = null, modifier = Modifier.size(18.dp))
        Slider(
            value = shown,
            onValueChange = {
                dragging = true
                value = it
            },
            onValueChangeFinished = {
                dragging = false
                onVolume(room.id, value.toInt())
            },
            valueRange = 0f..100f,
            enabled = s.connected,
            modifier = Modifier
                .padding(start = 8.dp)
                .weight(1f)
                .testTag("play-on-volume-${room.id}")
                .semantics {
                    contentDescription = label
                    stateDescription = "${shown.toInt()}%"
                },
        )
    }
}

/**
 * "Play on…" (Wave 3.5), the one player manager: this phone and the other players this
 * account may control. Tapping a row controls that player; its ⋮ moves playback there and,
 * with [rooms] (Rooms folded in), plays or pauses it, opens its alarms and switches it on or
 * off; the controlled player and any playing one have a volume slider. *Group players…* joins
 * players to the controlled one's group. *All rooms* (the expanded view) is offered by the
 * caller on large windows only.
 */
@Composable
fun PlayOnList(
    state: PlayerUiState,
    actions: PlayOnActions,
    modifier: Modifier = Modifier,
    rooms: List<Room> = emptyList(),
    roomActions: RoomsActions? = null,
) {
    var grouping by rememberSaveable { mutableStateOf(false) }
    val busy = state.transfer is TransferState.Running
    val hasQueue = state.status?.current != null
    Column(modifier.fillMaxWidth().selectableGroup()) {
        Text(
            stringResource(R.string.play_on_title),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp).semantics { heading() },
        )
        TransferStatus(state.transfer, actions.onDismissTransfer)
        val phone = state.localSelected || (state.selectedId == null && state.waitingForPhone)
        val phoneName = stringResource(R.string.this_phone)
        val phoneItems = buildList {
            // What the "Playing on" strip offered: move what the other player plays here.
            actions.onBringToPhone?.takeIf { state.controllingOther && hasQueue && !phone }?.let { bring ->
                add(RowItem(stringResource(R.string.move_playback_to_phone), Icons.AutoMirrored.Outlined.Input, !busy, "move", bring))
            }
        }
        ListItem(
            leadingContent = { Icon(if (phone) Icons.Outlined.Check else Icons.Outlined.PhoneAndroid, contentDescription = null) },
            headlineContent = { Text(phoneName) },
            supportingContent = {
                Column {
                    Text(playOnPhoneDetail(state.local))
                    CushionLine(state.local.cushionMs, Modifier.testTag("phone-cushion"))
                }
            },
            trailingContent = { PlayerRowMenu(phoneName, phoneItems, tag = "play-on-phone") },
            colors = ListItemDefaults.colors(containerColor = Color.Transparent),
            modifier = Modifier.selectable(selected = phone, role = Role.RadioButton, onClick = actions.onBackToPhone),
        )
        val others = state.otherPlayers
        if (others.isEmpty()) {
            Text(
                stringResource(R.string.no_other_players),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 24.dp, vertical = 12.dp),
            )
        }
        others.forEach { p ->
            val selected = p.id == state.selectedId
            val room = rooms.firstOrNull { it.id == p.id }
            val status = room?.status
            val items = buildList {
                actions.onMoveHere?.takeIf { state.selectedId != null && !selected }?.let { m ->
                    add(RowItem(stringResource(R.string.move_here_named, p.name), Icons.AutoMirrored.Outlined.Input, !busy && hasQueue, "move") { m(p.id) })
                }
                if (roomActions != null && status != null && status.connected) {
                    add(
                        RowItem(
                            stringResource(if (room.playing) R.string.room_pause else R.string.room_play, p.name),
                            if (room.playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow,
                            true,
                            "play",
                        ) { roomActions.onTogglePlay(p.id) },
                    )
                    roomActions.onAlarms?.let { alarms ->
                        add(RowItem(stringResource(R.string.alarms), Icons.Outlined.Alarm, true, "alarms") { alarms(p.id, p.name) })
                    }
                    add(
                        RowItem(
                            stringResource(if (status.powered) R.string.room_switch_off else R.string.room_switch_on, p.name),
                            Icons.Outlined.PowerSettingsNew,
                            true,
                            "power",
                        ) { roomActions.onPower(p.id, !status.powered) },
                    )
                }
            }
            ListItem(
                leadingContent = { Icon(if (selected) Icons.Outlined.Check else Icons.Outlined.Speaker, contentDescription = null) },
                headlineContent = { Text(p.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                supportingContent = {
                    Column {
                        Text(playerLine(p, state), maxLines = 1, overflow = TextOverflow.Ellipsis)
                        // The controlled player and any playing one: their volume, at hand.
                        if (roomActions != null && room != null && status?.powered == true && (selected || room.playing)) {
                            RowVolume(room, roomActions.onVolume)
                        }
                    }
                },
                trailingContent = { PlayerRowMenu(p.name, items, tag = "play-on-${p.id}") },
                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                modifier = Modifier.selectable(selected = selected, role = Role.RadioButton) { actions.onControl(p.id) },
            )
        }
        // Grouping: one row, a dialog of who plays in step with the controlled player.
        val sync = actions.onSync
        val current = state.selected
        if (sync != null && current != null && others.any { it.id != current.id }) {
            val group = state.syncGroups.firstOrNull { current.id in it.playerIds }
                ?.playerIds?.mapNotNull { id -> state.players.firstOrNull { it.id == id }?.name }
                ?.takeIf { it.size > 1 }
            ListItem(
                leadingContent = { Icon(Icons.Outlined.SpeakerGroup, contentDescription = null) },
                headlineContent = { Text(stringResource(R.string.group_players)) },
                supportingContent = {
                    Text(group?.joinToString(" + ") ?: stringResource(R.string.group_players_detail, current.name), maxLines = 2, overflow = TextOverflow.Ellipsis)
                },
                colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                modifier = Modifier.clickable(enabled = !busy, role = Role.Button) { grouping = true }.testTag("group-players"),
            )
        }
        actions.onAllRooms?.let { AllRoomsButton(it, Modifier.padding(horizontal = 16.dp)) }
    }
    if (grouping && actions.onSync != null) {
        GroupPlayersDialog(state, actions.onSync, onDismiss = { grouping = false })
    }
}

/** *Group players…*: each other player, ticked while it plays in step with the controlled one. */
@Composable
private fun GroupPlayersDialog(state: PlayerUiState, onSync: (String, Boolean) -> Unit, onDismiss: () -> Unit) {
    val current = state.selected ?: return
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.group_players_title, current.name)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                state.otherPlayers.filter { it.id != current.id }.forEach { p ->
                    val together = SyncChoice.together(state.selectedId, p.id, state.syncGroups)
                    val label = stringResource(R.string.sync_with, p.name, current.name)
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        modifier = Modifier
                            .fillMaxWidth()
                            .toggleable(value = together, role = Role.Checkbox) { onSync(p.id, it) }
                            .semantics { contentDescription = label }
                            .padding(vertical = 8.dp)
                            .testTag("sync-${p.id}"),
                    ) {
                        Checkbox(checked = together, onCheckedChange = null)
                        Text(p.name, modifier = Modifier.padding(start = 12.dp))
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_done)) } },
    )
}

@Composable
private fun playerLine(p: Player, state: PlayerUiState): String {
    val t = state.playingOn[p.id]
    val line = when {
        !p.connected -> stringResource(R.string.player_disconnected)
        p.playing && t != null -> listOfNotNull(t.title.ifEmpty { null }, t.artist).joinToString(" · ")
        p.playing -> stringResource(R.string.player_playing)
        !p.powered -> stringResource(R.string.player_off)
        else -> stringResource(R.string.player_idle)
    }
    // A grouped player says who it plays in step with (the groups aren't listed apart).
    val with = state.syncGroups.firstOrNull { p.id in it.playerIds }?.playerIds
        ?.filter { it != p.id }
        ?.mapNotNull { id -> state.players.firstOrNull { it.id == id }?.name }
        .orEmpty()
    return if (with.isEmpty()) line else stringResource(R.string.player_in_step_with, line, with.joinToString(", "))
}

/** Settings › Playback: which player the app controls when it starts (per device, D20), a row with a picker. */
@Composable
fun DefaultPlayerSetting(
    state: PlayerUiState,
    onChoose: (DefaultPlayer, String?) -> Unit,
    modifier: Modifier = Modifier,
) {
    // A choice is the setting and, for a specific player, its id.
    val choices = buildList<Choice<Pair<DefaultPlayer, String?>>> {
        add(Choice(DefaultPlayer.THIS_PHONE to null, stringResource(R.string.default_player_phone), stringResource(R.string.default_player_phone_detail), tag = "default-player-phone"))
        add(Choice(DefaultPlayer.LAST_USED to null, stringResource(R.string.default_player_last), stringResource(R.string.default_player_last_detail), tag = "default-player-last"))
        state.otherPlayers.forEach { p -> add(Choice(DefaultPlayer.SPECIFIC to p.id, p.name, tag = "default-player-${p.id}")) }
    }
    val selected = when (state.defaultPlayer) {
        DefaultPlayer.SPECIFIC -> DefaultPlayer.SPECIFIC to state.defaultPlayerId
        else -> state.defaultPlayer to null
    }
    // A chosen player that isn't listed now (switched off, or access removed).
    val missing = state.defaultPlayerId?.takeIf { id ->
        state.defaultPlayer == DefaultPlayer.SPECIFIC && state.otherPlayers.none { it.id == id }
    }
    ChoiceRow(
        title = stringResource(R.string.section_default_player),
        choices = choices,
        selected = selected,
        onSelect = { (d, id) -> onChoose(d, id) },
        modifier = modifier,
        icon = Icons.Outlined.Devices,
        tag = "default-player-setting",
        current = missing?.let { stringResource(R.string.default_player_missing) },
    )
}
