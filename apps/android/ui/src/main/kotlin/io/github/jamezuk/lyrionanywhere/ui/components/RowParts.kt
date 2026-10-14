package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.PlaylistAdd
import androidx.compose.material.icons.automirrored.outlined.PlaylistPlay
import androidx.compose.material.icons.filled.DownloadForOffline
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.OfflinePin
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.composed
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R

/**
 * The one "available offline" mark (Wave 3.5 rule 2): a small filled ↓ in a circle, on any
 * track row whose track is downloaded. It says so to TalkBack too.
 */
@Composable
fun OfflineMark(modifier: Modifier = Modifier) {
    Icon(
        Icons.Filled.DownloadForOffline,
        contentDescription = stringResource(R.string.downloaded),
        tint = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.padding(end = 8.dp).size(18.dp),
    )
}

/**
 * The cache mark: kept on this phone from streaming. Shown only while your music server is
 * out of reach (Wave 3.5 rule 2), when it says the row can still be played.
 */
@Composable
fun CacheMark(modifier: Modifier = Modifier) {
    Icon(
        Icons.Outlined.OfflinePin,
        contentDescription = stringResource(R.string.queue_mark_on_device),
        tint = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.padding(end = 8.dp).size(18.dp),
    )
}

/**
 * A row's ⋮ with where else the item can go in the queue: Play next and Add to queue (its one
 * trailing control, Wave 3.5 rule 3).
 */
@Composable
fun PlacementMenu(name: String, onPlayNext: () -> Unit, onAdd: () -> Unit, modifier: Modifier = Modifier) {
    var open by remember { mutableStateOf(false) }
    Box(modifier) {
        IconButton(onClick = { open = true }, modifier = Modifier.testTag("placement-menu")) {
            Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.more_for, name))
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.action_play_next)) },
                leadingIcon = { Icon(Icons.AutoMirrored.Outlined.PlaylistPlay, contentDescription = null) },
                onClick = {
                    open = false
                    onPlayNext()
                },
                modifier = Modifier.testTag("placement-next"),
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.action_add_to_queue)) },
                leadingIcon = { Icon(Icons.AutoMirrored.Outlined.PlaylistAdd, contentDescription = null) },
                onClick = {
                    open = false
                    onAdd()
                },
                modifier = Modifier.testTag("placement-add"),
            )
        }
    }
}

/** TalkBack's way to Play next and Add to queue on a row (as its ⋮ offers them). */
fun Modifier.placementActions(name: String, onPlayNext: () -> Unit, onAdd: () -> Unit): Modifier = composed {
    val next = stringResource(R.string.menu_play_next, name)
    val add = stringResource(R.string.add_named, name)
    semantics {
        customActions = listOf(
            CustomAccessibilityAction(next) { onPlayNext(); true },
            CustomAccessibilityAction(add) { onAdd(); true },
        )
    }
}
