package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState

/** What the Queue tab can do. */
data class QueueScreenActions(
    val queue: QueueActions,
    val onClearUpcoming: () -> Unit = {},
    /** Open "Save as playlist" (offered only when [PlayerUiState.canSaveQueue]). */
    val onSaveQueue: () -> Unit = {},
    /** No player chosen yet: open *Play on…*. */
    val onChoosePlayer: () -> Unit = {},
    val onRetry: () -> Unit = {},
)

/**
 * The Queue tab (the same four tabs as the web app: Home, Library, Search, Queue): the
 * selected player's queue, with its sections (Now playing, Up next, Then), swipes (right to
 * play next, left to remove, with Undo), drag to reorder and Clear, exactly as under Now
 * playing.
 */
@Composable
fun QueueScreen(state: PlayerUiState, actions: QueueScreenActions, modifier: Modifier = Modifier) {
    val player = state.selected
    val status = state.status
    if (player == null) {
        if (state.waitingForPhone) {
            LoadingState(modifier)
            return
        }
        Column(
            modifier.fillMaxSize().padding(24.dp).testTag("queue-no-player"),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
        ) {
            Text(stringResource(R.string.choose_player_first), style = MaterialTheme.typography.bodyLarge, textAlign = TextAlign.Center)
            Button(onClick = actions.onChoosePlayer) {
                Icon(Icons.Outlined.Devices, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(stringResource(R.string.action_choose_player), Modifier.padding(start = 8.dp))
            }
        }
        return
    }
    if (status == null) {
        if (state.error != null) ErrorState(state.error, actions.onRetry, modifier) else LoadingState(modifier)
        return
    }
    val drag = remember { QueueDrag() }
    LazyColumn(
        modifier.fillMaxSize().testTag("queue-screen"),
        contentPadding = PaddingValues(vertical = 8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        item(key = "queue-header") {
            Column(Modifier.widthIn(max = 840.dp).fillMaxWidth()) {
                QueueHeader(
                    state.queue.size,
                    player.name,
                    actions.queue.onClear,
                    state.upcoming,
                    actions.onClearUpcoming,
                    onSaveAsPlaylist = actions.onSaveQueue.takeIf { state.canSaveQueue },
                )
            }
        }
        queueItems(state.queue, status.queueIndex?.toInt(), drag, actions.queue, hasMore = state.queueMore, sections = state.sections)
    }
}
