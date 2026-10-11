package io.github.jamezuk.lyrionanywhere.ui.offline

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.offline.DownloadSettings
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader

@Composable
fun formatName(f: DownloadFormat): String = stringResource(
    when (f) {
        DownloadFormat.ORIGINAL -> R.string.format_original
        DownloadFormat.MP3 -> R.string.format_mp3
        DownloadFormat.OPUS -> R.string.format_opus
    },
)

@Composable
private fun capName(cap: Long): String =
    if (cap <= 0) stringResource(R.string.storage_no_limit) else formatBytes(cap)

@Composable
private fun recentName(n: Int): String =
    if (n <= 0) stringResource(R.string.mode_off) else pluralStringResource(R.plurals.smart_recent_n, n, n)

@Composable
private fun <T> Choices(options: List<T>, selected: T, label: @Composable (T) -> String, onSelect: (T) -> Unit) {
    Column {
        options.forEach { o ->
            ListItem(
                leadingContent = { RadioButton(selected = o == selected, onClick = null) },
                headlineContent = { Text(label(o)) },
                modifier = Modifier.selectable(selected = o == selected, role = Role.RadioButton) { onSelect(o) },
            )
        }
    }
}

@Composable
private fun SwitchRow(title: String, detail: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(detail) },
        trailingContent = { Switch(checked = checked, onCheckedChange = null) },
        modifier = Modifier.toggleable(value = checked, role = Role.Switch, onValueChange = onChange),
    )
}

/** Settings → Downloads and storage: the cap, the formats, Wi-Fi only, smart downloads. */
@Composable
fun StorageSettingsScreen(
    state: OfflineState,
    refreshing: Boolean,
    error: AppError?,
    onChange: ((DownloadSettings) -> DownloadSettings) -> Unit,
    onRefreshSmart: () -> Unit,
    onDownloads: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val d = state.settings
    LazyColumn(modifier.fillMaxSize().testTag("storage-settings"), contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            ListItem(
                leadingContent = { Icon(Icons.Outlined.Download, contentDescription = null) },
                headlineContent = { Text(stringResource(R.string.title_downloads)) },
                supportingContent = {
                    Text(
                        if (d.capBytes > 0) {
                            stringResource(R.string.storage_used_of, formatBytes(state.usedBytes), formatBytes(d.capBytes))
                        } else {
                            stringResource(R.string.storage_used, formatBytes(state.usedBytes))
                        },
                    )
                },
                modifier = Modifier.clickable(onClick = onDownloads),
            )
            HorizontalDivider()
        }
        item { SectionHeader(stringResource(R.string.storage_cap)) }
        item {
            Text(
                stringResource(R.string.storage_cap_body),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
            Choices(DownloadSettings.CAP_CHOICES, d.capBytes, { capName(it) }) { c -> onChange { it.copy(capBytes = c) } }
        }
        item { SectionHeader(stringResource(R.string.format_wifi)) }
        item {
            Choices(DownloadFormat.entries, d.wifiFormat, { formatName(it) }) { f -> onChange { it.copy(wifiFormat = f) } }
        }
        item { SectionHeader(stringResource(R.string.format_mobile)) }
        item {
            Choices(DownloadFormat.entries, d.mobileFormat, { formatName(it) }) { f -> onChange { it.copy(mobileFormat = f) } }
            Text(
                stringResource(R.string.format_note),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        item {
            SwitchRow(
                stringResource(R.string.wifi_only),
                stringResource(R.string.wifi_only_body),
                d.wifiOnly,
            ) { on -> onChange { it.copy(wifiOnly = on) } }
            HorizontalDivider()
        }
        item { SectionHeader(stringResource(R.string.smart_downloads)) }
        item {
            Text(
                stringResource(R.string.smart_downloads_body),
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        item { SectionHeader(stringResource(R.string.smart_recent)) }
        item {
            Choices(DownloadSettings.RECENT_CHOICES, d.smartRecentAlbums, { recentName(it) }) { n ->
                onChange { it.copy(smartRecentAlbums = n) }
            }
        }
        item {
            SwitchRow(
                stringResource(R.string.smart_favourites),
                stringResource(R.string.smart_favourites_body),
                d.smartFavourites,
            ) { on -> onChange { it.copy(smartFavourites = on) } }
        }
        if (d.smartOn) {
            item {
                OutlinedButton(onClick = onRefreshSmart, enabled = !refreshing, modifier = Modifier.padding(horizontal = 16.dp)) {
                    Text(stringResource(if (refreshing) R.string.smart_refreshing else R.string.smart_refresh))
                }
                error?.let {
                    Text(
                        errorMessage(it),
                        color = MaterialTheme.colorScheme.error,
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(16.dp),
                    )
                }
            }
        }
    }
}
