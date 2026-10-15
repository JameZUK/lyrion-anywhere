package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.Smartphone
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.MyDevice
import io.github.jamezuk.lyrionanywhere.data.DevicesMessage
import io.github.jamezuk.lyrionanywhere.data.LastUsed
import io.github.jamezuk.lyrionanywhere.data.lastUsed
import io.github.jamezuk.lyrionanywhere.data.orderDevices
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage

/**
 * Settings › Account › Your devices: the devices signed in to this account, this phone first
 * and marked, each other one with Sign out (after a confirm), then Sign out all other devices.
 * One message at a time, announced.
 */
fun LazyListScope.yourDevicesItems(
    devices: List<MyDevice>?,
    busy: Boolean,
    message: DevicesMessage?,
    nowSeconds: () -> Long,
    onSignOut: (MyDevice) -> Unit,
    onSignOutOthers: () -> Unit,
) {
    item {
        Text(
            stringResource(R.string.your_devices_explained),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
        )
    }
    if (devices == null) {
        if (message == null) {
            item { Text(stringResource(R.string.your_devices_loading), Modifier.padding(16.dp).testTag("devices-loading")) }
        }
    } else {
        val ordered = orderDevices(devices)
        items(ordered, key = { it.id }) { d -> DeviceRow(d, busy, nowSeconds(), onSignOut) }
        item {
            if (ordered.any { !it.current }) {
                OutlinedButton(
                    onClick = onSignOutOthers,
                    enabled = !busy,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("sign-out-others"),
                ) { Text(stringResource(R.string.your_devices_sign_out_others)) }
            } else {
                Text(
                    stringResource(R.string.your_devices_none_other),
                    Modifier.padding(horizontal = 16.dp, vertical = 8.dp).testTag("no-other-devices"),
                )
            }
        }
    }
    if (message != null) {
        item {
            val failed = message is DevicesMessage.Failed
            Text(
                devicesMessageText(message),
                color = if (failed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface,
                modifier = Modifier
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .testTag("devices-message")
                    .semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
    }
}

@Composable
private fun DeviceRow(d: MyDevice, busy: Boolean, nowSeconds: Long, onSignOut: (MyDevice) -> Unit) {
    val signOutLabel = stringResource(R.string.your_devices_sign_out_named, d.name)
    ListItem(
        leadingContent = {
            Icon(if (d.kind == "android") Icons.Outlined.Smartphone else Icons.Outlined.Computer, contentDescription = null)
        },
        headlineContent = { Text(d.name) },
        overlineContent = if (d.current) {
            { Text(stringResource(R.string.your_devices_this), modifier = Modifier.testTag("this-device")) }
        } else {
            null
        },
        supportingContent = { Text(deviceLine(d, nowSeconds)) },
        trailingContent = if (d.current) {
            null
        } else {
            {
                TextButton(
                    onClick = { onSignOut(d) },
                    enabled = !busy,
                    modifier = Modifier.testTag("sign-out-device-${d.id}").semantics { contentDescription = signOutLabel },
                ) { Text(stringResource(R.string.action_sign_out)) }
            }
        },
        modifier = Modifier.testTag("device-${d.id}"),
    )
}

/** "Pixel 8 · Last used 3 hours ago". */
@Composable
fun deviceLine(d: MyDevice, nowSeconds: Long): String {
    val what = d.platform ?: when (d.kind) {
        "android" -> stringResource(R.string.device_kind_android)
        "pwa" -> stringResource(R.string.device_kind_pwa)
        "browser" -> stringResource(R.string.device_kind_browser)
        else -> null
    }
    val used = when (val u = lastUsed(d, nowSeconds)) {
        LastUsed.Now -> stringResource(R.string.last_used_now)
        LastUsed.Never -> stringResource(R.string.last_used_never)
        LastUsed.JustNow -> stringResource(R.string.last_used_just_now)
        is LastUsed.Minutes -> pluralStringResource(R.plurals.last_used_minutes, u.n, u.n)
        is LastUsed.Hours -> pluralStringResource(R.plurals.last_used_hours, u.n, u.n)
        is LastUsed.Days -> pluralStringResource(R.plurals.last_used_days, u.n, u.n)
        is LastUsed.Months -> pluralStringResource(R.plurals.last_used_months, u.n, u.n)
    }
    return if (what != null) stringResource(R.string.your_devices_line, what, used) else used
}

@Composable
private fun devicesMessageText(m: DevicesMessage): String = when (m) {
    is DevicesMessage.SignedOut -> stringResource(R.string.your_devices_signed_out, m.name)
    is DevicesMessage.SignedOutOthers -> pluralStringResource(R.plurals.password_signed_out_others, m.count, m.count)
    DevicesMessage.AlreadyGone -> stringResource(R.string.your_devices_already_gone)
    is DevicesMessage.Failed -> errorMessage(m.error)
}

/** Ask before signing [device] out, or (null) all [others] other devices. */
@Composable
fun SignOutDeviceDialog(device: MyDevice?, others: Int, onConfirm: () -> Unit, onDismiss: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = Modifier.testTag("sign-out-device-dialog"),
        title = {
            Text(
                if (device != null) {
                    stringResource(R.string.your_devices_confirm_title, device.name)
                } else {
                    stringResource(R.string.your_devices_confirm_others_title)
                },
            )
        },
        text = {
            Text(
                if (device != null) {
                    stringResource(R.string.your_devices_confirm_body)
                } else {
                    pluralStringResource(R.plurals.your_devices_confirm_others_body, others, others)
                },
                modifier = Modifier.verticalScroll(rememberScrollState()),
            )
        },
        confirmButton = {
            TextButton(onClick = onConfirm, modifier = Modifier.testTag("sign-out-device-confirm")) {
                Text(stringResource(if (device != null) R.string.action_sign_out else R.string.your_devices_sign_out_all))
            }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}
