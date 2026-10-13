package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.FastForward
import androidx.compose.material.icons.outlined.SignalCellularAlt
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.ReadAheadSettings
import io.github.jamezuk.lyrionanywhere.ui.R

/**
 * Settings › Playback › Gaps in the signal (rolling cache, Phase A): this phone's player
 * fetches the next track long before the current one ends, so tunnels and lifts don't stop
 * the music. Both switches are on by default; the second matters only while the first is on.
 */
@Composable
fun ReadAheadSetting(settings: ReadAheadSettings, onChange: (ReadAheadSettings) -> Unit) {
    Column(Modifier.testTag("read-ahead-settings")) {
        Text(
            stringResource(R.string.gaps_title),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp).semantics { heading() },
        )
        Text(
            stringResource(R.string.gaps_explain),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
        ListItem(
            leadingContent = { Icon(Icons.Outlined.FastForward, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.read_ahead)) },
            supportingContent = { Text(stringResource(R.string.read_ahead_detail)) },
            trailingContent = { Switch(checked = settings.enabled, onCheckedChange = null) },
            modifier = Modifier
                .toggleable(value = settings.enabled, role = Role.Switch) { onChange(settings.copy(enabled = it)) }
                .testTag("read-ahead-setting"),
        )
        ListItem(
            leadingContent = { Icon(Icons.Outlined.SignalCellularAlt, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.read_ahead_mobile)) },
            supportingContent = {
                Text(
                    stringResource(
                        if (settings.enabled) R.string.read_ahead_mobile_detail else R.string.read_ahead_mobile_off,
                    ),
                )
            },
            trailingContent = {
                Switch(checked = settings.enabled && settings.onMobileData, onCheckedChange = null, enabled = settings.enabled)
            },
            modifier = Modifier
                .toggleable(
                    value = settings.enabled && settings.onMobileData,
                    enabled = settings.enabled,
                    role = Role.Switch,
                ) { onChange(settings.copy(onMobileData = it)) }
                .testTag("read-ahead-mobile-setting"),
        )
    }
}
