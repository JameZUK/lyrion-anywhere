package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Devices
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
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage

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

/**
 * Says plainly when another player is being controlled (D20), with one tap back to this
 * phone. Shows nothing while this phone is the player.
 */
@Composable
fun PlayingOnStrip(state: PlayerUiState, onBackToPhone: () -> Unit, modifier: Modifier = Modifier) {
    if (!state.controllingOther) return
    val name = state.selected?.name ?: return
    val bg = MaterialTheme.colorScheme.tertiaryContainer
    val fg = MaterialTheme.colorScheme.onTertiaryContainer
    Row(
        modifier
            .fillMaxWidth()
            .background(bg)
            .padding(start = 16.dp, end = 4.dp)
            .semantics(mergeDescendants = false) { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Outlined.Speaker, contentDescription = null, tint = fg)
        Text(
            stringResource(R.string.playing_on, name),
            color = fg,
            style = MaterialTheme.typography.labelLarge,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
        )
        TextButton(onClick = onBackToPhone) {
            Text(stringResource(R.string.back_to_phone), color = fg)
        }
    }
}

/** What the "Play on…" sheet can do. Each player row has room for more actions later
 *  (e.g. "Move playback here"), next to "Control". */
data class PlayOnActions(
    val onControl: (String) -> Unit,
    val onBackToPhone: () -> Unit,
    /** Extra actions for a player row (none yet). */
    val rowActions: @Composable RowScope.(Player) -> Unit = {},
)

/** "Play on…": this phone, the other players this account may control, and sync groups. */
@Composable
fun PlayOnList(state: PlayerUiState, actions: PlayOnActions, modifier: Modifier = Modifier) {
    Column(modifier.fillMaxWidth().selectableGroup()) {
        Text(
            stringResource(R.string.play_on_title),
            style = MaterialTheme.typography.titleLarge,
            modifier = Modifier.padding(horizontal = 24.dp, vertical = 8.dp).semantics { heading() },
        )
        val phone = state.localSelected || (state.selectedId == null && state.waitingForPhone)
        ListItem(
            leadingContent = { Icon(Icons.Outlined.PhoneAndroid, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.this_phone)) },
            supportingContent = { Text(playOnPhoneDetail(state.local)) },
            trailingContent = { if (phone) Icon(Icons.Outlined.Check, contentDescription = null) },
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
            Row(verticalAlignment = Alignment.CenterVertically) {
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.Speaker, contentDescription = null) },
                    headlineContent = { Text(p.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    supportingContent = { Text(playerLine(p, state), maxLines = 1, overflow = TextOverflow.Ellipsis) },
                    trailingContent = { if (selected) Icon(Icons.Outlined.Check, contentDescription = null) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    modifier = Modifier
                        .weight(1f)
                        .selectable(selected = selected, role = Role.RadioButton) {
                            actions.onControl(p.id)
                        },
                )
                actions.rowActions(this, p)
            }
        }
        val groups = state.syncGroups.mapNotNull { g ->
            val names = g.playerIds.mapNotNull { id -> state.players.firstOrNull { it.id == id }?.name }
            if (names.size > 1) g.playerIds.first() to names.joinToString(" + ") else null
        }
        if (groups.isNotEmpty()) {
            Text(
                stringResource(R.string.sync_groups),
                style = MaterialTheme.typography.titleSmall,
                color = MaterialTheme.colorScheme.primary,
                modifier = Modifier.padding(start = 24.dp, top = 16.dp, bottom = 4.dp).semantics { heading() },
            )
            groups.forEach { (first, names) ->
                ListItem(
                    leadingContent = { Icon(Icons.Outlined.SpeakerGroup, contentDescription = null) },
                    headlineContent = { Text(names, maxLines = 2, overflow = TextOverflow.Ellipsis) },
                    supportingContent = { Text(stringResource(R.string.sync_group_detail)) },
                    colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                    modifier = Modifier.selectable(
                        selected = first == state.selectedId,
                        role = Role.RadioButton,
                    ) { actions.onControl(first) },
                )
            }
        }
    }
}

@Composable
private fun playerLine(p: Player, state: PlayerUiState): String {
    val t = state.playingOn[p.id]
    return when {
        !p.connected -> stringResource(R.string.player_disconnected)
        p.playing && t != null -> listOfNotNull(t.title.ifEmpty { null }, t.artist).joinToString(" · ")
        p.playing -> stringResource(R.string.player_playing)
        !p.powered -> stringResource(R.string.player_off)
        else -> stringResource(R.string.player_idle)
    }
}

/** Settings: which player the app controls when it starts (per device, D20). */
@Composable
fun DefaultPlayerSetting(
    state: PlayerUiState,
    onChoose: (DefaultPlayer, String?) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier.selectableGroup()) {
        @Composable
        fun option(label: String, detail: String?, selected: Boolean, onClick: () -> Unit) {
            ListItem(
                headlineContent = { Text(label) },
                supportingContent = detail?.let { { Text(it) } },
                leadingContent = { RadioButton(selected = selected, onClick = null) },
                modifier = Modifier.selectable(selected = selected, role = Role.RadioButton, onClick = onClick),
            )
        }
        option(
            stringResource(R.string.default_player_phone),
            stringResource(R.string.default_player_phone_detail),
            state.defaultPlayer == DefaultPlayer.THIS_PHONE,
        ) { onChoose(DefaultPlayer.THIS_PHONE, null) }
        option(
            stringResource(R.string.default_player_last),
            stringResource(R.string.default_player_last_detail),
            state.defaultPlayer == DefaultPlayer.LAST_USED,
        ) { onChoose(DefaultPlayer.LAST_USED, null) }
        state.otherPlayers.forEach { p ->
            option(
                p.name,
                null,
                state.defaultPlayer == DefaultPlayer.SPECIFIC && state.defaultPlayerId == p.id,
            ) { onChoose(DefaultPlayer.SPECIFIC, p.id) }
        }
        // A chosen player that isn't listed now (switched off, or access removed).
        val missing = state.defaultPlayerId?.takeIf { id ->
            state.defaultPlayer == DefaultPlayer.SPECIFIC && state.otherPlayers.none { it.id == id }
        }
        if (missing != null) {
            Text(
                stringResource(R.string.default_player_missing),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
        }
    }
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
