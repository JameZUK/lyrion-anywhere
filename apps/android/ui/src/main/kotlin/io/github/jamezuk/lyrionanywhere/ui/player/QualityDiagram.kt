package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowForward
import androidx.compose.material.icons.outlined.ArrowDownward
import androidx.compose.material.icons.outlined.ExpandLess
import androidx.compose.material.icons.outlined.ExpandMore
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.isTraversalGroup
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.DiagramBadge
import io.github.jamezuk.lyrionanywhere.core.DiagramBlock
import io.github.jamezuk.lyrionanywhere.core.DiagramBlockKind
import io.github.jamezuk.lyrionanywhere.core.DiagramSwitch
import io.github.jamezuk.lyrionanywhere.core.DiagramVerdict
import io.github.jamezuk.lyrionanywhere.core.QualityDiagram
import io.github.jamezuk.lyrionanywhere.core.QualityFile
import io.github.jamezuk.lyrionanywhere.core.QualityOrigin
import io.github.jamezuk.lyrionanywhere.core.SignalPathInfo
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.data.QualityDescriber
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.LocalWindowLayout
import io.github.jamezuk.lyrionanywhere.ui.components.WidthClass
import io.github.jamezuk.lyrionanywhere.ui.settings.qualityName
import io.github.jamezuk.lyrionanywhere.ui.theme.amberTint

/**
 * What the selected player's diagram is built from, so it is built again only when one of
 * these changes (docs/plans/signal-path-redesign.md). Downloads build theirs elsewhere.
 */
sealed interface DiagramKey {
    /** This phone: its signal path (the gateway's stream message in it), the file, why the level is in force. */
    data class Local(val path: SignalPathInfo?, val file: QualityFile?, val origin: QualityOrigin, val seekable: Boolean) : DiagramKey

    /** Another player: the file only. */
    data class Remote(val name: String, val file: QualityFile?) : DiagramKey
}

/** The key for [s]'s diagram (null: nothing playing, or the downloads). */
fun diagramKey(s: PlayerUiState): DiagramKey? {
    if (s.downloads) return null
    val status = s.status ?: return null
    val track = status.current ?: return null
    return if (s.localSelected) {
        DiagramKey.Local(s.local.signalPath, track.fileFormat, s.local.qualityOrigin, status.canSeek)
    } else {
        DiagramKey.Remote(s.selected?.name ?: status.name, track.fileFormat)
    }
}

/** The diagram for [key], by the shared rules. */
fun buildDiagram(key: DiagramKey, describer: QualityDescriber): QualityDiagram = when (key) {
    is DiagramKey.Local -> describer.local(key.path, key.file, key.origin, key.seekable)
    is DiagramKey.Remote -> describer.remote(key.name, key.file)
}

/** Semantics: the badge or block is drawn in the amber tint (Transcoded, Lossy, the lowering block). */
val Amber = SemanticsPropertyKey<Boolean>("Amber")
var SemanticsPropertyReceiver.amber by Amber

/** A badge's container and content colours: amber when lowered, the badge colour otherwise. */
@Composable
private fun badgeColours(badge: DiagramBadge): Pair<Color, Color> = when {
    badge.lowered -> amberTint().let { it.container to it.onContainer }
    badge.verdict == DiagramVerdict.HI_RES -> MaterialTheme.colorScheme.primaryContainer to MaterialTheme.colorScheme.onPrimaryContainer
    else -> MaterialTheme.colorScheme.surfaceVariant to MaterialTheme.colorScheme.onSurfaceVariant
}

/**
 * The quality badge: the weakest link, then what is heard ("Transcoded · Opus 160"), from
 * the diagram (never the file's format alone when this phone plays). A tap opens the
 * signal path. [compact]: the small form beside the artist.
 */
@Composable
fun QualityBadge(badge: DiagramBadge, onClick: () -> Unit, modifier: Modifier = Modifier, compact: Boolean = false) {
    val label = stringResource(R.string.quality_show_path, badge.text)
    val (container, content) = badgeColours(badge)
    Surface(
        onClick = onClick,
        shape = RoundedCornerShape(50),
        color = container,
        contentColor = content,
        modifier = modifier.testTag("quality-badge").semantics {
            contentDescription = label
            amber = badge.lowered
        },
    ) {
        Text(
            badge.text,
            style = if (compact) MaterialTheme.typography.labelSmall else MaterialTheme.typography.labelLarge,
            maxLines = 1,
            modifier = Modifier.padding(horizontal = if (compact) 8.dp else 12.dp, vertical = if (compact) 3.dp else 6.dp),
        )
    }
}

/** The verdict word alone ("Hi-Res", "Lossless", "Transcoded", "Lossy"): the badge's compact form. */
@Composable
fun verdictWord(verdict: DiagramVerdict): String = stringResource(
    when (verdict) {
        DiagramVerdict.HI_RES -> R.string.badge_hi_res
        DiagramVerdict.LOSSLESS -> R.string.badge_lossless
        DiagramVerdict.TRANSCODED -> R.string.badge_transcoded
        DiagramVerdict.LOSSY -> R.string.badge_lossy
    },
)

/**
 * The badge as a small chip for the mini player's second line: the verdict word alone, in
 * the badge's colours (amber for Transcoded and Lossy), so the artist keeps its room; the
 * full text ("Hi-Res · FLAC 24/96") is on Now playing and read out by TalkBack. Not a
 * button: the bar opens Now playing.
 */
@Composable
fun QualityChip(badge: DiagramBadge, modifier: Modifier = Modifier) {
    val (container, content) = badgeColours(badge)
    Surface(
        shape = RoundedCornerShape(50),
        color = container,
        contentColor = content,
        modifier = modifier.testTag("mini-quality").semantics(mergeDescendants = true) {
            contentDescription = badge.text
            amber = badge.lowered
        },
    ) {
        Text(
            verdictWord(badge.verdict),
            style = MaterialTheme.typography.labelSmall,
            maxLines = 1,
            softWrap = false,
            modifier = Modifier.padding(horizontal = 6.dp, vertical = 1.dp),
        )
    }
}

/**
 * The signal path as a block diagram: the blocks as cards, a column on compact widths and a
 * row with arrows between on medium and expanded ones; the block that lowers the verdict
 * tinted amber. [onLevel] (this phone only, with [QualityDiagram.switch]) shows the quality
 * switch under the gateway block; [details] goes under the device block, collapsed.
 */
@Composable
fun QualityDiagramView(
    diagram: QualityDiagram,
    modifier: Modifier = Modifier,
    onLevel: ((StreamQuality?) -> Unit)? = null,
    details: (@Composable () -> Unit)? = null,
) {
    val wide = LocalWindowLayout.current.width != WidthClass.COMPACT
    val switch = diagram.switch?.takeIf { onLevel != null }
    val blocks = diagram.blocks
    // Under the gateway block when there is one, else under the first.
    val switchAt = blocks.indexOfFirst { it.kind == DiagramBlockKind.GATEWAY }
    val detailsAt = blocks.indexOfLast { it.kind == DiagramBlockKind.DEVICE }.takeIf { it >= 0 } ?: blocks.lastIndex
    var open by rememberSaveable { mutableStateOf(false) }
    val extras: @Composable (Int) -> Unit = { i ->
        if (switch != null && i == switchAt) QualitySwitch(switch, onLevel!!)
        if (details != null && i == detailsAt) Details(open, { open = !open }, details)
    }
    if (wide) {
        Column(modifier.testTag("quality-diagram").semantics { isTraversalGroup = true }, verticalArrangement = Arrangement.spacedBy(12.dp)) {
            Row(
                Modifier.fillMaxWidth().height(IntrinsicSize.Min).testTag("quality-diagram-row"),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                blocks.forEachIndexed { i, b ->
                    if (i > 0) {
                        Icon(
                            Icons.AutoMirrored.Outlined.ArrowForward,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 4.dp).size(20.dp),
                        )
                    }
                    BlockCard(b, Modifier.weight(1f).fillMaxHeight())
                }
            }
            blocks.indices.forEach { extras(it) }
        }
    } else {
        Column(
            modifier.testTag("quality-diagram").semantics { isTraversalGroup = true },
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            blocks.forEachIndexed { i, b ->
                if (i > 0) {
                    Icon(
                        Icons.Outlined.ArrowDownward,
                        contentDescription = null,
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(20.dp),
                    )
                }
                BlockCard(b, Modifier.fillMaxWidth())
                extras(i)
            }
        }
    }
}

/** One block: its title, the format at that point and what happened there; amber when it lowers the verdict. */
@Composable
private fun BlockCard(block: DiagramBlock, modifier: Modifier = Modifier) {
    val amberColours = amberTint()
    val lowers = stringResource(R.string.block_lowers)
    val what = listOf(block.format, block.note).filter { it.isNotBlank() }.joinToString(", ")
    val spoken = buildString {
        append(if (what.isEmpty()) block.title else stringResource(R.string.block_spoken, block.title, what))
        if (block.lowers) append(", ").append(lowers)
    }
    Card(
        colors = if (block.lowers) {
            CardDefaults.cardColors(containerColor = amberColours.container, contentColor = amberColours.onContainer)
        } else {
            CardDefaults.cardColors()
        },
        modifier = modifier
            .testTag("diagram-block")
            .semantics(mergeDescendants = true) {
                contentDescription = spoken
                amber = block.lowers
            },
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(block.title, style = MaterialTheme.typography.labelLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (block.format.isNotBlank()) {
                Text(block.format, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (block.note.isNotBlank()) {
                Text(block.note, style = MaterialTheme.typography.bodySmall)
            }
        }
    }
}

/**
 * The quality switch: Original · High · Data saver, the level in force selected, why it is
 * in force under it, and "Back to automatic" while one chosen here is in force.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun QualitySwitch(switch: DiagramSwitch, onLevel: (StreamQuality?) -> Unit) {
    Column(Modifier.fillMaxWidth().testTag("quality-switch"), verticalArrangement = Arrangement.spacedBy(4.dp)) {
        Text(stringResource(R.string.switch_title), style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.primary)
        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
            switch.levels.forEachIndexed { i, level ->
                SegmentedButton(
                    selected = level == switch.inForce,
                    onClick = { if (level != switch.inForce) onLevel(level) },
                    shape = SegmentedButtonDefaults.itemShape(i, switch.levels.size),
                    modifier = Modifier.testTag("quality-switch-${level.name.lowercase()}"),
                ) {
                    Text(qualityName(level), maxLines = 1)
                }
            }
        }
        if (switch.originText.isNotBlank()) {
            Text(
                switch.originText,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("quality-switch-origin"),
            )
        }
        Text(
            stringResource(if (switch.seekable) R.string.switch_now else R.string.switch_next_track),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag(if (switch.seekable) "quality-switch-now" else "quality-switch-next-track"),
        )
        if (switch.overridden) {
            TextButton(onClick = { onLevel(null) }, modifier = Modifier.testTag("quality-switch-automatic")) {
                Text(stringResource(R.string.switch_back_to_automatic))
            }
        }
    }
}

/** "Details": today's stage list and the rest, collapsed until asked for. */
@Composable
private fun Details(open: Boolean, onToggle: () -> Unit, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxWidth()) {
        val state = stringResource(if (open) R.string.block_details_hide else R.string.block_details_show)
        TextButton(
            onClick = onToggle,
            modifier = Modifier.testTag("diagram-details-toggle").semantics { stateDescription = state },
        ) {
            Text(stringResource(R.string.block_details))
            Icon(if (open) Icons.Outlined.ExpandLess else Icons.Outlined.ExpandMore, contentDescription = null, modifier = Modifier.padding(start = 4.dp))
        }
        AnimatedVisibility(open) {
            Box(Modifier.fillMaxWidth().testTag("diagram-details")) { content() }
        }
    }
}
