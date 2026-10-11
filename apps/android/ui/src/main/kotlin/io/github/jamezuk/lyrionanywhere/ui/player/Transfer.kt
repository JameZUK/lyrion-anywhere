package io.github.jamezuk.lyrionanywhere.ui.player

import android.content.res.Resources
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.TransferRefusal
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.errorText

/** Where a transfer (docs/PROTOCOL.md › Transfer) has got to, for the UI. */
sealed interface TransferState {
    data object Idle : TransferState

    /** Moving playback to [targetName]; [stage] says how far it has got. */
    data class Running(val targetId: String, val targetName: String, val stage: TransferStage) : TransferState

    /** It wasn't done, and nothing was changed. */
    data class Refused(val targetName: String, val reason: TransferRefusal) : TransferState

    /** It failed part-way or couldn't start; [error] carries the detail. */
    data class Failed(val targetName: String, val error: AppError) : TransferState

    /** "Bring to this phone" before the phone is an LMS player (it is being started). */
    data object PhoneNotReady : TransferState
}

/** The detail an error carries (the network layer's or the server's reason), if any. */
fun errorDetail(error: AppError): String? = when (error) {
    is AppError.Offline -> error.detail
    is AppError.Forbidden -> error.detail
    is AppError.Server -> error.detail
    is AppError.InvalidInput -> null
    is AppError.Unexpected -> error.detail
    AppError.SignedOut, AppError.TwoStepRequired, is AppError.RateLimited -> null
}?.trim()?.take(200)?.takeIf { it.isNotEmpty() }

/** A message for [error] with its detail on a second line (for failures worth diagnosing). */
fun errorWithDetail(res: Resources, error: AppError): String {
    val text = errorText(res, error)
    val detail = errorDetail(error) ?: return text
    // Offline already shows its detail.
    return if (detail in text) text else "$text\n($detail)"
}

/** What to say about a transfer (null while idle). */
fun transferText(res: Resources, state: TransferState): String? = when (state) {
    TransferState.Idle -> null
    is TransferState.Running -> res.getString(
        when (state.stage) {
            TransferStage.READING -> R.string.transfer_reading
            TransferStage.GROUPING -> R.string.transfer_grouping
            TransferStage.STARTING -> R.string.transfer_starting
            TransferStage.COPYING -> R.string.transfer_copying
        },
        state.targetName,
    )
    is TransferState.Refused -> when (state.reason) {
        TransferRefusal.SAME -> res.getString(R.string.transfer_same, state.targetName)
        TransferRefusal.EMPTY -> res.getString(R.string.transfer_empty)
        TransferRefusal.REMOTE -> res.getString(R.string.transfer_remote, state.targetName)
        TransferRefusal.TOO_LONG -> res.getString(R.string.transfer_too_long, state.targetName)
        TransferRefusal.NOT_GROUPED -> res.getString(R.string.transfer_not_grouped, state.targetName)
    }
    is TransferState.Failed -> res.getString(R.string.transfer_failed, state.targetName) + "\n" +
        errorWithDetail(res, state.error)
    TransferState.PhoneNotReady -> res.getString(R.string.transfer_phone_not_ready)
}

/**
 * The transfer's progress, or why it failed (with Dismiss). Shows nothing while idle.
 * Announced politely to screen readers as it changes.
 */
@Composable
fun TransferStatus(state: TransferState, onDismiss: () -> Unit, modifier: Modifier = Modifier) {
    val text = transferText(LocalResources.current, state) ?: return
    val running = state is TransferState.Running
    val colours = MaterialTheme.colorScheme
    Column(
        modifier
            .fillMaxWidth()
            .background(if (running) colours.secondaryContainer else colours.errorContainer)
            .testTag("transfer-status")
            .semantics { liveRegion = LiveRegionMode.Polite },
    ) {
        if (running) LinearProgressIndicator(Modifier.fillMaxWidth())
        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(start = 16.dp, end = 4.dp)) {
            Text(
                text,
                style = MaterialTheme.typography.bodyMedium,
                color = if (running) colours.onSecondaryContainer else colours.onErrorContainer,
                modifier = Modifier.weight(1f).padding(vertical = 10.dp),
            )
            if (!running) {
                TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_dismiss)) }
            }
        }
    }
}
