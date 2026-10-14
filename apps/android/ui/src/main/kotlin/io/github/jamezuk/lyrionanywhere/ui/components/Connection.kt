package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.HourglassEmpty
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.role
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R

/** Test tags for the connection's banner and pill. */
object ConnectionTags {
    const val BANNER = "connection-banner"
    const val PILL = "connection-reconnecting"
    const val BUSY = "connection-server-busy"
}

/**
 * "Can’t reach your server · Downloads": a strip on the current screen while the server
 * can't be reached, which leads to the downloads. The screen stays as it was (offline mode
 * takes over only when the server stays away; SPEC §8.4). One tap target, at least 48 dp
 * high, read once by TalkBack (and announced when it appears).
 */
@Composable
fun ServerUnreachableBanner(onDownloads: () -> Unit, modifier: Modifier = Modifier) {
    val description = stringResource(R.string.connection_unreachable_description)
    val clickLabel = stringResource(R.string.connection_open_downloads)
    Surface(
        color = MaterialTheme.colorScheme.tertiaryContainer,
        contentColor = MaterialTheme.colorScheme.onTertiaryContainer,
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = 48.dp)
            .testTag(ConnectionTags.BANNER)
            .clickable(onClickLabel = clickLabel, role = Role.Button, onClick = onDownloads)
            .clearAndSetSemantics {
                contentDescription = description
                role = Role.Button
                liveRegion = LiveRegionMode.Polite
            },
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Icon(Icons.Outlined.CloudOff, contentDescription = null)
            Text(
                stringResource(R.string.connection_unreachable_downloads),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.weight(1f),
            )
            Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null)
        }
    }
}

/** "Reconnecting…": a small pill over the current screen while a reconnect is under way. */
@Composable
fun ReconnectingPill(modifier: Modifier = Modifier) {
    val text = stringResource(R.string.connection_reconnecting)
    Surface(
        shape = CircleShape,
        color = MaterialTheme.colorScheme.inverseSurface,
        contentColor = MaterialTheme.colorScheme.inverseOnSurface,
        shadowElevation = 2.dp,
        modifier = modifier
            .testTag(ConnectionTags.PILL)
            .clearAndSetSemantics {
                contentDescription = text
                liveRegion = LiveRegionMode.Polite
            },
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
        ) {
            CircularProgressIndicator(
                strokeWidth = 2.dp,
                color = MaterialTheme.colorScheme.inverseOnSurface,
                modifier = Modifier.size(14.dp),
            )
            Text(text, style = MaterialTheme.typography.labelLarge)
        }
    }
}

/**
 * "Your music server is busy. Waiting for it…": a calm pill over the current screen while
 * the music server has stopped answering for a while (the gateway is reachable). Not an
 * error: what is playing keeps playing, and it goes by itself when the server answers.
 */
@Composable
fun ServerBusyPill(modifier: Modifier = Modifier) {
    val text = stringResource(R.string.connection_server_busy)
    Surface(
        shape = CircleShape,
        color = MaterialTheme.colorScheme.secondaryContainer,
        contentColor = MaterialTheme.colorScheme.onSecondaryContainer,
        shadowElevation = 2.dp,
        modifier = modifier
            .padding(horizontal = 16.dp)
            .testTag(ConnectionTags.BUSY)
            .clearAndSetSemantics {
                contentDescription = text
                liveRegion = LiveRegionMode.Polite
            },
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
        ) {
            Icon(Icons.Outlined.HourglassEmpty, contentDescription = null, modifier = Modifier.size(16.dp))
            Text(text, style = MaterialTheme.typography.labelLarge)
        }
    }
}
