package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsPadding
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
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.unit.dp
import androidx.compose.ui.zIndex
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.ui.R

/** Test tags for the connection line, by what it says. */
object ConnectionTags {
    const val BANNER = "connection-banner"
    const val PILL = "connection-reconnecting"
    const val BUSY = "connection-server-busy"
    /** The screen's content under the line. */
    const val CONTENT = "connection-content"
}

/** What the connection line says (at most one thing at a time). */
enum class ConnectionMessage {
    /** A reconnect under way. */
    RECONNECTING,

    /** The music server is busy; the gateway answers. */
    BUSY,

    /** The server can't be reached. */
    UNREACHABLE,

    /** … and this phone carries on playing what it holds. */
    UNREACHABLE_PLAYING_HERE,
}

/**
 * The one message for the link to the server, or none (pure, tested on the JVM). Offline
 * mode having the whole screen says it by itself. [playingHere]: this phone is playing (its
 * player, or downloads).
 */
fun connectionMessage(c: ConnectionUi, playingHere: Boolean): ConnectionMessage? = when {
    c.takeover -> null
    c.unreachable -> if (playingHere) ConnectionMessage.UNREACHABLE_PLAYING_HERE else ConnectionMessage.UNREACHABLE
    c.busy -> ConnectionMessage.BUSY
    c.reconnecting -> ConnectionMessage.RECONNECTING
    else -> null
}

/**
 * The screen under one connection line (the offline controls spec): the line sits at the
 * top and takes its own space, so it never covers the transport, the seek bar or the queue;
 * the content keeps clear of the status bar as before (the line handles it while shown).
 */
@Composable
fun ConnectionFrame(
    message: ConnectionMessage?,
    /** Opens the downloads (null: not offered, e.g. already there). */
    onDownloads: (() -> Unit)?,
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    Column(modifier) {
        // Drawn above the content, so nothing the page paints past its edge can hide it.
        if (message != null) ConnectionLine(message, onDownloads, Modifier.zIndex(1f).statusBarsPadding())
        Box(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .testTag(ConnectionTags.CONTENT)
                .then(if (message != null) Modifier.consumeWindowInsets(WindowInsets.statusBars) else Modifier),
        ) { content() }
    }
}

/**
 * One line about the link to the server: "Reconnecting…", "Your music server is busy.
 * Waiting for it…", or "Can’t reach your server." (with "Playing from this device." while the
 * phone plays on), which leads to the downloads. Read once by TalkBack, and announced when it
 * appears; as a button it is at least 48 dp high.
 */
@Composable
fun ConnectionLine(message: ConnectionMessage, onDownloads: (() -> Unit)?, modifier: Modifier = Modifier) {
    val text = stringResource(
        when (message) {
            ConnectionMessage.RECONNECTING -> R.string.connection_reconnecting
            ConnectionMessage.BUSY -> R.string.connection_server_busy
            ConnectionMessage.UNREACHABLE -> R.string.connection_unreachable_short
            ConnectionMessage.UNREACHABLE_PLAYING_HERE -> R.string.connection_unreachable_here
        },
    )
    val unreachable = message == ConnectionMessage.UNREACHABLE || message == ConnectionMessage.UNREACHABLE_PLAYING_HERE
    val open = onDownloads?.takeIf { unreachable }
    val clickLabel = stringResource(R.string.connection_open_downloads)
    val colours = MaterialTheme.colorScheme
    val (container, onContainer) = when (message) {
        ConnectionMessage.RECONNECTING -> colours.surfaceVariant to colours.onSurfaceVariant
        ConnectionMessage.BUSY -> colours.secondaryContainer to colours.onSecondaryContainer
        else -> colours.tertiaryContainer to colours.onTertiaryContainer
    }
    val tag = when (message) {
        ConnectionMessage.RECONNECTING -> ConnectionTags.PILL
        ConnectionMessage.BUSY -> ConnectionTags.BUSY
        else -> ConnectionTags.BANNER
    }
    Surface(
        color = container,
        contentColor = onContainer,
        modifier = modifier
            .fillMaxWidth()
            .heightIn(min = if (open != null) 48.dp else 36.dp)
            .testTag(tag)
            .then(if (open != null) Modifier.clickable(onClickLabel = clickLabel, role = Role.Button, onClick = open) else Modifier)
            .clearAndSetSemantics {
                contentDescription = text
                liveRegion = LiveRegionMode.Polite
                if (open != null) {
                    role = Role.Button
                    onClick(label = clickLabel) {
                        open()
                        true
                    }
                }
            },
    ) {
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        ) {
            when (message) {
                ConnectionMessage.RECONNECTING -> CircularProgressIndicator(
                    strokeWidth = 2.dp,
                    color = onContainer,
                    modifier = Modifier.size(16.dp),
                )
                ConnectionMessage.BUSY -> Icon(Icons.Outlined.HourglassEmpty, contentDescription = null, modifier = Modifier.size(18.dp))
                else -> Icon(Icons.Outlined.CloudOff, contentDescription = null, modifier = Modifier.size(18.dp))
            }
            Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
            if (open != null) Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null)
        }
    }
}

