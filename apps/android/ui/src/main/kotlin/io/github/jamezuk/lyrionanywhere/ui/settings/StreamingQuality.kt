package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.NetworkCheck
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
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
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.data.StreamingSettings
import io.github.jamezuk.lyrionanywhere.ui.R

/** The levels offered, best first. */
private val LEVELS = listOf(StreamQuality.ORIGINAL, StreamQuality.HIGH, StreamQuality.LOW)

@Composable
internal fun qualityName(q: StreamQuality): String = stringResource(
    when (q) {
        StreamQuality.ORIGINAL -> R.string.stream_quality_original
        StreamQuality.HIGH -> R.string.stream_quality_high
        StreamQuality.LOW -> R.string.stream_quality_low
    },
)

@Composable
private fun qualityDetail(q: StreamQuality): String = stringResource(
    when (q) {
        StreamQuality.ORIGINAL -> R.string.stream_quality_original_detail
        StreamQuality.HIGH -> R.string.stream_quality_high_detail
        StreamQuality.LOW -> R.string.stream_quality_low_detail
    },
)

/**
 * Settings › Streaming quality (docs/plans/streaming-quality.md): the level of this phone's
 * stream on Wi-Fi and on mobile data. Lower levels need a server that offers them.
 */
@Composable
fun StreamingQualitySetting(streaming: StreamingSettings, onChange: (StreamingSettings) -> Unit) {
    var open by rememberSaveable { mutableStateOf(false) }
    val summary = stringResource(
        R.string.stream_quality_summary,
        qualityName(streaming.wifi),
        qualityName(streaming.mobile),
    )
    ListItem(
        leadingContent = { Icon(Icons.Outlined.NetworkCheck, contentDescription = null) },
        headlineContent = { Text(stringResource(R.string.settings_stream_quality)) },
        supportingContent = { Text(summary) },
        modifier = Modifier.testTag("stream-quality-setting").clickable { open = true },
    )
    if (open) {
        AlertDialog(
            onDismissRequest = { open = false },
            title = { Text(stringResource(R.string.settings_stream_quality)) },
            text = {
                Column(Modifier.verticalScroll(rememberScrollState())) {
                    Text(stringResource(R.string.stream_quality_explain), style = MaterialTheme.typography.bodySmall)
                    Choices(
                        title = stringResource(R.string.stream_quality_on_wifi),
                        selected = streaming.wifi,
                        tag = "wifi",
                        onSelect = { onChange(streaming.copy(wifi = it)) },
                    )
                    Choices(
                        title = stringResource(R.string.stream_quality_on_mobile),
                        selected = streaming.mobile,
                        tag = "mobile",
                        onSelect = { onChange(streaming.copy(mobile = it)) },
                    )
                }
            },
            confirmButton = { TextButton(onClick = { open = false }) { Text(stringResource(R.string.action_done)) } },
        )
    }
}

@Composable
private fun Choices(title: String, selected: StreamQuality, tag: String, onSelect: (StreamQuality) -> Unit) {
    Text(
        title,
        style = MaterialTheme.typography.titleSmall,
        modifier = Modifier.padding(top = 16.dp, bottom = 4.dp).semantics { heading() },
    )
    Column(Modifier.selectableGroup()) {
        LEVELS.forEach { q ->
            Row(
                Modifier
                    .fillMaxWidth()
                    .selectable(selected = q == selected, role = Role.RadioButton, onClick = { onSelect(q) })
                    .padding(vertical = 6.dp)
                    .testTag("stream-quality-$tag-${q.name.lowercase()}"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                RadioButton(selected = q == selected, onClick = null)
                Column(Modifier.padding(start = 8.dp)) {
                    Text(qualityName(q))
                    Text(
                        qualityDetail(q),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}
