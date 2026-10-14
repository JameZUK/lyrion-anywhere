package io.github.jamezuk.lyrionanywhere.ui.offline

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import io.github.jamezuk.lyrionanywhere.ui.components.Choice
import io.github.jamezuk.lyrionanywhere.ui.components.ChoiceRow
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
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
private fun SwitchRow(title: String, detail: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(detail) },
        trailingContent = { Switch(checked = checked, onCheckedChange = null) },
        modifier = Modifier.toggleable(value = checked, role = Role.Switch, onValueChange = onChange),
    )
}

/**
 * Settings › Downloads › Download options: the storage limit, the formats, Wi-Fi only and
 * smart downloads. Each list of options is one row with a picker (Wave 3.5). The downloads
 * themselves (progress, what's on the phone) are the Downloads page, one level up.
 */
@Composable
fun StorageSettingsScreen(
    state: OfflineState,
    refreshing: Boolean,
    error: AppError?,
    onChange: ((DownloadSettings) -> DownloadSettings) -> Unit,
    onRefreshSmart: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val d = state.settings
    LazyColumn(modifier.fillMaxSize().testTag("storage-settings"), contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            ChoiceRow(
                title = stringResource(R.string.storage_cap),
                choices = DownloadSettings.CAP_CHOICES.mapIndexed { i, c -> Choice(c, capName(c), tag = "storage-cap-$i") },
                selected = d.capBytes,
                onSelect = { c -> onChange { it.copy(capBytes = c) } },
                tag = "storage-cap",
                note = stringResource(R.string.storage_cap_body),
            )
        }
        item {
            ChoiceRow(
                title = stringResource(R.string.format_wifi),
                choices = DownloadFormat.entries.map { Choice(it, formatName(it), tag = "format-wifi-${it.name.lowercase()}") },
                selected = d.wifiFormat,
                onSelect = { f -> onChange { it.copy(wifiFormat = f) } },
                tag = "format-wifi",
                note = stringResource(R.string.format_note),
            )
        }
        item {
            ChoiceRow(
                title = stringResource(R.string.format_mobile),
                choices = DownloadFormat.entries.map { Choice(it, formatName(it), tag = "format-mobile-${it.name.lowercase()}") },
                selected = d.mobileFormat,
                onSelect = { f -> onChange { it.copy(mobileFormat = f) } },
                tag = "format-mobile",
                note = stringResource(R.string.format_note),
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
            ChoiceRow(
                title = stringResource(R.string.smart_recent),
                choices = DownloadSettings.RECENT_CHOICES.mapIndexed { i, n -> Choice(n, recentName(n), tag = "smart-recent-$i") },
                selected = d.smartRecentAlbums,
                onSelect = { n -> onChange { it.copy(smartRecentAlbums = n) } },
                tag = "smart-recent",
                note = stringResource(R.string.smart_downloads_body),
            )
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
                        modifier = Modifier.padding(16.dp).semantics { liveRegion = LiveRegionMode.Polite },
                    )
                }
            }
        }
    }
}
