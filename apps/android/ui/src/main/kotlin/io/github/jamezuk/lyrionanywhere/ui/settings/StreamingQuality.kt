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
import io.github.jamezuk.lyrionanywhere.ui.components.Choice
import io.github.jamezuk.lyrionanywhere.ui.components.ChoiceRow
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
 * Settings › Playback › Streaming quality (docs/plans/streaming-quality.md): the level of
 * this phone's stream on Wi-Fi and on mobile data, a row with a picker each (Wave 3.5).
 * Lower levels need a server that offers them.
 */
@Composable
fun StreamingQualitySetting(streaming: StreamingSettings, onChange: (StreamingSettings) -> Unit) {
    val explain = stringResource(R.string.stream_quality_explain)
    @Composable
    fun choices(tag: String) = LEVELS.map { q -> Choice(q, qualityName(q), qualityDetail(q), tag = "stream-quality-$tag-${q.name.lowercase()}") }
    Column(Modifier.testTag("stream-quality-setting")) {
        ChoiceRow(
            title = stringResource(R.string.stream_quality_wifi_row),
            choices = choices("wifi"),
            selected = streaming.wifi,
            onSelect = { onChange(streaming.copy(wifi = it)) },
            icon = Icons.Outlined.NetworkCheck,
            tag = "stream-quality-wifi",
            note = explain,
        )
        ChoiceRow(
            title = stringResource(R.string.stream_quality_mobile_row),
            choices = choices("mobile"),
            selected = streaming.mobile,
            onSelect = { onChange(streaming.copy(mobile = it)) },
            icon = Icons.Outlined.NetworkCheck,
            tag = "stream-quality-mobile",
            note = explain,
        )
    }
}
