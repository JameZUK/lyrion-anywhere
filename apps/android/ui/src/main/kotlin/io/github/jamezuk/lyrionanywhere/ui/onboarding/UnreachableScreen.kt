package io.github.jamezuk.lyrionanywhere.ui.onboarding

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.settings.SignOutDialog

/**
 * The server can't be reached (and there are no downloads to show offline): try again, or
 * sign out. Signing out asks first, as it does in Settings, and goes the same way (downloads
 * kept unless the user chooses to remove them).
 */
@Composable
fun UnreachableScreen(
    serverName: String,
    detail: String,
    downloadedTracks: Int,
    downloadedBytes: Long,
    onRetry: () -> Unit,
    onSignOut: (removeDownloads: Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    var confirm by rememberSaveable { mutableStateOf(false) }
    Box(modifier.fillMaxSize().safeDrawingPadding().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                stringResource(R.string.unreachable_title),
                style = MaterialTheme.typography.headlineSmall,
                modifier = Modifier.semantics { heading() },
            )
            Text(stringResource(R.string.unreachable_body, serverName), textAlign = TextAlign.Center)
            if (detail.isNotBlank()) {
                Text(
                    detail.take(200),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
            }
            Button(onClick = onRetry) { Text(stringResource(R.string.action_retry)) }
            TextButton(onClick = { confirm = true }, modifier = Modifier.testTag("unreachable-sign-out")) {
                Text(stringResource(R.string.action_sign_out))
            }
        }
    }
    if (confirm) {
        SignOutDialog(
            downloadedTracks = downloadedTracks,
            downloadedBytes = downloadedBytes,
            onSignOut = { remove ->
                confirm = false
                onSignOut(remove)
            },
            onDismiss = { confirm = false },
        )
    }
}
