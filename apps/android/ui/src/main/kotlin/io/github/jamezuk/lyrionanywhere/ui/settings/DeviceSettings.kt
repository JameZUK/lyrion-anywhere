package io.github.jamezuk.lyrionanywhere.ui.settings

import android.os.Build
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Palette
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import io.github.jamezuk.lyrionanywhere.ui.R

/**
 * Settings: the wallpaper's colours (Android 12+, Wave 2 §5.5), on by default. Older Android
 * has none, so the switch is shown off and can't be changed there.
 */
@Composable
fun DynamicColourSetting(on: Boolean, onChange: (Boolean) -> Unit, available: Boolean = Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
    ListItem(
        leadingContent = { Icon(Icons.Outlined.Palette, contentDescription = null) },
        headlineContent = { Text(stringResource(R.string.settings_dynamic_colour)) },
        supportingContent = {
            Text(stringResource(if (available) R.string.settings_dynamic_colour_detail else R.string.settings_dynamic_colour_unavailable))
        },
        trailingContent = { Switch(checked = on && available, onCheckedChange = null, enabled = available) },
        modifier = Modifier
            .toggleable(value = on && available, enabled = available, role = Role.Switch, onValueChange = onChange)
            .testTag("dynamic-colour-setting"),
    )
}
