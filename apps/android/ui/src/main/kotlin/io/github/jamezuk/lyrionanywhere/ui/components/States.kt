package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.HourglassEmpty
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import android.content.res.Resources
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.ui.R

/** The network layer's reason on its own line, shortened, or nothing. */
fun detailLine(detail: String): String {
    val d = detail.trim().take(200)
    return if (d.isEmpty()) "" else "\n($d)"
}

/** A message for an error, in plain British English (usable outside composition). */
fun errorText(res: Resources, error: AppError): String = when (error) {
    is AppError.Offline -> res.getString(R.string.error_offline) + detailLine(error.detail)
    AppError.SignedOut -> res.getString(R.string.error_signed_out)
    AppError.TwoStepRequired -> res.getString(R.string.error_two_step_required)
    is AppError.Forbidden -> res.getString(R.string.error_forbidden)
    is AppError.RateLimited -> {
        val minutes = ((error.retryAfterSeconds + 59) / 60).toInt()
        if (error.retryAfterSeconds < 60) {
            res.getQuantityString(R.plurals.error_rate_limited_seconds, error.retryAfterSeconds.toInt(), error.retryAfterSeconds.toInt())
        } else {
            res.getQuantityString(R.plurals.error_rate_limited_minutes, minutes, minutes)
        }
    }
    is AppError.Server -> res.getString(R.string.error_server)
    is AppError.InvalidInput -> error.detail.replaceFirstChar { it.uppercase() }
    is AppError.Unexpected -> res.getString(R.string.error_unexpected)
}

@Composable
fun errorMessage(error: AppError): String = errorText(LocalResources.current, error)

private fun icon(error: AppError): ImageVector = when (error) {
    is AppError.Offline -> Icons.Outlined.CloudOff
    is AppError.Forbidden, AppError.SignedOut -> Icons.Outlined.Lock
    is AppError.RateLimited -> Icons.Outlined.HourglassEmpty
    else -> Icons.Outlined.ErrorOutline
}

/** A full-area error with an optional retry. */
@Composable
fun ErrorState(error: AppError, onRetry: (() -> Unit)?, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(icon(error), contentDescription = null, modifier = Modifier.size(48.dp))
            // The live region is the message itself, so it is read once, without the button.
            Text(
                errorMessage(error),
                style = MaterialTheme.typography.bodyLarge,
                textAlign = TextAlign.Center,
                modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
            )
            if (onRetry != null) {
                FilledTonalButton(onClick = onRetry) { Text(stringResource(R.string.action_retry)) }
            }
        }
    }
}

@Composable
fun LoadingState(modifier: Modifier = Modifier) {
    val label = stringResource(R.string.loading)
    Box(modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
        CircularProgressIndicator(Modifier.semantics { this.contentDescription = label })
    }
}

@Composable
fun EmptyState(text: String, modifier: Modifier = Modifier) {
    Box(modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
        Text(text, style = MaterialTheme.typography.bodyLarge, textAlign = TextAlign.Center)
    }
}

/** "3:07", or "1:02:03" for long tracks. */
fun formatDuration(seconds: Double?): String {
    if (seconds == null || seconds.isNaN() || seconds < 0) return "–:––"
    val total = seconds.toLong()
    val h = total / 3600
    val m = (total % 3600) / 60
    val s = total % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%d:%02d".format(m, s)
}
