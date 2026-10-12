package io.github.jamezuk.lyrionanywhere.ui.settings

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DeleteSweep
import androidx.compose.material.icons.outlined.History
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import io.github.jamezuk.lyrionanywhere.core.HistoryEntry
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.errorMessage
import java.text.DateFormat
import java.util.Date
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.receiveAsFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** One play, with the track's details where the library still has it. */
data class HistoryRow(val entry: HistoryEntry, val track: Track?)

data class HistoryUiState(
    /** Is history recorded? (null: not known yet) */
    val enabled: Boolean? = null,
    val rows: List<HistoryRow> = emptyList(),
    val next: String? = null,
    val loading: Boolean = false,
    val loaded: Boolean = false,
    val error: AppError? = null,
)

sealed interface HistoryMessage {
    data object Cleared : HistoryMessage

    data class Failed(val error: AppError) : HistoryMessage
}

/** The user's listening history (docs/plans/wave1-design.md §8): see, clear, switch off. */
class HistoryViewModel(private val me: MeRepository) : ViewModel() {
    private val _state = MutableStateFlow(HistoryUiState())
    val state: StateFlow<HistoryUiState> = _state.asStateFlow()
    private val _messages = Channel<HistoryMessage>(Channel.BUFFERED)
    val messages: Flow<HistoryMessage> = _messages.receiveAsFlow()

    init {
        load()
    }

    fun load() = page(null)

    fun loadMore() {
        val next = _state.value.next ?: return
        page(next)
    }

    private fun page(before: String?) {
        if (_state.value.loading) return
        _state.update { it.copy(loading = true, error = null) }
        viewModelScope.launch {
            when (val r = me.history(PAGE, before)) {
                is ApiResult.Ok -> {
                    val ids = r.value.items.map { it.trackId }.distinct()
                    val tracks = (me.tracks(ids) as? ApiResult.Ok)?.value.orEmpty().associateBy { it.id }
                    val rows = r.value.items.map { HistoryRow(it, tracks[it.trackId]) }
                    _state.update {
                        it.copy(
                            enabled = r.value.enabled,
                            rows = if (before == null) rows else it.rows + rows,
                            next = r.value.next,
                            loading = false,
                            loaded = true,
                        )
                    }
                }
                is ApiResult.Err -> _state.update { it.copy(loading = false, error = r.error) }
            }
        }
    }

    fun setEnabled(on: Boolean) {
        val before = _state.value.enabled
        _state.update { it.copy(enabled = on) }
        viewModelScope.launch {
            when (val r = me.setHistoryEnabled(on)) {
                is ApiResult.Ok -> _state.update { it.copy(enabled = r.value) }
                is ApiResult.Err -> {
                    _state.update { it.copy(enabled = before) }
                    _messages.trySend(HistoryMessage.Failed(r.error))
                }
            }
        }
    }

    fun clear() {
        viewModelScope.launch {
            when (val r = me.clearHistory()) {
                is ApiResult.Ok -> {
                    _state.update { it.copy(rows = emptyList(), next = null) }
                    _messages.trySend(HistoryMessage.Cleared)
                }
                is ApiResult.Err -> _messages.trySend(HistoryMessage.Failed(r.error))
            }
        }
    }

    companion object {
        const val PAGE = 50
    }
}

/** The Settings rows: the on/off switch, View history and Clear history. */
@Composable
fun HistorySettings(state: HistoryUiState, onEnabled: (Boolean) -> Unit, onView: () -> Unit, onClear: () -> Unit) {
    var confirm by rememberSaveable { mutableStateOf(false) }
    val on = state.enabled ?: true
    Column {
        ListItem(
            headlineContent = { Text(stringResource(R.string.history_record)) },
            supportingContent = { Text(stringResource(R.string.history_explained)) },
            trailingContent = { Switch(checked = on, onCheckedChange = null) },
            modifier = Modifier.clickable(role = Role.Switch, enabled = state.enabled != null) { onEnabled(!on) },
        )
        ListItem(
            leadingContent = { Icon(Icons.Outlined.History, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.history_view)) },
            modifier = Modifier.clickable(onClick = onView),
        )
        ListItem(
            leadingContent = { Icon(Icons.Outlined.DeleteSweep, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.history_clear)) },
            modifier = Modifier.clickable { confirm = true },
        )
    }
    if (confirm) {
        AlertDialog(
            onDismissRequest = { confirm = false },
            title = { Text(stringResource(R.string.history_clear_title)) },
            text = { Text(stringResource(R.string.history_clear_explained)) },
            confirmButton = {
                TextButton(onClick = {
                    confirm = false
                    onClear()
                }) { Text(stringResource(R.string.action_clear)) }
            },
            dismissButton = { TextButton(onClick = { confirm = false }) { Text(stringResource(R.string.action_cancel)) } },
        )
    }
}

/** The history itself, newest first. */
@Composable
fun HistoryScreen(state: HistoryUiState, onMore: () -> Unit, onRetry: () -> Unit, modifier: Modifier = Modifier) {
    when {
        !state.loaded && state.error != null -> ErrorState(state.error, onRetry, modifier)
        !state.loaded -> LoadingState(modifier)
        else -> LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
            if (state.enabled == false) {
                item {
                    Text(
                        stringResource(R.string.history_off),
                        style = MaterialTheme.typography.bodyMedium,
                        modifier = Modifier.padding(16.dp),
                    )
                }
            }
            if (state.rows.isEmpty()) {
                item { EmptyState(stringResource(R.string.history_empty)) }
            }
            items(state.rows, key = { "${it.entry.playedAt}-${it.entry.trackKey}-${it.entry.seconds}" }) { row ->
                val t = row.track
                val at = DateFormat.getDateTimeInstance(DateFormat.MEDIUM, DateFormat.SHORT).format(Date(row.entry.playedAt * 1000))
                ListItem(
                    headlineContent = { Text(t?.title ?: stringResource(R.string.unknown_track)) },
                    supportingContent = {
                        val parts = listOfNotNull(t?.artist, at, if (row.entry.offline) stringResource(R.string.history_offline) else null)
                        Text(parts.joinToString(" · "))
                    },
                )
            }
            if (state.next != null) {
                item {
                    OutlinedButton(onClick = onMore, enabled = !state.loading, modifier = Modifier.padding(16.dp)) {
                        Text(stringResource(R.string.history_more))
                    }
                }
            }
            state.error?.let { e ->
                item { Text(errorMessage(e), Modifier.padding(16.dp)) }
            }
        }
    }
}
