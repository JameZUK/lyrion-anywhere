package io.github.jamezuk.lyrionanywhere.ui.onboarding

import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.res.stringResource
import io.github.jamezuk.lyrionanywhere.ui.R

/**
 * A pairing link opened while this phone is signed in. Using it means signing out here
 * first, so the user is asked; declining leaves everything as it was. Continuing only signs
 * out: the pairing screen then shows the link's server and asks again before pairing.
 *
 * [target] is the server the link names (null: the one this phone uses).
 */
@Composable
fun SwitchAccountDialog(current: String, target: String?, onContinue: () -> Unit, onCancel: () -> Unit) {
    AlertDialog(
        onDismissRequest = onCancel,
        title = { Text(stringResource(R.string.pair_link_switch_title)) },
        text = {
            Text(
                if (target == null || target == current) {
                    stringResource(R.string.pair_link_switch_same, current)
                } else {
                    stringResource(R.string.pair_link_switch_body, current, target)
                },
            )
        },
        confirmButton = {
            TextButton(onClick = onContinue) { Text(stringResource(R.string.pair_link_switch_confirm)) }
        },
        dismissButton = {
            TextButton(onClick = onCancel) { Text(stringResource(R.string.action_cancel)) }
        },
    )
}

/** The app was opened with a link that isn't a valid pairing link: it was ignored. */
@Composable
fun RefusedLinkDialog(onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.pair_link_refused_title)) },
        text = { Text(stringResource(R.string.pair_link_refused_body)) },
        confirmButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_close)) }
        },
    )
}
