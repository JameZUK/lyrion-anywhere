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

/**
 * A player row's ⋮ (its one trailing control): "Move playback to <player>" (the secondary
 * action, Wave 3.5: tapping the row is the primary one, controlling it). Shows nothing when
 * there is nothing to offer.
 */
@Composable
private fun PlayerRowMenu(
    name: String,
    move: (() -> Unit)?,
    moveEnabled: Boolean,
    tag: String,
    moveLabel: String = stringResource(R.string.move_here_named, name),
) {
    if (move == null) return
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }, modifier = Modifier.testTag("$tag-more")) {
            Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.more_for, name))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text(moveLabel) },
                leadingIcon = { Icon(Icons.AutoMirrored.Outlined.Input, contentDescription = null) },
                enabled = moveEnabled,
                onClick = {
                    open = false
                    move()
                },
                modifier = Modifier.testTag("$tag-move"),
            )
        }
    }
}

/**
 * "Play on…" (Wave 3.5): this phone and the other players this account may control. Tapping a
 * row controls that player; its ⋮ moves playback there. *Group players…* joins players to the
 * controlled one's group, and a grouped player says who it plays with.
 */
@Composable
fun PlayOnList(state: PlayerUiState, actions: PlayOnActions, modifier: Modifier = Modifier) {
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
        ListItem(
            leadingContent = { Icon(Icons.Outlined.PhoneAndroid, contentDescription = null) },
            headlineContent = { Text(phoneName) },
            supportingContent = {
                Column {
                    Text(playOnPhoneDetail(state.local))
                    CushionLine(state.local.cushionMs, Modifier.testTag("phone-cushion"))
                }
            },
            trailingContent = {
                if (phone) {
                    Icon(Icons.Outlined.Check, contentDescription = null)
                } else {
                    // What the "Playing on" strip offered: move what the other player plays here.
                    val bring = actions.onBringToPhone?.takeIf { state.controllingOther && hasQueue }
                    PlayerRowMenu(phoneName, bring, moveEnabled = !busy, tag = "play-on-phone", moveLabel = stringResource(R.string.move_playback_to_phone))
                }
            },
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
            ListItem(
                leadingContent = { Icon(Icons.Outlined.Speaker, contentDescription = null) },
                headlineContent = { Text(p.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                supportingContent = { Text(playerLine(p, state), maxLines = 1, overflow = TextOverflow.Ellipsis) },
                trailingContent = {
                    if (selected) {
                        Icon(Icons.Outlined.Check, contentDescription = null)
                    } else {
                        val move = actions.onMoveHere?.takeIf { state.selectedId != null }?.let { m -> { m(p.id) } }
                        PlayerRowMenu(p.name, move, moveEnabled = !busy && hasQueue, tag = "play-on-${p.id}")
                    }
                },
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

/** A compact "now playing" card for Home: small, so the library comes first (D20). */
@Composable
fun CompactPlayerCard(state: PlayerUiState, onOpen: () -> Unit, modifier: Modifier = Modifier) {
    val track = state.status?.current ?: return
    ListItem(
        overlineContent = {
            Text(
                if (state.controllingOther) {
                    stringResource(R.string.playing_on, state.selected?.name ?: "")
                } else {
                    stringResource(if (state.playing) R.string.now_playing else R.string.paused)
                },
            )
        },
        headlineContent = { Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
        supportingContent = track.artist?.let { { Text(it, maxLines = 1, overflow = TextOverflow.Ellipsis) } },
        leadingContent = {
            ArtworkImage(
                track.artwork,
                120,
                contentDescription = null,
                modifier = Modifier.size(48.dp),
                corner = 6.dp,
            )
        },
        modifier = modifier.clickable(onClickLabel = stringResource(R.string.open_now_playing), onClick = onOpen),
    )
}
