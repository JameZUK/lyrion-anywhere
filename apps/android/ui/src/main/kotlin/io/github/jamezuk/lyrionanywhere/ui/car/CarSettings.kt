package io.github.jamezuk.lyrionanywhere.ui.car

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DirectionsCar
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
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleStartEffect
import io.github.jamezuk.lyrionanywhere.data.CarAutoOpen
import io.github.jamezuk.lyrionanywhere.data.CarModeSetting
import io.github.jamezuk.lyrionanywhere.ui.R

/** Android's side of the car-mode setting: the permission and the paired devices. */
interface CarBluetoothAccess {
    fun granted(): Boolean

    /** Ask for the permission; [onResult] gets the answer. */
    fun request(onResult: (Boolean) -> Unit)

    fun bonded(): List<CarDevice>
}

/** The real thing: BLUETOOTH_CONNECT asked through Android's prompt (Android 12+). */
@Composable
fun rememberCarBluetoothAccess(): CarBluetoothAccess {
    val context = LocalContext.current
    var pending by remember { mutableStateOf<((Boolean) -> Unit)?>(null) }
    val launcher = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        pending?.invoke(granted)
        pending = null
    }
    return remember(context, launcher) {
        object : CarBluetoothAccess {
            override fun granted() = CarBluetooth.granted(context)

            override fun request(onResult: (Boolean) -> Unit) {
                if (!CarBluetooth.permissionNeeded() || granted()) return onResult(true)
                pending = onResult
                try {
                    launcher.launch(CarBluetooth.PERMISSION)
                } catch (_: Exception) {
                    pending = null
                    onResult(false)
                }
            }

            override fun bonded() = CarBluetooth.bonded(context)
        }
    }
}

/**
 * Settings: how car mode opens by itself (Off, Any car audio, This device). The permission
 * is asked for only when an automatic option is first chosen; refused, the setting stays
 * Off with a note. "This device" is chosen from the paired devices; its name and address
 * are kept on this phone only.
 */
@Composable
fun CarModeSettingView(setting: CarModeSetting, onChange: (CarModeSetting) -> Unit, access: CarBluetoothAccess) {
    var refused by rememberSaveable { mutableStateOf(false) }
    var picking by rememberSaveable { mutableStateOf(false) }
    val change by rememberUpdatedState(onChange)
    fun step(option: CarAutoOpen, s: CarSettingFlow.Step) {
        when (s) {
            CarSettingFlow.Step.APPLY -> {
                refused = false
                change(CarModeSetting(option).sanitised())
            }
            CarSettingFlow.Step.PICK -> {
                refused = false
                picking = true
            }
            CarSettingFlow.Step.REFUSED -> {
                refused = true
                change(CarModeSetting())
            }
            CarSettingFlow.Step.ASK -> access.request { granted -> step(option, CarSettingFlow.answered(option, granted)) }
        }
    }
    val label = stringResource(R.string.car_auto_title)
    Column {
        ListItem(
            leadingContent = { Icon(Icons.Outlined.DirectionsCar, contentDescription = null) },
            headlineContent = { Text(label) },
            supportingContent = { Text(stringResource(R.string.car_auto_detail)) },
        )
        Column(Modifier.selectableGroup().semantics { contentDescription = label }) {
            val options = listOf(
                Triple(CarAutoOpen.OFF, R.string.car_auto_off, null),
                Triple(CarAutoOpen.ANY_CAR, R.string.car_auto_any, null),
                Triple(
                    CarAutoOpen.THIS_DEVICE,
                    R.string.car_auto_device,
                    setting.deviceName.takeIf { setting.auto == CarAutoOpen.THIS_DEVICE && it.isNotEmpty() },
                ),
            )
            options.forEach { (option, title, detail) ->
                val selected = setting.auto == option
                ListItem(
                    leadingContent = { RadioButton(selected = selected, onClick = null) },
                    headlineContent = { Text(stringResource(title)) },
                    supportingContent = detail?.let { { Text(it) } },
                    modifier = Modifier
                        .selectable(selected = selected, role = Role.RadioButton) {
                            // Choosing This device again picks another device.
                            if (!selected || option == CarAutoOpen.THIS_DEVICE) step(option, CarSettingFlow.choose(option, access.granted()))
                        }
                        .testTag("car-auto-${option.name}"),
                )
            }
        }
        if (refused) {
            Text(
                stringResource(R.string.car_auto_refused),
                color = MaterialTheme.colorScheme.error,
                style = MaterialTheme.typography.bodyMedium,
                modifier = Modifier
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .semantics { liveRegion = LiveRegionMode.Polite }
                    .testTag("car-auto-refused"),
            )
        }
    }
    if (picking) {
        val devices = remember { access.bonded() }
        CarDevicePicker(
            devices,
            chosen = setting.deviceAddress,
            onPick = {
                picking = false
                change(CarSettingFlow.picked(it))
            },
            onDismiss = { picking = false },
        )
    }
}

/** The paired devices, to choose the car (a plain list in a dialog). */
@Composable
private fun CarDevicePicker(devices: List<CarDevice>, chosen: String, onPick: (CarDevice) -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(stringResource(R.string.car_pick_title)) },
        text = {
            if (devices.isEmpty()) {
                Text(stringResource(R.string.car_pick_none))
            } else {
                LazyColumn(Modifier.selectableGroup().testTag("car-pick-list")) {
                    items(devices, key = { it.address }) { d ->
                        val selected = d.address == chosen
                        ListItem(
                            leadingContent = { RadioButton(selected = selected, onClick = null) },
                            headlineContent = { Text(d.name.ifEmpty { stringResource(R.string.car_unnamed_device) }) },
                            supportingContent = if (d.carAudio) ({ Text(stringResource(R.string.car_audio_device)) }) else null,
                            modifier = Modifier
                                .heightIn(min = 48.dp)
                                .selectable(selected = selected, role = Role.RadioButton) { onPick(d) }
                                .testTag("car-pick-${d.address}"),
                        )
                    }
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}

/**
 * Car mode opening and leaving by itself, while the app is in front (Wave 2 §5.1): listens
 * for Bluetooth connections, and reads what is connected each time the app comes to the
 * front. [onAction] opens or leaves car mode. Without the permission (revoked in Android's
 * settings) nothing is read, and [onPermissionLost] puts the setting back to Off.
 */
@Composable
fun CarAutoOpenEffect(
    setting: CarModeSetting,
    opener: CarAutoOpener,
    onAction: (CarAction) -> Unit,
    onPermissionLost: () -> Unit,
) {
    val context = LocalContext.current
    val latest by rememberUpdatedState(setting)
    val act by rememberUpdatedState(onAction)
    val lost by rememberUpdatedState(onPermissionLost)
    LifecycleStartEffect(setting.auto, setting.deviceAddress) {
        var active = true
        var watcher: CarBluetoothWatcher? = null
        if (setting.auto != CarAutoOpen.OFF) {
            if (!CarBluetooth.granted(context)) {
                lost()
            } else {
                watcher = CarBluetoothWatcher(context) { e ->
                    if (active) {
                        act(
                            when (e) {
                                is CarBtEvent.Connected -> opener.connected(e.device, latest, foreground = true)
                                is CarBtEvent.Disconnected -> opener.disconnected(e.address)
                            },
                        )
                    }
                }.also { it.start() }
                CarBluetooth.connectedNow(context) { devices -> if (active) act(opener.snapshot(devices, latest, foreground = true)) }
            }
        }
        onStopOrDispose {
            active = false
            watcher?.stop()
        }
    }
}
