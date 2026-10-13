package io.github.jamezuk.lyrionanywhere.ui.library

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.PlaylistAdd
import androidx.compose.material.icons.automirrored.outlined.PlaylistPlay
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SwipeToDismissBox
import androidx.compose.material3.SwipeToDismissBoxValue
import androidx.compose.material3.rememberSwipeToDismissBoxState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.ui.R
import kotlinx.coroutines.launch

/**
 * A browse row's swipes (Wave 2 §3.4, Squeezer style): right to play it next, left to add
 * it to the queue. The row springs back either way. The same two are TalkBack actions on
 * the row ([content] puts the modifier it is given on its clickable node), and the row's
 * menu offers them too; a swipe is never the only way. Without [onQueue], a plain row.
 */
@Composable
fun BrowseSwipe(
    onQueue: ((Placement) -> Unit)?,
    modifier: Modifier = Modifier,
    content: @Composable (Modifier) -> Unit,
) {
    if (onQueue == null) {
        Box(modifier) { content(Modifier) }
        return
    }
    val state = rememberSwipeToDismissBoxState()
    val scope = rememberCoroutineScope()
    val haptics = LocalHapticFeedback.current
    val playNext = stringResource(R.string.action_play_next)
    val add = stringResource(R.string.action_add_to_queue)
    SwipeToDismissBox(
        state = state,
        modifier = modifier,
        onDismiss = {
            when (it) {
                SwipeToDismissBoxValue.StartToEnd -> {
                    haptics.performHapticFeedback(HapticFeedbackType.GestureEnd)
                    onQueue(Placement.PLAY_NEXT)
                }
                SwipeToDismissBoxValue.EndToStart -> {
                    haptics.performHapticFeedback(HapticFeedbackType.GestureEnd)
                    onQueue(Placement.ADD_TO_QUEUE)
                }
                else -> {}
            }
            // Nothing leaves the list: the row comes back.
            scope.launch { state.snapTo(SwipeToDismissBoxValue.Settled) }
        },
        backgroundContent = {
            val right = state.dismissDirection == SwipeToDismissBoxValue.StartToEnd
            Box(
                Modifier
                    .fillMaxSize()
                    .background(if (right) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.secondaryContainer)
                    .padding(horizontal = 24.dp),
                contentAlignment = if (right) Alignment.CenterStart else Alignment.CenterEnd,
            ) {
                Icon(
                    if (right) Icons.AutoMirrored.Outlined.PlaylistPlay else Icons.AutoMirrored.Outlined.PlaylistAdd,
                    contentDescription = null,
                    tint = if (right) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSecondaryContainer,
                )
            }
        },
    ) {
        Box(Modifier.background(MaterialTheme.colorScheme.background)) {
            content(
                Modifier.semantics {
                    customActions = listOf(
                        CustomAccessibilityAction(playNext) { onQueue(Placement.PLAY_NEXT); true },
                        CustomAccessibilityAction(add) { onQueue(Placement.ADD_TO_QUEUE); true },
                    )
                },
            )
        }
    }
}
