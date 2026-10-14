package io.github.jamezuk.lyrionanywhere.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R

/** One option of a [ChoiceRow]: its value, its name, a line about it, and a test tag. */
class Choice<T>(val value: T, val label: String, val detail: String? = null, val tag: String? = null, val enabled: Boolean = true)

/**
 * A setting with a few options (Wave 3.5 rule 5): one row, its name and the current choice
 * under it; a tap opens a picker of the options, which closes as one is chosen. Used across
 * Settings, Download options and Sound in place of lists laid out inline. [note] is at most
 * one sentence under the options.
 */
@Composable
fun <T> ChoiceRow(
    title: String,
    choices: List<Choice<T>>,
    selected: T,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    icon: ImageVector? = null,
    tag: String = "choice",
    enabled: Boolean = true,
    note: String? = null,
    /** The current value's wording, when it isn't one of [choices] (e.g. a missing player). */
    current: String? = null,
) {
    var open by rememberSaveable { mutableStateOf(false) }
    val chosen = choices.firstOrNull { it.value == selected }
    ListItem(
        leadingContent = icon?.let { i -> { Icon(i, contentDescription = null) } },
        headlineContent = { Text(title) },
        supportingContent = { Text(current ?: chosen?.label.orEmpty()) },
        modifier = modifier
            .clickable(enabled = enabled, onClickLabel = stringResource(R.string.choice_change), role = Role.Button) { open = true }
            .alpha(if (enabled) 1f else DISABLED)
            .testTag(tag),
    )
    if (open) {
        ChoiceDialog(title, choices, selected, note, tag, onDismiss = { open = false }) {
            open = false
            onSelect(it)
        }
    }
}

/** The picker of a [ChoiceRow]: the options as radio buttons, Cancel to leave it as it was. */
@Composable
fun <T> ChoiceDialog(
    title: String,
    choices: List<Choice<T>>,
    selected: T,
    note: String?,
    tag: String,
    onDismiss: () -> Unit,
    onSelect: (T) -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text(title) },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                Column(Modifier.selectableGroup()) {
                    choices.forEachIndexed { i, c ->
                        val isChosen = c.value == selected
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(selected = isChosen, enabled = c.enabled, role = Role.RadioButton) { onSelect(c.value) }
                                .padding(vertical = 8.dp)
                                .testTag(c.tag ?: "$tag-option-$i"),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = isChosen, onClick = null, enabled = c.enabled)
                            Column(Modifier.padding(start = 12.dp)) {
                                Text(c.label)
                                c.detail?.let {
                                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                                }
                            }
                        }
                    }
                }
                note?.let {
                    Text(
                        it,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 8.dp),
                    )
                }
            }
        },
        confirmButton = {},
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
    )
}

/** How faint a row that can't be used now is drawn. */
private const val DISABLED = 0.38f

/** A settings row that opens a page of its own, with a one-line summary of what's in it. */
@Composable
fun NavRow(title: String, summary: String?, onClick: () -> Unit, modifier: Modifier = Modifier, icon: ImageVector? = null, tag: String? = null) {
    ListItem(
        leadingContent = icon?.let { i -> { Icon(i, contentDescription = null) } },
        headlineContent = { Text(title) },
        supportingContent = summary?.let { { Text(it, maxLines = 1, overflow = androidx.compose.ui.text.style.TextOverflow.Ellipsis) } },
        trailingContent = { Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, contentDescription = null) },
        modifier = modifier
            .clickable(role = Role.Button, onClick = onClick)
            .then(if (tag != null) Modifier.testTag(tag) else Modifier),
    )
}
