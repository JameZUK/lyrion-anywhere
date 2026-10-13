package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Album
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.ui.R

/** What a shuffle mode is called, as the button's state and in its menu. */
@Composable
fun shuffleState(mode: ShuffleMode): String = stringResource(
    when (mode) {
        ShuffleMode.OFF -> R.string.mode_off
        ShuffleMode.SONGS -> R.string.shuffle_songs
        ShuffleMode.ALBUMS -> R.string.shuffle_albums
    },
)

/**
 * The shuffle button (Wave 2 §3.5): a tap cycles Off → Tracks → Albums; a long press (or
 * TalkBack's long-press action) offers the three. Its name stays "Shuffle"; its state says
 * which, and the icon shows it too (an album mark for Albums), so it never relies on colour.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun ShuffleButton(mode: ShuffleMode, onCycle: () -> Unit, onChoose: (ShuffleMode) -> Unit) {
    var open by remember { mutableStateOf(false) }
    val haptics = LocalHapticFeedback.current
    val label = stringResource(R.string.shuffle)
    val state = shuffleState(mode)
    val on = mode != ShuffleMode.OFF
    val tint = if (on) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant
    Box {
        Box(
            Modifier
                .size(48.dp)
                .clip(CircleShape)
                .testTag("shuffle-button")
                .combinedClickable(
                    role = Role.Button,
                    onLongClickLabel = stringResource(R.string.shuffle_choose),
                    onLongClick = {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        open = true
                    },
                    onClick = onCycle,
                )
                .semantics {
                    contentDescription = label
                    stateDescription = state
                },
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Outlined.Shuffle, contentDescription = null, tint = tint)
            if (mode == ShuffleMode.ALBUMS) {
                Icon(
                    Icons.Outlined.Album,
                    contentDescription = null,
                    tint = tint,
                    modifier = Modifier.align(Alignment.BottomEnd).padding(6.dp).size(14.dp).testTag("shuffle-albums-mark"),
                )
            }
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            listOf(
                ShuffleMode.OFF to R.string.shuffle_option_off,
                ShuffleMode.SONGS to R.string.shuffle_option_tracks,
                ShuffleMode.ALBUMS to R.string.shuffle_option_albums,
            ).forEach { (m, text) ->
                DropdownMenuItem(
                    text = { Text(stringResource(text)) },
                    onClick = {
                        open = false
                        onChoose(m)
                    },
                    trailingIcon = if (m == mode) {
                        { Icon(Icons.Outlined.Check, contentDescription = null) }
                    } else {
                        null
                    },
                    modifier = Modifier.semantics { selected = m == mode },
                )
            }
        }
    }
}

/**
 * The queue peek (Wave 2 §3.4): the queue in a sheet over Now playing, opened by swiping up
 * on the artwork or with the Queue button. The same rows, sections and actions as the
 * queue below Now playing.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun QueuePeekSheet(state: PlayerUiState, actions: NowPlayingActions, onDismiss: () -> Unit) {
    val drag = remember { QueueDrag() }
    val queueActions = actions.queueActions()
    ModalBottomSheet(onDismissRequest = onDismiss, modifier = Modifier.testTag("queue-peek")) {
        LazyColumn(Modifier.fillMaxWidth(), contentPadding = PaddingValues(bottom = 24.dp)) {
            item(key = "queue-header") {
                QueueHeader(
                    state.queue.size,
                    state.selected?.name.orEmpty(),
                    actions.onClearQueue,
                    upcoming = state.upcoming,
                    onClearUpcoming = actions.onClearUpcoming,
                )
            }
            queueItems(
                state.queue,
                state.status?.queueIndex?.toInt(),
                drag,
                queueActions,
                hasMore = state.queueMore,
                sections = state.sections,
            )
        }
    }
}

/** The queue's actions, from Now playing's. */
internal fun NowPlayingActions.queueActions() =
    QueueActions(onJump, onRemove, onMove, onClearQueue, onLoadMoreQueue, onPlayNext = onPlayNextInQueue)
