package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.GraphicEq
import androidx.compose.material.icons.outlined.SwapHoriz
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.Surface
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.NotBitPerfect
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.SignalStage
import io.github.jamezuk.lyrionanywhere.core.SignalVerdict
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.ui.R

/** "44.1 kHz", "96 kHz", "176.4 kHz". */
fun formatRate(hz: UInt): String {
    val k = hz.toLong()
    return if (k % 1000 == 0L) "${k / 1000} kHz" else "${"%.1f".format(java.util.Locale.ROOT, k / 1000.0)} kHz"
}

@Composable
private fun verdictText(v: SignalVerdict): String = stringResource(
    when (v) {
        SignalVerdict.BIT_PERFECT -> R.string.verdict_bit_perfect
        SignalVerdict.LOSSLESS_PROCESSED -> R.string.verdict_lossless_processed
        SignalVerdict.RESAMPLED -> R.string.verdict_resampled
        SignalVerdict.LOSSY_SOURCE -> R.string.verdict_lossy
    },
)

/**
 * The signal path of what this phone is playing: source, every stage, output, and an
 * honest verdict. "Bit-perfect" is only claimed end to end when Android passes the audio
 * through untouched; otherwise the panel says the engine is bit-perfect up to the mixer.
 */
@Composable
fun SignalPathPanel(
    path: SignalPathInfo?,
    modifier: Modifier = Modifier,
    /** The stream level asked for, and whether on mobile data (Streaming quality). */
    quality: StreamQuality = StreamQuality.ORIGINAL,
    metered: Boolean = false,
) {
    Card(modifier.fillMaxWidth()) {
        Column(Modifier.padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.GraphicEq, contentDescription = null)
                Text(
                    stringResource(R.string.signal_path),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.padding(start = 8.dp).weight(1f).semantics { heading() },
                )
                if (path != null) {
                    Surface(
                        shape = MaterialTheme.shapes.small,
                        color = if (path.verdict == SignalVerdict.BIT_PERFECT) {
                            MaterialTheme.colorScheme.primaryContainer
                        } else {
                            MaterialTheme.colorScheme.secondaryContainer
                        },
                    ) {
                        Text(
                            verdictText(path.verdict),
                            style = MaterialTheme.typography.labelLarge,
                            modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
                        )
                    }
                }
            }
            if (path == null) {
                Text(stringResource(R.string.signal_path_none), style = MaterialTheme.typography.bodyMedium)
                return@Column
            }
            val source = buildList {
                add(path.sourceCodec)
                add(formatRate(path.sourceRate))
                path.sourceBits?.let { add(stringResource(R.string.bits_format, it.toInt())) }
            }.joinToString(" · ")
            LabelledLine(stringResource(R.string.signal_source), source)
            if (quality != StreamQuality.ORIGINAL) {
                // Lossless tracks arrive re-encoded; others as they are.
                LabelledLine(
                    stringResource(R.string.signal_streaming),
                    stringResource(
                        if (metered) R.string.signal_streaming_mobile else R.string.signal_streaming_wifi,
                        io.github.jamezuk.lyrionanywhere.ui.settings.qualityName(quality),
                    ),
                )
            }
            path.stages.forEach { StageRow(it) }
            LabelledLine(
                stringResource(R.string.signal_output),
                listOf(path.outputDevice, formatRate(path.outputRate), path.outputFormat).joinToString(" · "),
            )
            val note = when {
                path.bitPerfect -> R.string.signal_note_bit_perfect
                path.engineBitPerfect -> R.string.signal_note_mixer
                path.resampled -> R.string.signal_note_resampled
                path.dspActive -> R.string.signal_note_dsp
                else -> null
            }
            note?.let {
                Text(
                    stringResource(it),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            if (path.fixedVolume) {
                Text(stringResource(R.string.signal_volume_on_dac), style = MaterialTheme.typography.bodySmall)
            }
            if (!path.bitPerfect && path.notBitPerfect.isNotEmpty()) {
                val reasons = path.notBitPerfect.map { stringResource(reasonText(it)) }
                Column(Modifier.semantics(mergeDescendants = true) {}) {
                    Text(
                        stringResource(R.string.signal_why_not),
                        style = MaterialTheme.typography.labelMedium,
                        color = MaterialTheme.colorScheme.primary,
                    )
                    reasons.forEach {
                        Text("• $it", style = MaterialTheme.typography.bodySmall)
                    }
                }
            }
        }
    }
}

/** Why the path isn't bit-perfect end to end, in words. */
fun reasonText(r: NotBitPerfect): Int = when (r) {
    NotBitPerfect.TURNED_OFF -> R.string.reason_turned_off
    NotBitPerfect.NO_USB_DAC -> R.string.reason_no_usb_dac
    NotBitPerfect.ANDROID_TOO_OLD -> R.string.reason_android_too_old
    NotBitPerfect.NOT_OFFERED -> R.string.reason_not_offered
    NotBitPerfect.RATE_NOT_OFFERED -> R.string.reason_rate_not_offered
    NotBitPerfect.DAC_TOO_SHALLOW -> R.string.reason_dac_too_shallow
    NotBitPerfect.SOURCE_TOO_DEEP -> R.string.reason_source_too_deep
    NotBitPerfect.NOT_APPLIED -> R.string.reason_not_applied
    NotBitPerfect.FORMAT_REFUSED -> R.string.reason_format_refused
    NotBitPerfect.DSP_ACTIVE -> R.string.reason_dsp_active
    NotBitPerfect.RESAMPLED -> R.string.reason_resampled
    NotBitPerfect.LOSSY_SOURCE -> R.string.reason_lossy_source
    NotBitPerfect.OTHER_PROCESSING -> R.string.reason_other_processing
    NotBitPerfect.CROSSFADE -> R.string.reason_crossfade
}

@Composable
private fun LabelledLine(label: String, value: String) {
    // One stop for TalkBack: the label and its value together.
    Column(Modifier.semantics(mergeDescendants = true) {}) {
        Text(label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary)
        Text(value, style = MaterialTheme.typography.bodyMedium)
    }
}

@Composable
private fun StageRow(stage: SignalStage) {
    val (icon, kind) = when {
        stage.resamples -> Icons.Outlined.SwapHoriz to R.string.stage_resamples
        stage.bitTransparent -> Icons.Outlined.CheckCircle to R.string.stage_transparent
        else -> Icons.Outlined.Tune to R.string.stage_processing
    }
    val kindText = stringResource(kind)
    Row(
        verticalAlignment = Alignment.CenterVertically,
        modifier = Modifier.semantics(mergeDescendants = true) { contentDescription = "${stage.name}, $kindText" },
    ) {
        Icon(icon, contentDescription = null, modifier = Modifier.size(18.dp))
        Text(stage.name, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(start = 8.dp))
    }
}

/** This phone's player's state, in a line. */
@Composable
internal fun playOnPhoneDetail(local: LocalPlayerState): String {
    val status: LocalPlayerStatus? = local.status
    return when {
        !local.enabled -> stringResource(R.string.play_on_phone_off)
        local.error != null -> stringResource(R.string.play_on_phone_failed)
        status == null -> stringResource(R.string.play_on_phone_starting)
        else -> when (val c = status.connection) {
            PlayerConnection.Connecting -> stringResource(R.string.play_on_phone_starting)
            PlayerConnection.Connected -> stringResource(R.string.play_on_phone_ready, status.name)
            is PlayerConnection.Reconnecting -> stringResource(R.string.play_on_phone_reconnecting)
            is PlayerConnection.Stopped -> stringResource(R.string.play_on_phone_stopped, c.reason)
        }
    }
}
