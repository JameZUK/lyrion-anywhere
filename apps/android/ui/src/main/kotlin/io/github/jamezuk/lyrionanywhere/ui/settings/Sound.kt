package io.github.jamezuk.lyrionanywhere.ui.settings

import android.content.Context
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import android.net.Uri
import android.os.Build
import android.provider.OpenableColumns
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.data.CrossfeedMode
import io.github.jamezuk.lyrionanywhere.data.DitherMode
import io.github.jamezuk.lyrionanywhere.data.EqCheck
import io.github.jamezuk.lyrionanywhere.data.EqProfile
import io.github.jamezuk.lyrionanywhere.data.AudioOutput
import io.github.jamezuk.lyrionanywhere.data.OutputClass
import io.github.jamezuk.lyrionanywhere.data.OutputEq
import io.github.jamezuk.lyrionanywhere.data.LevellingMode
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.SoundStage
import io.github.jamezuk.lyrionanywhere.data.TransitionsMode
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.util.Locale

/** A one-off result of an EQ import, for the screen to show. */
sealed interface EqMessage {
    data class Imported(val name: String) : EqMessage

    data class Invalid(val reason: String) : EqMessage

    data object Flat : EqMessage

    data object Unreadable : EqMessage
}

data class SoundUiState(
    val sound: SoundSettings = SoundSettings(),
    /** Android 14+: bit-perfect USB output is possible at all. */
    val bitPerfectAvailable: Boolean = true,
    val message: EqMessage? = null,
    /** Where the sound goes now (null: not known: this phone isn't playing). */
    val output: AudioOutput? = null,
    /** The EQ is bypassed for a comparison (A/B). */
    val bypass: Boolean = false,
    /**
     * Lyrion's own level for this phone's player (LMS's volume for it, 0–100); null while
     * this phone isn't a player in LMS (or it isn't known yet).
     */
    val playerLevel: Int? = null,
) {
    /** The settings as they apply on the current output (per-output EQ). */
    val effective: SoundSettings get() = sound.forOutput(output)

    /** The profile the curve shows: the one in use here, else the main one. */
    val shownProfile: EqProfile? get() = effective.eq ?: sound.eq
}

/** The Sound settings: every change is saved and applied to the engine at once. */
class SoundViewModel(
    private val local: LocalPlayerRepository,
    sdk: Int = Build.VERSION.SDK_INT,
    /** LMS, for this phone's player level (none: not offered). */
    private val players: PlayersRepository? = null,
) : ViewModel() {
    private val _state = MutableStateFlow(
        SoundUiState(sound = local.state.value.sound, bitPerfectAvailable = sdk >= BIT_PERFECT_API),
    )
    val state: StateFlow<SoundUiState> = _state.asStateFlow()

    init {
        viewModelScope.launch {
            local.state.collect { l -> _state.update { it.copy(sound = l.sound, output = l.output, bypass = l.eqBypass) } }
        }
        if (players != null) {
            viewModelScope.launch {
                // The level LMS holds for this phone's player, read when the player (re)appears.
                local.state.map { it.playerId }.distinctUntilChanged().collect { id ->
                    val level = id?.let { (players.status(it) as? ApiResult.Ok)?.value?.volume?.toInt() }
                    _state.update { it.copy(playerLevel = level?.coerceIn(0, 100)) }
                }
            }
        }
    }

    /**
     * Lyrion's own level for this phone's player (LMS's volume for it, applied in the engine
     * before the phone's media volume). Now playing's volume no longer touches it while this
     * phone plays; LMS (another controller, an alarm) still can.
     */
    fun setPlayerLevel(level: Int) {
        val repo = players ?: return
        val id = local.state.value.playerId ?: return
        val v = level.coerceIn(0, 100)
        _state.update { it.copy(playerLevel = v) }
        viewModelScope.launch { repo.setVolume(id, v) }
    }

    private fun change(transform: (SoundSettings) -> SoundSettings) {
        val s = transform(_state.value.sound).sanitised()
        _state.update { it.copy(sound = s) }
        viewModelScope.launch { local.setSound(s) }
    }

    fun setPreamp(db: Double) = change { it.copy(preampDb = roundTo(db, 0.5)) }

    fun setLevelling(mode: LevellingMode) = change { it.copy(levelling = mode) }

    fun setLimiter(on: Boolean) = change { it.copy(limiter = on) }

    fun setCeiling(db: Double) = change { it.copy(limiterCeilingDb = roundTo(db, 0.1)) }

    fun setEqEnabled(on: Boolean) = change { it.copy(eqEnabled = on && (it.eq != null || it.outputEq.any { b -> b.profile != null })) }

    /** Bind [profile] (null: no EQ) to an output: a kind, and a device's name for USB or Bluetooth. */
    fun bindOutput(kind: OutputClass, name: String, profile: EqProfile?) =
        change { it.copy(outputEq = it.outputEq + OutputEq(kind, name, profile), eqEnabled = it.eqEnabled || profile != null) }

    /** Forget an output's binding: the main profile applies there again. */
    fun unbindOutput(kind: OutputClass, name: String) {
        val n = if (kind.named) AudioOutput.cleanName(name) else ""
        change { it.copy(outputEq = it.outputEq.filterNot { b -> b.kind == kind && b.deviceName == n }) }
    }

    /** Import an AutoEq profile for one output (as [importEq], but bound to it). */
    fun importEqFor(kind: OutputClass, deviceName: String, name: String, text: String) {
        val message = when (val p = checked(name, text)) {
            is EqMessage.Imported -> {
                bindOutput(kind, deviceName, imported)
                p
            }
            else -> p
        }
        _state.update { it.copy(message = message) }
    }

    /** The A/B comparison: bypass the EQ while held (or toggled with TalkBack). */
    fun setBypass(on: Boolean) = local.setEqBypass(on)

    override fun onCleared() {
        // A comparison never outlives the screen.
        local.setEqBypass(false)
    }

    /** The last profile [checked] accepted. */
    private var imported: EqProfile? = null

    private fun checked(name: String, text: String): EqMessage = when (val c = local.checkEq(text)) {
        is EqCheck.Invalid -> EqMessage.Invalid(c.reason)
        is EqCheck.Ok -> if (c.flat || c.filters == 0) {
            EqMessage.Flat
        } else {
            val profile = EqProfile(cleanName(name), text, c.filters, c.preampDb)
            imported = profile
            EqMessage.Imported(profile.name)
        }
    }

    fun setCrossfeed(mode: CrossfeedMode) = change { it.copy(crossfeed = mode) }

    fun setLoudness(on: Boolean) = change { it.copy(loudness = on) }

    fun setDither(mode: DitherMode) = change { it.copy(dither = mode) }

    fun setBitPerfectUsb(on: Boolean) = change { it.copy(bitPerfectUsb = on) }

    fun setTransitions(mode: TransitionsMode) = change { it.copy(transitions = mode) }

    fun setCrossfadeSecs(secs: Double) = change { it.copy(crossfadeSecs = Math.round(secs).toDouble()) }

    /** Import an AutoEq profile (pasted, or read from a file), if the engine accepts it. */
    fun importEq(name: String, text: String) {
        val message = when (val c = local.checkEq(text)) {
            is EqCheck.Invalid -> EqMessage.Invalid(c.reason)
            is EqCheck.Ok -> if (c.flat || c.filters == 0) {
                EqMessage.Flat
            } else {
                val profile = EqProfile(cleanName(name), text, c.filters, c.preampDb)
                change { it.copy(eq = profile, eqEnabled = true) }
                EqMessage.Imported(profile.name)
            }
        }
        _state.update { it.copy(message = message) }
    }

    fun eqUnreadable() = _state.update { it.copy(message = EqMessage.Unreadable) }

    fun removeEq() = change { it.copy(eq = null, eqEnabled = false) }

    fun messageShown() = _state.update { it.copy(message = null) }

    companion object {
        /** `AudioManager` bit-perfect mixer attributes. */
        const val BIT_PERFECT_API = 34

        private fun roundTo(v: Double, step: Double) = Math.round(v / step) * step

        /** A file name fit to show: printable, at most 64 characters, never empty. */
        fun cleanName(name: String): String =
            name.filter { !it.isISOControl() }.trim().take(64).ifEmpty { "EQ" }
    }
}

/** What the Sound screen can ask for. */
interface SoundActions {
    fun preamp(db: Double) {}

    fun levelling(mode: LevellingMode) {}

    fun limiter(on: Boolean) {}

    fun ceiling(db: Double) {}

    fun eqEnabled(on: Boolean) {}

    fun importEq(name: String, text: String) {}

    fun eqUnreadable() {}

    fun removeEq() {}

    fun bindOutput(kind: OutputClass, name: String, profile: EqProfile?) {}

    fun unbindOutput(kind: OutputClass, name: String) {}

    fun importEqFor(kind: OutputClass, deviceName: String, name: String, text: String) {}

    fun eqBypass(on: Boolean) {}

    fun loudness(on: Boolean) {}

    fun crossfeed(mode: CrossfeedMode) {}

    fun dither(mode: DitherMode) {}

    fun bitPerfectUsb(on: Boolean) {}

    fun transitions(mode: TransitionsMode) {}

    fun crossfadeSecs(secs: Double) {}

    /** This phone's player level in LMS (0–100). */
    fun playerLevel(level: Int) {}

    fun messageShown() {}
}

fun SoundViewModel.actions(): SoundActions = object : SoundActions {
    override fun preamp(db: Double) = setPreamp(db)

    override fun levelling(mode: LevellingMode) = setLevelling(mode)

    override fun limiter(on: Boolean) = setLimiter(on)

    override fun ceiling(db: Double) = setCeiling(db)

    override fun eqEnabled(on: Boolean) = setEqEnabled(on)

    override fun importEq(name: String, text: String) = this@actions.importEq(name, text)

    override fun eqUnreadable() = this@actions.eqUnreadable()

    override fun removeEq() = this@actions.removeEq()

    override fun bindOutput(kind: OutputClass, name: String, profile: EqProfile?) = this@actions.bindOutput(kind, name, profile)

    override fun unbindOutput(kind: OutputClass, name: String) = this@actions.unbindOutput(kind, name)

    override fun importEqFor(kind: OutputClass, deviceName: String, name: String, text: String) =
        this@actions.importEqFor(kind, deviceName, name, text)

    override fun eqBypass(on: Boolean) = setBypass(on)

    override fun loudness(on: Boolean) = setLoudness(on)

    override fun crossfeed(mode: CrossfeedMode) = setCrossfeed(mode)

    override fun dither(mode: DitherMode) = setDither(mode)

    override fun bitPerfectUsb(on: Boolean) = setBitPerfectUsb(on)

    override fun transitions(mode: TransitionsMode) = setTransitions(mode)

    override fun crossfadeSecs(secs: Double) = setCrossfadeSecs(secs)

    override fun playerLevel(level: Int) = setPlayerLevel(level)

    override fun messageShown() = this@actions.messageShown()
}

/** "preamp, EQ" for the active stages. */
@Composable
fun stageNames(stages: List<SoundStage>): String = stages.map {
    stringResource(
        when (it) {
            SoundStage.PREAMP -> R.string.stage_name_preamp
            SoundStage.LEVELLING -> R.string.stage_name_levelling
            SoundStage.EQ -> R.string.stage_name_eq
            SoundStage.LOUDNESS -> R.string.stage_name_loudness
            SoundStage.CROSSFEED -> R.string.stage_name_crossfeed
            SoundStage.CROSSFADE -> R.string.stage_name_crossfade
        },
    )
}.joinToString(", ")

/** "−3.5 dB", "+2.0 dB", "0.0 dB". */
fun formatDb(db: Double): String = when {
    db < 0 -> "−${String.format(Locale.ROOT, "%.1f", -db)} dB"
    db > 0 -> "+${String.format(Locale.ROOT, "%.1f", db)} dB"
    else -> "0.0 dB"
}

@Composable
fun SoundScreen(state: SoundUiState, actions: SoundActions, modifier: Modifier = Modifier) {
    val s = state.sound
    var pasting by rememberSaveable { mutableStateOf(false) }
    LazyColumn(modifier.fillMaxSize()) {
        item { BitPerfectIndicator(state.effective) }
        state.message?.let { m -> item { EqMessageLine(m, actions) } }

        item { SectionHeader(stringResource(R.string.sound_section_output)) }
        item {
            SwitchRow(
                title = stringResource(R.string.sound_bit_perfect_usb),
                body = stringResource(
                    if (state.bitPerfectAvailable) R.string.sound_bit_perfect_usb_body else R.string.sound_bit_perfect_usb_old,
                ),
                checked = s.bitPerfectUsb,
                enabled = state.bitPerfectAvailable,
                onChange = actions::bitPerfectUsb,
            )
        }
        item { HorizontalDivider() }

        item { SectionHeader(stringResource(R.string.sound_section_loudness)) }
        item { PlayerLevel(state.playerLevel, actions::playerLevel) }
        item {
            RadioGroup(
                label = stringResource(R.string.sound_levelling),
                options = listOf(
                    Triple(LevellingMode.OFF, R.string.levelling_off, R.string.levelling_off_body),
                    Triple(LevellingMode.REPLAYGAIN_TRACK, R.string.levelling_rg_track, R.string.levelling_rg_track_body),
                    Triple(LevellingMode.REPLAYGAIN_ALBUM, R.string.levelling_rg_album, R.string.levelling_rg_album_body),
                    Triple(LevellingMode.R128, R.string.levelling_r128, R.string.levelling_r128_body),
                ),
                selected = s.levelling,
                onSelect = actions::levelling,
            )
        }
        item {
            DbSlider(
                label = { stringResource(R.string.sound_preamp_value, formatDb(it)) },
                accessibleName = stringResource(R.string.sound_preamp),
                value = s.preampDb,
                range = SoundSettings.PREAMP_RANGE,
                step = 0.5,
                onChange = actions::preamp,
                tag = "preamp",
            )
        }
        item {
            SwitchRow(
                title = stringResource(R.string.sound_limiter),
                body = stringResource(R.string.sound_limiter_body),
                checked = s.limiter,
                onChange = actions::limiter,
            )
        }
        if (s.limiter) {
            item {
                DbSlider(
                    label = {
                        stringResource(R.string.sound_limiter_ceiling, String.format(Locale.ROOT, "%.1f", it).replace("-", "−"))
                    },
                    accessibleName = stringResource(R.string.sound_limiter_ceiling_name),
                    value = s.limiterCeilingDb,
                    range = SoundSettings.CEILING_RANGE,
                    step = 0.1,
                    onChange = actions::ceiling,
                    tag = "ceiling",
                )
            }
        }
        item { HorizontalDivider() }

        item { SectionHeader(stringResource(R.string.sound_section_eq)) }
        item {
            val eq = s.eq
            SwitchRow(
                title = stringResource(R.string.sound_eq),
                body = if (eq == null) {
                    stringResource(R.string.sound_eq_none)
                } else {
                    stringResource(
                        R.string.sound_eq_profile,
                        eq.name,
                        pluralStringResource(R.plurals.sound_eq_filters, eq.filters, eq.filters),
                        formatDb(eq.preampDb),
                    )
                },
                checked = s.eqEnabled,
                enabled = eq != null || s.outputEq.any { it.profile != null },
                onChange = actions::eqEnabled,
            )
        }
        item { EqButtons(hasProfile = s.eq != null, onPaste = { pasting = !pasting }, actions = actions) }
        state.shownProfile?.let { profile ->
            item { EqCurveView(profile) }
            item { EqCompareButton(state.bypass, enabled = state.output != null && state.effective.eqEnabled, onBypass = actions::eqBypass) }
        }
        item { OutputEqSection(state, actions) }
        item {
            // Driven by this phone's player volume (not Android's); the engine bypasses it
            // on a bit-perfect route, where the DAC's volume rules.
            SwitchRow(
                title = stringResource(R.string.sound_loudness_volume),
                body = stringResource(R.string.sound_loudness_volume_body),
                checked = s.loudness,
                onChange = actions::loudness,
                tag = "loudness-volume",
            )
        }
        if (pasting) {
            item {
                val pastedName = stringResource(R.string.sound_eq_pasted_name)
                PasteBox(
                    onCancel = { pasting = false },
                    onImport = { text ->
                        pasting = false
                        actions.importEq(pastedName, text)
                    },
                )
            }
        }
        item { HorizontalDivider() }

        item { SectionHeader(stringResource(R.string.sound_section_crossfeed)) }
        item {
            RadioGroup(
                label = null, // The section header above names it.
                options = listOf(
                    Triple(CrossfeedMode.OFF, R.string.crossfeed_off, null),
                    Triple(CrossfeedMode.LIGHT, R.string.crossfeed_light, null),
                    Triple(CrossfeedMode.MODERATE, R.string.crossfeed_moderate, null),
                    Triple(CrossfeedMode.NATURAL, R.string.crossfeed_natural, null),
                ),
                selected = s.crossfeed,
                onSelect = actions::crossfeed,
            )
        }
        item { HorizontalDivider() }

        item { SectionHeader(stringResource(R.string.sound_section_transitions)) }
        item {
            RadioGroup(
                label = null, // The section header above names it.
                options = listOf(
                    Triple(TransitionsMode.OFF, R.string.transitions_off, R.string.transitions_off_body),
                    Triple(TransitionsMode.CROSSFADE, R.string.transitions_crossfade, R.string.transitions_crossfade_body),
                    Triple(TransitionsMode.SMART, R.string.transitions_smart, R.string.transitions_smart_body),
                ),
                selected = s.transitions,
                onSelect = actions::transitions,
            )
        }
        if (s.transitions != TransitionsMode.OFF) {
            item {
                DbSlider(
                    label = { stringResource(R.string.sound_crossfade_length, Math.round(it).toInt()) },
                    accessibleName = stringResource(R.string.sound_crossfade_length_name),
                    value = s.crossfadeSecs,
                    range = SoundSettings.CROSSFADE_RANGE,
                    step = 1.0,
                    onChange = actions::crossfadeSecs,
                    tag = "crossfade",
                )
            }
        }
        item { HorizontalDivider() }

        item { SectionHeader(stringResource(R.string.sound_section_dither)) }
        item {
            Text(
                stringResource(R.string.sound_dither_body),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        }
        item {
            RadioGroup(
                label = null, // The section header above names it.
                options = listOf(
                    Triple(DitherMode.TPDF, R.string.dither_tpdf, null),
                    Triple(DitherMode.TPDF_SHAPED, R.string.dither_shaped, null),
                    Triple(DitherMode.OFF, R.string.dither_off, null),
                ),
                selected = s.dither,
                onSelect = actions::dither,
            )
        }
    }
}

/** "Bit-perfect" while every stage is off; otherwise which stages change the samples. */
@Composable
private fun BitPerfectIndicator(s: SoundSettings) {
    val perfect = s.bitPerfect
    Card(
        colors = CardDefaults.cardColors(
            containerColor = if (perfect) {
                MaterialTheme.colorScheme.primaryContainer
            } else {
                MaterialTheme.colorScheme.secondaryContainer
            },
        ),
        modifier = Modifier
            .fillMaxWidth()
            .padding(16.dp)
            .testTag("sound-indicator")
            .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
    ) {
        Row(Modifier.padding(16.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(if (perfect) Icons.Outlined.CheckCircle else Icons.Outlined.Tune, contentDescription = null)
            Column(Modifier.padding(start = 12.dp)) {
                Text(
                    stringResource(if (perfect) R.string.sound_bit_perfect_title else R.string.sound_processing_title),
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.semantics { heading() },
                )
                Text(
                    if (perfect) {
                        stringResource(R.string.sound_bit_perfect_body)
                    } else {
                        stringResource(R.string.sound_processing_body, stageNames(s.activeStages))
                    },
                    style = MaterialTheme.typography.bodyMedium,
                )
            }
        }
    }
}

@Composable
private fun EqMessageLine(m: EqMessage, actions: SoundActions) {
    val text = when (m) {
        is EqMessage.Imported -> null
        is EqMessage.Invalid -> stringResource(R.string.sound_eq_invalid, m.reason)
        EqMessage.Flat -> stringResource(R.string.sound_eq_flat)
        EqMessage.Unreadable -> stringResource(R.string.sound_eq_unreadable)
    } ?: return
    ListItem(
        headlineContent = { Text(text, color = MaterialTheme.colorScheme.error) },
        trailingContent = {
            TextButton(onClick = actions::messageShown) { Text(stringResource(android.R.string.ok)) }
        },
        modifier = Modifier.semantics { liveRegion = LiveRegionMode.Assertive },
    )
}

@Composable
private fun SwitchRow(
    title: String,
    body: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
    enabled: Boolean = true,
    tag: String? = null,
) {
    // The switch is decoration: the whole row is the toggle (one target for touch and
    // screen readers).
    ListItem(
        headlineContent = { Text(title) },
        supportingContent = { Text(body) },
        trailingContent = { Switch(checked = checked, onCheckedChange = null, enabled = enabled) },
        modifier = Modifier
            .then(if (tag != null) Modifier.testTag(tag) else Modifier)
            .toggleable(value = checked, enabled = enabled, role = Role.Switch, onValueChange = onChange),
    )
}

/** Radio choices; [label] names the group, or null when the section header above already does. */
@Composable
private fun <T> RadioGroup(
    label: String?,
    options: List<Triple<T, Int, Int?>>,
    selected: T,
    onSelect: (T) -> Unit,
) {
    Column(Modifier.selectableGroup().semantics { if (label != null) contentDescription = label }) {
        options.forEach { (value, title, body) ->
            ListItem(
                leadingContent = { RadioButton(selected = value == selected, onClick = null) },
                headlineContent = { Text(stringResource(title)) },
                supportingContent = body?.let { { Text(stringResource(it)) } },
                modifier = Modifier.selectable(
                    selected = value == selected,
                    role = Role.RadioButton,
                    onClick = { onSelect(value) },
                ),
            )
        }
    }
}

/** A slider in dB steps; the value is saved when the finger lifts. */
@Composable
private fun DbSlider(
    label: @Composable (Double) -> String,
    accessibleName: String,
    value: Double,
    range: ClosedFloatingPointRange<Double>,
    step: Double,
    onChange: (Double) -> Unit,
    tag: String,
) {
    var dragging by remember { mutableStateOf(false) }
    var current by remember { mutableFloatStateOf(value.toFloat()) }
    val shown = if (dragging) current.toDouble() else value
    val text = label(Math.round(shown / step) * step)
    Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
        Text(text, style = MaterialTheme.typography.bodyLarge)
        Slider(
            value = shown.toFloat(),
            onValueChange = {
                dragging = true
                current = it
            },
            onValueChangeFinished = {
                dragging = false
                onChange(current.toDouble())
            },
            valueRange = range.start.toFloat()..range.endInclusive.toFloat(),
            steps = ((range.endInclusive - range.start) / step).toInt() - 1,
            modifier = Modifier
                .testTag(tag)
                .semantics {
                    contentDescription = accessibleName
                    stateDescription = text
                },
        )
    }
}

/**
 * Settings › Sound › Player level: Lyrion's own level for this phone's player (the software
 * volume LMS sets), normally left at 100 %. Without a player in LMS, a note says how to get one.
 */
@Composable
private fun PlayerLevel(level: Int?, onChange: (Int) -> Unit) {
    var dragging by remember { mutableStateOf(false) }
    var current by remember { mutableFloatStateOf((level ?: 100).toFloat()) }
    val shown = if (dragging) current.toInt() else level ?: 100
    val name = stringResource(R.string.sound_player_level)
    Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp).testTag("player-level-setting")) {
        Text(
            if (level == null) name else stringResource(R.string.sound_player_level_value, shown),
            style = MaterialTheme.typography.bodyLarge,
        )
        Text(
            stringResource(if (level == null) R.string.sound_player_level_off else R.string.sound_player_level_body),
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        if (level != null) {
            Slider(
                value = shown.toFloat(),
                onValueChange = {
                    dragging = true
                    current = it
                },
                onValueChangeFinished = {
                    dragging = false
                    onChange(current.toInt())
                },
                valueRange = 0f..100f,
                modifier = Modifier
                    .testTag("player-level")
                    .semantics {
                        contentDescription = name
                        stateDescription = "$shown%"
                    },
            )
        }
    }
}

@Composable
private fun EqButtons(hasProfile: Boolean, onPaste: () -> Unit, actions: SoundActions) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val picker = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri ->
        if (uri != null) {
            scope.launch {
                val read = withContext(Dispatchers.IO) { readProfile(context, uri) }
                if (read == null) actions.eqUnreadable() else actions.importEq(read.first, read.second)
            }
        }
    }
    // Wraps onto two lines with large text, rather than squeezing either button.
    FlowRow(
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
    ) {
        OutlinedButton(onClick = onPaste) { Text(stringResource(R.string.sound_eq_paste)) }
        OutlinedButton(onClick = { picker.launch(arrayOf("text/plain", "application/octet-stream")) }) {
            Text(stringResource(R.string.sound_eq_file))
        }
    }
    if (hasProfile) {
        TextButton(onClick = actions::removeEq, modifier = Modifier.padding(horizontal = 8.dp)) {
            Text(stringResource(R.string.sound_eq_remove))
        }
    }
}

/** A picked profile file's name and text, read with a hard size limit; null if unreadable. */
internal fun readProfile(context: Context, uri: Uri): Pair<String, String>? = try {
    val resolver = context.contentResolver
    val name = resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
        if (c.moveToFirst()) c.getString(0) else null
    } ?: ""
    val bytes = resolver.openInputStream(uri)?.use { input ->
        val buf = ByteArray(SoundSettings.MAX_EQ_CHARS + 1)
        var n = 0
        while (n < buf.size) {
            val r = input.read(buf, n, buf.size - n)
            if (r < 0) break
            n += r
        }
        buf.copyOf(n)
    }
    // Over the limit: pass it on, and the size check refuses it with a reason.
    bytes?.let { name to it.toString(Charsets.UTF_8) }
} catch (_: Exception) {
    null
}

/** Paste a profile in place (a text field under the buttons). */
@Composable
internal fun PasteBox(onCancel: () -> Unit, onImport: (String) -> Unit) {
    var text by rememberSaveable { mutableStateOf("") }
    Column(Modifier.padding(horizontal = 16.dp, vertical = 4.dp)) {
        Text(
            stringResource(R.string.sound_eq_paste_title),
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.semantics { heading() },
        )
        OutlinedTextField(
            value = text,
            onValueChange = { text = it.take(SoundSettings.MAX_EQ_CHARS + 1) },
            label = { Text(stringResource(R.string.sound_eq_paste_label)) },
            placeholder = { Text(stringResource(R.string.sound_eq_paste_hint)) },
            minLines = 4,
            maxLines = 10,
            modifier = Modifier.fillMaxWidth().testTag("eq-paste"),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            TextButton(onClick = onCancel) { Text(stringResource(R.string.action_cancel)) }
            TextButton(onClick = { onImport(text) }, enabled = text.isNotBlank()) {
                Text(stringResource(R.string.sound_eq_import))
            }
        }
    }
}
