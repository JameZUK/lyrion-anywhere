package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import io.github.jamezuk.lyrionanywhere.ui.components.Choice
import io.github.jamezuk.lyrionanywhere.ui.components.ChoiceRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.ReplayCache
import io.github.jamezuk.lyrionanywhere.data.ReplayCacheUsage
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.offline.formatBytes

/** Stands for "no size chosen" while the account may not keep anything. */
private const val NOT_ALLOWED = -1L

/** "500 MB", "1 GB", "5 GB"; null for off (shown as "Off"). */
fun replayCacheSizeName(bytes: Long): String? = when {
    bytes <= 0 -> null
    bytes % ReplayCache.GB == 0L -> "${bytes / ReplayCache.GB} GB"
    else -> "${bytes / ReplayCache.MB} MB"
}

/**
 * Settings › Playback › Gaps in the signal › Replay cache (rolling cache, Phase B): tracks
 * this phone streamed in full are kept, so a replay or a skip back costs almost no data.
 * The size (off, 500 MB, 1 GB, 2 GB or 5 GB; 1 GB by default), what it holds, and "Clear".
 * Without the download permission nothing is kept (the owner's decision 1): the choices are
 * shown disabled, with why. [downloadsAllowed] is null while not yet known.
 */
@Composable
fun ReplayCacheSetting(
    bytes: Long,
    usage: ReplayCacheUsage?,
    downloadsAllowed: Boolean?,
    onSize: (Long) -> Unit,
    onClear: () -> Unit,
) {
    val allowed = downloadsAllowed != false
    Column(Modifier.testTag("replay-cache-settings")) {
        Text(
            stringResource(R.string.replay_cache_title),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.primary,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp).semantics { heading() },
        )
        Text(
            stringResource(if (allowed) R.string.replay_cache_explain else R.string.replay_cache_not_allowed),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp).testTag("replay-cache-explain"),
        )
        // The size: one row with a picker (Wave 3.5); without the permission every size is
        // shown disabled and none chosen.
        val off = stringResource(R.string.mode_off)
        ChoiceRow(
            title = stringResource(R.string.replay_cache_size),
            choices = ReplayCache.CHOICES.map { c -> Choice(c, replayCacheSizeName(c) ?: off, tag = "replay-cache-size-$c", enabled = allowed) },
            selected = if (allowed) bytes else NOT_ALLOWED,
            onSelect = onSize,
            tag = "replay-cache-size",
            current = if (allowed) null else off,
        )
        if (allowed) {
            val used = usage?.takeIf { it.tracks > 0 }
            Text(
                if (used != null) {
                    pluralStringResource(R.plurals.replay_cache_used, used.tracks.toInt(), formatBytes(used.bytes), used.tracks.toInt())
                } else {
                    stringResource(R.string.replay_cache_empty)
                },
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier
                    .padding(horizontal = 16.dp, vertical = 4.dp)
                    .semantics { liveRegion = LiveRegionMode.Polite }
                    .testTag("replay-cache-usage"),
            )
            OutlinedButton(
                onClick = onClear,
                enabled = used != null,
                modifier = Modifier.padding(horizontal = 16.dp).testTag("replay-cache-clear"),
            ) {
                Text(stringResource(R.string.replay_cache_clear))
            }
        }
    }
}
