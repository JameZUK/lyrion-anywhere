package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.AlertDialogDefaults
import androidx.compose.material3.BasicAlertDialog
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Surface
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.PlaylistNameProblem
import io.github.jamezuk.lyrionanywhere.data.PlaylistNames
import io.github.jamezuk.lyrionanywhere.ui.R
import androidx.compose.ui.platform.LocalResources
import io.github.jamezuk.lyrionanywhere.ui.components.reasonText
import java.time.LocalDate
import java.time.format.DateTimeFormatter
import java.util.Locale

/** "Save as playlist", while open: the name to start from and how the save went. */
data class SaveQueueDialog(
    val suggestion: String,
    val busy: Boolean = false,
    /** The name tried is taken: ask before replacing that playlist. */
    val exists: ExistingPlaylist? = null,
    val error: SaveQueueError? = null,
)

/** A saved playlist called (ignoring case) what was [typed]. */
data class ExistingPlaylist(val id: Long, val name: String, val typed: String)

sealed interface SaveQueueError {
    /** The queue is empty: nothing to save. */
    data object Empty : SaveQueueError

    data class Failed(val error: AppError) : SaveQueueError
}

/** Today, as a suggested playlist name writes it ("14 October 2026"). */
fun saveQueueToday(): String = LocalDate.now().format(DateTimeFormatter.ofPattern("d MMMM yyyy", Locale.UK))

/** What to say about a name that can't be used. */
@Composable
private fun problemText(p: PlaylistNameProblem): String = stringResource(
    when (p) {
        PlaylistNameProblem.EMPTY -> R.string.save_queue_name_empty
        PlaylistNameProblem.TOO_LONG -> R.string.save_queue_name_long
        PlaylistNameProblem.HIDDEN_CHARACTERS -> R.string.save_queue_name_hidden
        PlaylistNameProblem.RESERVED_CHARACTERS -> R.string.save_queue_name_reserved_chars
        PlaylistNameProblem.DOTS -> R.string.save_queue_name_dots
        PlaylistNameProblem.LEADING_DOT -> R.string.save_queue_name_leading_dot
        PlaylistNameProblem.TRAILING_DOT -> R.string.save_queue_name_trailing_dot
        PlaylistNameProblem.RESERVED_NAME -> R.string.save_queue_name_reserved
    },
)

@Composable
private fun errorText(e: SaveQueueError): String = when (e) {
    SaveQueueError.Empty -> stringResource(R.string.save_queue_empty)
    is SaveQueueError.Failed -> when (val a = e.error) {
        is AppError.Forbidden -> stringResource(R.string.save_queue_failed_detail, a.detail)
        is AppError.InvalidInput -> stringResource(R.string.save_queue_failed_detail, reasonText(LocalResources.current, a.reason) ?: a.detail)
        is AppError.Server -> stringResource(R.string.save_queue_failed_detail, a.detail)
        is AppError.ServerBusy -> io.github.jamezuk.lyrionanywhere.ui.components.errorMessage(a)
        else -> stringResource(R.string.save_queue_failed)
    }
}

/**
 * Save the queue as a playlist, in a dialog (see [SaveQueueForm]).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SaveQueueDialogView(
    dialog: SaveQueueDialog,
    onSave: (name: String, replace: Boolean) -> Unit,
    onChangeName: () -> Unit,
    onDismiss: () -> Unit,
) {
    BasicAlertDialog(onDismissRequest = onDismiss, modifier = Modifier.testTag("save-queue-dialog")) {
        Surface(shape = AlertDialogDefaults.shape, tonalElevation = AlertDialogDefaults.TonalElevation) {
            SaveQueueForm(dialog, onSave, onChangeName, onDismiss)
        }
    }
}

/**
 * What "Save as playlist" shows: a name (checked as it's typed, by the gateway's rules),
 * then Save. A playlist of that name is never overwritten silently: it says so and offers
 * Replace.
 */
@Composable
fun SaveQueueForm(
    dialog: SaveQueueDialog,
    onSave: (name: String, replace: Boolean) -> Unit,
    onChangeName: () -> Unit,
    onDismiss: () -> Unit,
) {
    var name by rememberSaveable { mutableStateOf(dialog.suggestion) }
    var touched by rememberSaveable { mutableStateOf(false) }
    val problem = PlaylistNames.problem(PlaylistNames.clean(name))
    val exists = dialog.exists
    Column(Modifier.padding(24.dp).verticalScroll(rememberScrollState())) {
        Text(
            stringResource(if (exists != null) R.string.save_queue_exists_title else R.string.save_queue_title),
            style = MaterialTheme.typography.headlineSmall,
            modifier = Modifier.padding(bottom = 16.dp).semantics { heading() },
        )
        if (exists != null) {
            Text(stringResource(R.string.save_queue_exists, exists.name), Modifier.testTag("save-queue-exists"))
            dialog.error?.let { Text(errorText(it), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("save-queue-error")) }
            Row(Modifier.fillMaxWidth().padding(top = 16.dp), horizontalArrangement = Arrangement.End) {
                // Back to the name, to choose another.
                TextButton(onClick = onChangeName, modifier = Modifier.testTag("save-queue-rename")) {
                    Text(stringResource(R.string.save_queue_change_name))
                }
                TextButton(onClick = { onSave(exists.typed, true) }, enabled = !dialog.busy, modifier = Modifier.testTag("save-queue-replace")) {
                    Text(stringResource(R.string.save_queue_replace))
                }
            }
            return@Column
        }
        val save = {
            touched = true
            if (problem == null && !dialog.busy) onSave(name, false)
        }
        OutlinedTextField(
            value = name,
            onValueChange = {
                name = it
                touched = true
            },
            label = { Text(stringResource(R.string.save_queue_name)) },
            singleLine = true,
            isError = touched && problem != null,
            supportingText = if (touched && problem != null) {
                { Text(problemText(problem), Modifier.testTag("save-queue-problem")) }
            } else {
                null
            },
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences, imeAction = ImeAction.Done),
            keyboardActions = KeyboardActions(onDone = { save() }),
            modifier = Modifier.fillMaxWidth().testTag("save-queue-name"),
        )
        dialog.error?.let { Text(errorText(it), color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("save-queue-error")) }
        Row(Modifier.fillMaxWidth().padding(top = 16.dp), horizontalArrangement = Arrangement.End) {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
            TextButton(onClick = save, enabled = !dialog.busy && !(touched && problem != null), modifier = Modifier.testTag("save-queue-save")) {
                Text(stringResource(if (dialog.busy) R.string.save_queue_saving else R.string.action_save))
            }
        }
    }
}
