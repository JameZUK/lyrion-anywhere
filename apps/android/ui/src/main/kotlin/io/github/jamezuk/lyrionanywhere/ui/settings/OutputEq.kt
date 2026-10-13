package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.CompareArrows
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material3.Card
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.disabled
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.EqCurve
import io.github.jamezuk.lyrionanywhere.data.EqProfile
import io.github.jamezuk.lyrionanywhere.data.OutputClass
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** "Wired headphones", "Bluetooth · <name>", "Any USB DAC"… */
@Composable
fun outputName(kind: OutputClass, name: String): String = when (kind) {
    OutputClass.SPEAKER -> stringResource(R.string.output_speaker)
    OutputClass.WIRED -> stringResource(R.string.output_wired)
    OutputClass.USB -> if (name.isEmpty()) stringResource(R.string.output_usb_any) else stringResource(R.string.output_usb_named, name)
    OutputClass.BLUETOOTH ->
        if (name.isEmpty()) stringResource(R.string.output_bluetooth_any) else stringResource(R.string.output_bluetooth_named, name)
}

/**
 * The profile's curve (Wave 2 §5.3): its gain from 20 Hz to 20 kHz, with the 0 dB line.
 * TalkBack reads its range; nothing here can be pressed.
 */
@Composable
fun EqCurveView(profile: EqProfile, modifier: Modifier = Modifier) {
    val curve = remember(profile.text) { EqCurve.parse(profile.text)?.let { EqCurve.response(it, CURVE_POINTS) } } ?: return
    val low = curve.minOrNull() ?: 0.0
    val high = curve.maxOrNull() ?: 0.0
    val described = stringResource(R.string.eq_curve_described, profile.name, formatDb(low), formatDb(high))
    val line = MaterialTheme.colorScheme.primary
    val grid = MaterialTheme.colorScheme.outlineVariant
    Card(modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 4.dp)) {
        Column(Modifier.padding(12.dp)) {
            Text(stringResource(R.string.eq_curve_title, profile.name), style = MaterialTheme.typography.titleSmall)
            Canvas(
                Modifier
                    .fillMaxWidth()
                    .height(120.dp)
                    .padding(top = 8.dp)
                    .testTag("eq-curve")
                    .semantics { contentDescription = described },
            ) {
                val range = EqCurve.RANGE_DB.toFloat()
                fun y(db: Double) = size.height / 2 - (db.toFloat() / range) * (size.height / 2)
                for (g in listOf(-12.0, -6.0, 0.0, 6.0, 12.0)) {
                    drawLine(grid, Offset(0f, y(g)), Offset(size.width, y(g)), strokeWidth = if (g == 0.0) 3f else 1f)
                }
                val path = Path()
                val step = size.width / (curve.size - 1)
                curve.forEachIndexed { i, db -> if (i == 0) path.moveTo(0f, y(db)) else path.lineTo(i * step, y(db)) }
                drawPath(path, line, style = Stroke(width = 5f, cap = StrokeCap.Round))
            }
            Row(Modifier.fillMaxWidth()) {
                Text("20 Hz", style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
                Text("1 kHz", style = MaterialTheme.typography.labelSmall, modifier = Modifier.weight(1f))
                Text("20 kHz", style = MaterialTheme.typography.labelSmall)
            }
        }
    }
}

private const val CURVE_POINTS = 96

/**
 * A/B: hear the music without the EQ while this is held, and with it again on release. With
 * TalkBack (or a keyboard) it is a switch instead: each press turns the bypass on or off.
 * The bypass never outlives the screen.
 */
@Composable
fun EqCompareButton(bypass: Boolean, enabled: Boolean, onBypass: (Boolean) -> Unit) {
    val latest by rememberUpdatedState(onBypass)
    DisposableEffect(Unit) { onDispose { latest(false) } }
    val label = stringResource(R.string.eq_compare)
    val stateText = stringResource(if (bypass) R.string.eq_compare_without else R.string.eq_compare_with)
    Surface(
        shape = MaterialTheme.shapes.medium,
        color = if (bypass) MaterialTheme.colorScheme.tertiaryContainer else MaterialTheme.colorScheme.secondaryContainer,
        modifier = Modifier
            .padding(horizontal = 16.dp, vertical = 4.dp)
            .fillMaxWidth()
            .heightIn(min = 56.dp)
            .testTag("eq-compare")
            .pointerInput(enabled) {
                if (enabled) {
                    detectTapGestures(onPress = {
                        latest(true)
                        try {
                            tryAwaitRelease()
                        } finally {
                            latest(false)
                        }
                    })
                }
            }
            .semantics(mergeDescendants = true) {
                role = Role.Switch
                contentDescription = label
                stateDescription = stateText
                toggleableState = ToggleableState(bypass)
                if (enabled) {
                    onClick(label) {
                        latest(!bypass)
                        true
                    }
                } else {
                    disabled()
                }
            },
    ) {
        Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.AutoMirrored.Outlined.CompareArrows, contentDescription = null)
            Column(Modifier.padding(start = 12.dp)) {
                Text(label, style = MaterialTheme.typography.titleSmall)
                Text(
                    stringResource(if (enabled) R.string.eq_compare_hint else R.string.eq_compare_unavailable),
                    style = MaterialTheme.typography.bodySmall,
                )
            }
        }
    }
}

/**
 * Per-output EQ (Wave 2 §5.3): which profile each output uses, and binding one: the
 * current output, or any built-in speaker, wired headphones, USB DAC or Bluetooth device.
 * A bound output uses its own profile (or none); others use the main one. The profile
 * switches by itself as the output changes. Device names stay on this phone.
 */
@OptIn(androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
fun OutputEqSection(state: SoundUiState, actions: SoundActions) {
    val s = state.sound
    val current = state.output
    val targets = remember(current) {
        buildList {
            current?.let(::add)
            add(AudioOutput(OutputClass.SPEAKER))
            add(AudioOutput(OutputClass.WIRED))
            add(AudioOutput(OutputClass.USB))
            add(AudioOutput(OutputClass.BLUETOOTH))
        }.distinct()
    }
    var chosen by rememberSaveable { mutableIntStateOf(0) }
    val target = targets.getOrElse(chosen) { targets.first() }
    var pasting by rememberSaveable { mutableStateOf(false) }
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) {
            scope.launch {
                val read = withContext(Dispatchers.IO) { readProfile(context, uri) }
                if (read == null) actions.eqUnreadable() else actions.importEqFor(target.kind, target.name, read.first, read.second)
            }
        }
    }
    Column(Modifier.testTag("output-eq")) {
        Text(
            stringResource(R.string.output_eq_title),
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.padding(start = 16.dp, end = 16.dp, top = 12.dp).semantics { heading() },
        )
        Text(
            stringResource(R.string.output_eq_detail),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
        )
        if (current != null) {
            val using = state.effective.let { if (it.eqEnabled) it.eq?.name else null }
            ListItem(
                headlineContent = { Text(stringResource(R.string.output_eq_now, outputName(current.kind, current.name))) },
                supportingContent = { Text(using?.let { stringResource(R.string.output_eq_using, it) } ?: stringResource(R.string.output_eq_using_none)) },
                modifier = Modifier.testTag("output-eq-now"),
            )
        }
        s.outputEq.forEach { b ->
            val name = outputName(b.kind, b.deviceName)
            ListItem(
                headlineContent = { Text(name) },
                supportingContent = { Text(b.profile?.name ?: stringResource(R.string.output_eq_off_here)) },
                trailingContent = {
                    IconButton(onClick = { actions.unbindOutput(b.kind, b.deviceName) }, modifier = Modifier.testTag("output-eq-remove-${b.kind}-${b.deviceName}")) {
                        Icon(Icons.Outlined.Delete, contentDescription = stringResource(R.string.output_eq_remove, name))
                    }
                },
                modifier = Modifier.testTag("output-eq-${b.kind}-${b.deviceName}"),
            )
        }
        SectionHeader(stringResource(R.string.output_eq_bind))
        Column(Modifier.selectableGroup()) {
            targets.forEachIndexed { i, t ->
                ListItem(
                    leadingContent = { RadioButton(selected = i == chosen, onClick = null) },
                    headlineContent = { Text(outputName(t.kind, t.name)) },
                    supportingContent = if (t == current) ({ Text(stringResource(R.string.output_eq_current)) }) else null,
                    modifier = Modifier
                        .selectable(selected = i == chosen, role = Role.RadioButton) { chosen = i }
                        .testTag("output-eq-target-$i"),
                )
            }
        }
        FlowRow(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
            OutlinedButton(
                onClick = { actions.bindOutput(target.kind, target.name, s.eq) },
                enabled = s.eq != null,
                modifier = Modifier.testTag("output-eq-use-main"),
            ) { Text(stringResource(R.string.output_eq_use_main)) }
            OutlinedButton(onClick = { actions.bindOutput(target.kind, target.name, null) }, modifier = Modifier.testTag("output-eq-none")) {
                Text(stringResource(R.string.output_eq_none))
            }
            OutlinedButton(onClick = { pasting = !pasting }, modifier = Modifier.testTag("output-eq-paste")) {
                Text(stringResource(R.string.output_eq_paste))
            }
            OutlinedButton(onClick = { picker.launch(arrayOf("text/plain", "application/octet-stream")) }) {
                Text(stringResource(R.string.output_eq_file))
            }
        }
        if (pasting) {
            val pastedName = stringResource(R.string.sound_eq_pasted_name)
            PasteBox(
                onCancel = { pasting = false },
                onImport = { text ->
                    pasting = false
                    actions.importEqFor(target.kind, target.name, pastedName, text)
                },
            )
        }
    }
}
