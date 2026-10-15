package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.BatteryAlert
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R

/**
 * The one-time offer at the top of Settings while this phone is a player and Android
 * restricts the app in the background: with battery optimisation on, Android may cut the
 * player's connection while the screen is off. [refused]: Android has already refused to
 * keep playback in the foreground this run (said so). Dismissed, it isn't shown again;
 * Settings › Playback keeps [BackgroundPlaybackSetting].
 */
@Composable
fun BackgroundPlaybackPrompt(refused: Boolean, onAllow: () -> Unit, onDismiss: () -> Unit) {
    Card(
        Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp)
            .testTag("background-playback-prompt"),
    ) {
        Column(Modifier.padding(start = 16.dp, end = 8.dp, top = 12.dp, bottom = 4.dp)) {
            Text(stringResource(R.string.background_playback_title), style = MaterialTheme.typography.titleSmall)
            Text(
                stringResource(if (refused) R.string.background_playback_refused else R.string.background_playback_explain),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 4.dp, end = 8.dp),
            )
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) {
                TextButton(onClick = onDismiss, modifier = Modifier.testTag("background-playback-dismiss")) {
                    Text(stringResource(R.string.background_playback_not_now))
                }
                TextButton(onClick = onAllow, modifier = Modifier.testTag("background-playback-allow")) {
                    Text(stringResource(R.string.background_playback_allow))
                }
            }
        }
    }
}

/** Settings › Playback: the same option, kept after the offer is dismissed. */
@Composable
fun BackgroundPlaybackSetting(onAllow: () -> Unit) {
    ListItem(
        leadingContent = { Icon(Icons.Outlined.BatteryAlert, contentDescription = null) },
        headlineContent = { Text(stringResource(R.string.background_playback_title)) },
        supportingContent = { Text(stringResource(R.string.background_playback_explain)) },
        trailingContent = {
            TextButton(onClick = onAllow, modifier = Modifier.testTag("background-playback-setting-allow")) {
                Text(stringResource(R.string.background_playback_allow))
            }
        },
        modifier = Modifier.testTag("background-playback-setting"),
    )
}
