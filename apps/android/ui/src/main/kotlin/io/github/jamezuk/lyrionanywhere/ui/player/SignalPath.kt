package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
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
import androidx.compose.ui.platform.testTag
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
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

/** A cushion of listening held ahead, in whole hours, minutes and seconds. */
data class CushionParts(val hours: Long, val minutes: Long, val seconds: Long)

/**
 * How a cushion is shown: seconds under a minute, whole minutes under an hour, then hours
 * and minutes; null below a second (nothing to show).
 */
fun cushionParts(ms: Long): CushionParts? = when {
    ms < 1_000 -> null
    ms < 60_000 -> CushionParts(0, 0, ms / 1_000)
    ms < 3_600_000 -> CushionParts(0, ms / 60_000, 0)
    else -> CushionParts(ms / 3_600_000, (ms % 3_600_000) / 60_000, 0)
}

/** "45 s", "4 min", "1 h 5 min", "2 h"; null below a second. */
fun formatCushion(ms: Long): String? = cushionParts(ms)?.let { p ->
    when {
        p.hours > 0 && p.minutes > 0 -> "${p.hours} h ${p.minutes} min"
        p.hours > 0 -> "${p.hours} h"
        p.minutes > 0 -> "${p.minutes} min"
        else -> "${p.seconds} s"
    }
}

/** The cushion in words, for TalkBack: "1 hour 5 minutes". */
@Composable
private fun cushionSpoken(p: CushionParts): String = buildList {
    if (p.hours > 0) add(pluralStringResource(R.plurals.cushion_hours, p.hours.toInt(), p.hours.toInt()))
    if (p.minutes > 0) add(pluralStringResource(R.plurals.cushion_minutes, p.minutes.toInt(), p.minutes.toInt()))
    if (p.hours == 0L && p.minutes == 0L) add(pluralStringResource(R.plurals.cushion_seconds, p.seconds.toInt(), p.seconds.toInt()))
}.joinToString(" ")

/** "4 min buffered ahead" (read out in full words); nothing below a second. */
@Composable
fun CushionLine(ms: Long?, modifier: Modifier = Modifier) {
    val p = ms?.let(::cushionParts) ?: return
    val shown = stringResource(R.string.cushion_ahead, formatCushion(ms).orEmpty())
    val spoken = stringResource(R.string.cushion_ahead, cushionSpoken(p))
    Text(
        shown,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.semantics { contentDescription = spoken },
    )
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
    /** The EQ profile in use on this output (Wave 2 §5.3; null: none). */
    eq: String? = null,
    /** Listening held ahead while reading ahead (ms; null: none to show). */
    cushionMs: Long? = null,
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
            CushionLine(cushionMs, Modifier.testTag("signal-cushion"))
            if (path.fromCache) {
                // The replay cache: its first bytes came from the network, the rest from the
                // phone, checked by its hash, so the audio is the same.
                Box(Modifier.testTag("signal-from-cache")) {
                    StageRow(SignalStage(stringResource(R.string.stage_from_cache), bitTransparent = true, resamples = false))
                }
            }
            if (path.fromDownloads) {
                // A downloaded original: its first bytes came from the network, the rest from
                // the download, whose audio was checked to be the same.
                Box(Modifier.testTag("signal-from-downloads")) {
                    StageRow(SignalStage(stringResource(R.string.stage_from_downloads), bitTransparent = true, resamples = false))
                }
            }
            path.stages.forEach { StageRow(it) }
            if (eq != null) EqChip(eq)
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

/** "EQ: <profile>": which profile this output uses (per-output EQ). */
@Composable
private fun EqChip(name: String) {
    Surface(
        shape = MaterialTheme.shapes.small,
        color = MaterialTheme.colorScheme.tertiaryContainer,
        modifier = Modifier.testTag("eq-chip"),
    ) {
        Text(
            stringResource(R.string.signal_eq_chip, name),
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onTertiaryContainer,
            maxLines = 1,
            overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis,
            modifier = Modifier.padding(horizontal = 10.dp, vertical = 4.dp),
        )
    }
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
