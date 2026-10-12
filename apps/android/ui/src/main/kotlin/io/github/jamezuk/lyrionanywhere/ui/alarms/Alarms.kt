package io.github.jamezuk.lyrionanywhere.ui.alarms

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TimePicker
import androidx.compose.material3.rememberTimePickerState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.AlarmChange
import io.github.jamezuk.lyrionanywhere.core.AlarmInfo
import io.github.jamezuk.lyrionanywhere.core.AlarmListing
import io.github.jamezuk.lyrionanywhere.data.AlarmsRepository
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

data class AlarmsUiState(
    val loading: Boolean = true,
    val listing: AlarmListing? = null,
    val error: AppError? = null,
    val busy: Boolean = false,
)

/** Alarms on one player (docs/plans/wave2-design.md §1). */
class AlarmsViewModel(private val repo: AlarmsRepository, private val player: String) : ViewModel() {
    private val _state = MutableStateFlow(AlarmsUiState())
    val state: StateFlow<AlarmsUiState> = _state.asStateFlow()

    init {
        run { repo.alarms(player) }
    }

    private fun run(block: suspend () -> ApiResult<AlarmListing>) {
        _state.update { it.copy(busy = true) }
        viewModelScope.launch {
            when (val r = block()) {
                is ApiResult.Ok -> _state.update { AlarmsUiState(loading = false, listing = r.value) }
                is ApiResult.Err -> _state.update { it.copy(loading = false, busy = false, error = r.error) }
            }
        }
    }

    fun retry() = run { repo.alarms(player) }

    fun add(change: AlarmChange) = run { repo.add(player, change.copy(enabled = true)) }

    fun change(id: String, change: AlarmChange) = run { repo.change(player, id, change) }

    fun setEnabled(id: String, on: Boolean) = run { repo.change(player, id, AlarmChange(null, null, on, null, null, null, null)) }

    fun delete(id: String) = run { repo.delete(player, id) }

    fun setAll(on: Boolean) = run { repo.setAll(player, on) }
}

/** "07:00". */
internal fun timeText(secs: UInt): String {
    val s = secs.toInt()
    return "%02d:%02d".format(s / 3600, (s % 3600) / 60)
}

private val DAY_LABELS = listOf("Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat")

@Composable
internal fun daysText(days: ByteArray): String {
    val d = days.map { it.toInt() }.toSortedSet()
    return when (d.joinToString(",")) {
        "0,1,2,3,4,5,6" -> stringResource(R.string.every_day)
        "1,2,3,4,5" -> stringResource(R.string.weekdays)
        "0,6" -> stringResource(R.string.weekends)
        else -> listOf(1, 2, 3, 4, 5, 6, 0).filter { it in d }.joinToString(" ") { DAY_LABELS[it] }
    }
}

/**
 * The Alarms screen: every alarm with its switch, the player's "all alarms" switch, and a
 * dialog to add or change one. Deleting asks first, on the row itself.
 */
@Composable
fun AlarmsScreen(vm: AlarmsViewModel, playerName: String, modifier: Modifier = Modifier) {
    val state by vm.state.collectAsStateWithLifecycle()
    var editing by rememberSaveable { mutableStateOf<String?>(null) }
    var confirmDelete by rememberSaveable { mutableStateOf<String?>(null) }
    val listing = state.listing
    when {
        state.loading -> LoadingState(modifier)
        listing == null -> ErrorState(state.error ?: AppError.Unexpected(""), vm::retry, modifier)
        else -> LazyColumn(modifier.fillMaxSize().testTag("alarms-list")) {
            item {
                Text(
                    stringResource(R.string.alarms_on_player, playerName),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(16.dp),
                )
            }
            item {
                ListItem(
                    headlineContent = { Text(stringResource(R.string.alarms_all)) },
                    trailingContent = { Switch(checked = listing.enabled, onCheckedChange = null) },
                    modifier = Modifier
                        .toggleable(value = listing.enabled, enabled = !state.busy, role = Role.Switch, onValueChange = vm::setAll)
                        .testTag("alarms-all"),
                )
                HorizontalDivider()
            }
            if (listing.alarms.isEmpty()) {
                item { Text(stringResource(R.string.alarms_none), modifier = Modifier.padding(16.dp)) }
            }
            items(listing.alarms, key = { it.id }) { a ->
                val time = timeText(a.timeSecs)
                ListItem(
                    headlineContent = { Text(time, style = MaterialTheme.typography.headlineSmall) },
                    supportingContent = {
                        val once = if (a.repeat) "" else " · " + stringResource(R.string.alarm_once)
                        Text("${daysText(a.days)}$once · ${a.soundTitle}")
                    },
                    trailingContent = {
                        Row(verticalAlignment = Alignment.CenterVertically) {
                            if (confirmDelete == a.id) {
                                TextButton(onClick = { vm.delete(a.id); confirmDelete = null }, modifier = Modifier.testTag("alarm-delete-confirm")) {
                                    Text(stringResource(R.string.alarm_delete_confirm))
                                }
                                TextButton(onClick = { confirmDelete = null }) { Text(stringResource(R.string.alarm_keep)) }
                            } else {
                                val onLabel = stringResource(R.string.alarm_on, time)
                                Switch(
                                    checked = a.enabled,
                                    onCheckedChange = { vm.setEnabled(a.id, it) },
                                    enabled = !state.busy,
                                    modifier = Modifier
                                        .semantics { contentDescription = onLabel }
                                        .testTag("alarm-switch-${a.id}"),
                                )
                                IconButton(onClick = { confirmDelete = a.id }, modifier = Modifier.testTag("alarm-delete-${a.id}")) {
                                    Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.alarm_delete, time))
                                }
                            }
                        }
                    },
                    modifier = Modifier.clickable { editing = a.id }.testTag("alarm-${a.id}"),
                )
                HorizontalDivider()
            }
            item {
                TextButton(onClick = { editing = "new" }, modifier = Modifier.padding(16.dp).testTag("alarm-add")) {
                    Text(stringResource(R.string.alarm_new))
                }
            }
        }
    }
    val e = editing
    if (e != null && listing != null) {
        AlarmEditor(
            alarm = listing.alarms.firstOrNull { it.id == e },
            listing = listing,
            onDismiss = { editing = null },
            onSave = { change ->
                if (e == "new") vm.add(change) else vm.change(e, change)
                editing = null
            },
        )
    }
}

/** Add or change an alarm. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalLayoutApi::class)
@Composable
internal fun AlarmEditor(alarm: AlarmInfo?, listing: AlarmListing, onDismiss: () -> Unit, onSave: (AlarmChange) -> Unit) {
    val start = alarm?.timeSecs?.toInt() ?: (7 * 3600)
    val time = rememberTimePickerState(initialHour = start / 3600, initialMinute = (start % 3600) / 60, is24Hour = true)
    var days by remember { mutableStateOf(alarm?.days?.map { it.toInt() }?.toSet()?.takeIf { it.isNotEmpty() } ?: setOf(1, 2, 3, 4, 5)) }
    var repeat by remember { mutableStateOf(alarm?.repeat ?: true) }
    var volume by remember { mutableFloatStateOf((alarm?.volume?.toInt() ?: 50).toFloat()) }
    var shuffle by remember { mutableIntStateOf(alarm?.shuffle?.toInt() ?: 0) }
    var sound by remember { mutableStateOf(alarm?.soundId ?: "current") }
    var soundOpen by remember { mutableStateOf(false) }
    var pickTime by remember { mutableStateOf(false) }
    if (soundOpen) {
        AlertDialog(
            onDismissRequest = { soundOpen = false },
            title = { Text(stringResource(R.string.alarm_sound)) },
            text = {
                LazyColumn(Modifier.selectableGroup()) {
                    items(listing.sounds, key = { it.id }) { s ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(selected = s.id == sound, role = Role.RadioButton, onClick = { sound = s.id; soundOpen = false })
                                .padding(vertical = 8.dp)
                                .testTag("alarm-sound-${s.id}"),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = s.id == sound, onClick = null)
                            Text(s.title.ifEmpty { s.category }, Modifier.padding(start = 8.dp))
                        }
                    }
                }
            },
            confirmButton = { TextButton(onClick = { soundOpen = false }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
    if (pickTime) {
        AlertDialog(
            onDismissRequest = { pickTime = false },
            title = { Text(stringResource(R.string.alarm_time)) },
            text = { TimePicker(state = time) },
            confirmButton = { TextButton(onClick = { pickTime = false }) { Text(stringResource(R.string.action_done)) } },
        )
    }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(if (alarm == null) R.string.alarm_new else R.string.alarm_change)) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState()).testTag("alarm-editor"), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                // The time on a button; the clock face opens in a dialog of its own.
                TextButton(onClick = { pickTime = true }, modifier = Modifier.testTag("alarm-time")) {
                    Text(
                        "%02d:%02d".format(time.hour, time.minute),
                        style = MaterialTheme.typography.displaySmall,
                    )
                }
                Text(stringResource(R.string.alarm_days), style = MaterialTheme.typography.titleSmall)
                FlowRow(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                    listOf(1, 2, 3, 4, 5, 6, 0).forEach { d ->
                        FilterChip(
                            selected = d in days,
                            onClick = { days = if (d in days) days - d else days + d },
                            label = { Text(DAY_LABELS[d]) },
                            modifier = Modifier.testTag("alarm-day-$d"),
                        )
                    }
                }
                Row(
                    Modifier.fillMaxWidth().toggleable(value = repeat, role = Role.Checkbox, onValueChange = { repeat = it }),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = repeat, onCheckedChange = null)
                    Text(stringResource(R.string.alarm_repeat), Modifier.padding(start = 8.dp))
                }
                ListItem(
                    headlineContent = { Text(stringResource(R.string.alarm_sound)) },
                    supportingContent = { Text(listing.sounds.firstOrNull { it.id == sound }?.title.orEmpty()) },
                    modifier = Modifier.clickable { soundOpen = true }.testTag("alarm-sound"),
                )
                Text(stringResource(R.string.alarm_shuffle), style = MaterialTheme.typography.titleSmall)
                listOf(R.string.alarm_shuffle_off, R.string.alarm_shuffle_songs, R.string.alarm_shuffle_albums).forEachIndexed { i, label ->
                    Row(
                        Modifier.fillMaxWidth().selectable(selected = shuffle == i, role = Role.RadioButton, onClick = { shuffle = i }),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = shuffle == i, onClick = null)
                        Text(stringResource(label), Modifier.padding(start = 8.dp))
                    }
                }
                Text(stringResource(R.string.alarm_volume, volume.toInt()))
                Slider(value = volume, onValueChange = { volume = it }, valueRange = 0f..100f, steps = 19)
            }
        },
        confirmButton = {
            TextButton(
                enabled = days.isNotEmpty(),
                onClick = {
                    onSave(
                        AlarmChange(
                            timeSecs = (time.hour * 3600 + time.minute * 60).toUInt(),
                            days = days.sorted().map { it.toByte() }.toByteArray(),
                            enabled = null,
                            repeat = repeat,
                            volume = volume.toInt().toUByte(),
                            shuffle = shuffle.toUByte(),
                            soundId = sound,
                        ),
                    )
                },
                modifier = Modifier.testTag("alarm-save"),
            ) { Text(stringResource(R.string.action_save)) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}
