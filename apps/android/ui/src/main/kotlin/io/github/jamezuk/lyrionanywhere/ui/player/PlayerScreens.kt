package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.PowerSettingsNew
import androidx.compose.material.icons.outlined.Repeat
import androidx.compose.material.icons.outlined.RepeatOne
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material.icons.outlined.SkipNext
import androidx.compose.material.icons.outlined.SkipPrevious
import androidx.compose.material.icons.outlined.Speaker
import androidx.compose.material.icons.automirrored.outlined.VolumeDown
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material3.Card
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import io.github.jamezuk.lyrionanywhere.ui.library.TrackRow
import kotlinx.coroutines.delay

/** The current position, ticking once a second while playing. */
@Composable
fun rememberElapsed(state: PlayerUiState, clock: () -> Long): Double {
    var now by remember { mutableLongStateOf(clock()) }
    LaunchedEffect(state.playing, state.statusAt) {
        now = clock()
        while (state.playing) {
            delay(1000)
            now = clock()
        }
    }
    return PlayerViewModel.elapsedAt(state, now)
}

/** The bar shown above the navigation: what's playing, with play/pause and next. */
@Composable
fun MiniPlayer(state: PlayerUiState, onOpen: () -> Unit, onTogglePlay: () -> Unit, onNext: () -> Unit) {
    val track = state.status?.current ?: return
    Surface(tonalElevation = 3.dp, modifier = Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .clickable(onClickLabel = stringResource(R.string.open_now_playing), onClick = onOpen)
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ArtworkImage(track.artwork, 120, contentDescription = null, modifier = Modifier.size(48.dp), corner = 6.dp)
            Column(Modifier.weight(1f)) {
                Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyLarge)
                Text(
                    listOfNotNull(track.artist, state.selected?.name).joinToString(" · "),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            PlayPauseButton(state.playing, onTogglePlay, filled = false)
            IconButton(onClick = onNext) {
                Icon(Icons.Outlined.SkipNext, contentDescription = stringResource(R.string.action_next))
            }
        }
    }
}

@Composable
private fun PlayPauseButton(playing: Boolean, onClick: () -> Unit, filled: Boolean) {
    val icon = if (playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow
    val label = stringResource(if (playing) R.string.action_pause else R.string.action_play)
    if (filled) {
        FilledIconButton(onClick = onClick, modifier = Modifier.size(72.dp)) {
            Icon(icon, contentDescription = label, modifier = Modifier.size(40.dp))
        }
    } else {
        IconButton(onClick = onClick) { Icon(icon, contentDescription = label) }
    }
}

/** The players this account may control, to pick one. */
@Composable
fun PlayerPicker(
    players: List<Player>,
    selectedId: String?,
    onSelect: (String) -> Unit,
) {
    Column {
        players.forEach { p ->
            ListItem(
                leadingContent = { Icon(Icons.Outlined.Speaker, contentDescription = null) },
                headlineContent = { Text(p.name) },
                supportingContent = {
                    Text(
                        stringResource(
                            when {
                                !p.connected -> R.string.player_disconnected
                                p.playing -> R.string.player_playing
                                !p.powered -> R.string.player_off
                                else -> R.string.player_idle
                            },
                        ),
                    )
                },
                trailingContent = { RadioButton(selected = p.id == selectedId, onClick = null) },
                modifier = Modifier.selectable(
                    selected = p.id == selectedId,
                    role = Role.RadioButton,
                    onClick = { onSelect(p.id) },
                ),
            )
        }
    }
}

/** Home: choose a player, and see what it's playing. */
@Composable
fun HomeScreen(
    state: PlayerUiState,
    onSelect: (String) -> Unit,
    onOpenNowPlaying: () -> Unit,
    onRetry: () -> Unit,
    modifier: Modifier = Modifier,
    onPlayOnPhone: (Boolean) -> Unit = {},
) {
    when {
        state.loading -> LoadingState(modifier)
        state.players.isEmpty() && state.error != null -> ErrorState(state.error, onRetry, modifier)
        else -> LazyColumn(modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 16.dp)) {
            val current = state.status?.current
            if (current != null) {
                item {
                    Card(
                        onClick = onOpenNowPlaying,
                        modifier = Modifier.fillMaxWidth().padding(16.dp),
                    ) {
                        Row(
                            Modifier.padding(16.dp),
                            horizontalArrangement = Arrangement.spacedBy(16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            ArtworkImage(current.artwork, 300, contentDescription = null, modifier = Modifier.size(96.dp))
                            Column(Modifier.weight(1f)) {
                                Text(
                                    stringResource(if (state.playing) R.string.now_playing else R.string.paused),
                                    style = MaterialTheme.typography.labelLarge,
                                    color = MaterialTheme.colorScheme.primary,
                                )
                                Text(current.title, style = MaterialTheme.typography.titleMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
                                current.artist?.let { Text(it, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                                current.album?.let {
                                    Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                }
                            }
                        }
                    }
                }
            }
            item { SectionHeader(stringResource(R.string.section_players)) }
            item { PlayOnPhoneRow(state.local, onPlayOnPhone) }
            if (state.players.isEmpty()) {
                item { Text(stringResource(R.string.no_players), Modifier.padding(16.dp)) }
            } else {
                item { PlayerPicker(state.players, state.selectedId, onSelect) }
            }
        }
    }
}

/** Full remote control for the selected player. */
@Composable
fun NowPlayingScreen(
    state: PlayerUiState,
    clock: () -> Long,
    actions: NowPlayingActions,
    modifier: Modifier = Modifier,
) {
    val status = state.status
    val player = state.selected
    if (player == null) {
        EmptyState(stringResource(R.string.choose_player_first), modifier)
        return
    }
    if (status == null) {
        if (state.error != null) ErrorState(state.error, actions.onRetry, modifier) else LoadingState(modifier)
        return
    }
    val track = status.current
    val elapsed = rememberElapsed(state, clock)
    LazyColumn(
        modifier.fillMaxSize(),
        horizontalAlignment = Alignment.CenterHorizontally,
        contentPadding = PaddingValues(16.dp),
    ) {
        item {
            Column(
                Modifier.widthIn(max = 560.dp).fillMaxWidth(),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                ArtworkImage(
                    track?.artwork,
                    800,
                    contentDescription = stringResource(R.string.album_cover),
                    modifier = Modifier.fillMaxWidth(0.85f),
                    corner = 16.dp,
                )
                Text(
                    track?.title ?: stringResource(R.string.nothing_playing),
                    style = MaterialTheme.typography.headlineSmall,
                    textAlign = TextAlign.Center,
                    modifier = Modifier.semantics { heading() },
                )
                Text(
                    listOfNotNull(track?.artist, track?.album).joinToString(" — "),
                    style = MaterialTheme.typography.bodyLarge,
                    textAlign = TextAlign.Center,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                SeekBar(
                    elapsed = elapsed,
                    duration = status.durationSecs,
                    enabled = status.canSeek && track != null,
                    onSeek = actions.onSeek,
                )
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceEvenly,
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    val shuffleLabel = stringResource(R.string.shuffle)
                    val shuffleState = stringResource(
                        when (status.shuffle) {
                            ShuffleMode.OFF -> R.string.mode_off
                            ShuffleMode.SONGS -> R.string.shuffle_songs
                            ShuffleMode.ALBUMS -> R.string.shuffle_albums
                        },
                    )
                    IconToggleButton(
                        checked = status.shuffle != ShuffleMode.OFF,
                        onCheckedChange = { actions.onShuffle() },
                        modifier = Modifier.semantics { stateDescription = shuffleState },
                    ) { Icon(Icons.Outlined.Shuffle, contentDescription = shuffleLabel) }
                    IconButton(onClick = actions.onPrevious) {
                        Icon(Icons.Outlined.SkipPrevious, contentDescription = stringResource(R.string.action_previous))
                    }
                    PlayPauseButton(state.playing, actions.onTogglePlay, filled = true)
                    IconButton(onClick = actions.onNext) {
                        Icon(Icons.Outlined.SkipNext, contentDescription = stringResource(R.string.action_next))
                    }
                    val repeatState = stringResource(
                        when (status.repeat) {
                            RepeatMode.OFF -> R.string.mode_off
                            RepeatMode.ONE -> R.string.repeat_one
                            RepeatMode.ALL -> R.string.repeat_all
                        },
                    )
                    IconToggleButton(
                        checked = status.repeat != RepeatMode.OFF,
                        onCheckedChange = { actions.onRepeat() },
                        modifier = Modifier.semantics { stateDescription = repeatState },
                    ) {
                        Icon(
                            if (status.repeat == RepeatMode.ONE) Icons.Outlined.RepeatOne else Icons.Outlined.Repeat,
                            contentDescription = stringResource(R.string.repeat),
                        )
                    }
                }
                VolumeBar(status.volume.toInt(), actions.onVolume)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(player.name, style = MaterialTheme.typography.labelLarge)
                    Spacer(Modifier.weight(1f))
                    IconToggleButton(checked = status.powered, onCheckedChange = actions.onPower) {
                        Icon(
                            Icons.Outlined.PowerSettingsNew,
                            contentDescription = stringResource(if (status.powered) R.string.power_off else R.string.power_on),
                        )
                    }
                }
                if (state.localSelected) {
                    SignalPathPanel(state.local.signalPath)
                }
            }
        }
        item {
            HorizontalDivider()
            SectionHeader(stringResource(R.string.section_queue))
        }
        if (state.queue.isEmpty()) {
            item { Text(stringResource(R.string.queue_empty), Modifier.padding(16.dp)) }
        }
        itemsIndexed(state.queue, key = { i, t -> "$i-${t.id}" }) { i, t ->
            val index = t.queueIndex?.toInt() ?: i
            TrackRow(
                t,
                onClick = { actions.onJump(index) },
                highlighted = index == status.queueIndex?.toInt(),
                showArt = true,
            )
        }
    }
}

/** Everything the Now Playing screen can do. */
data class NowPlayingActions(
    val onTogglePlay: () -> Unit,
    val onNext: () -> Unit,
    val onPrevious: () -> Unit,
    val onSeek: (Double) -> Unit,
    val onVolume: (Int) -> Unit,
    val onShuffle: () -> Unit,
    val onRepeat: () -> Unit,
    val onPower: (Boolean) -> Unit,
    val onJump: (Int) -> Unit,
    val onRetry: () -> Unit,
)

@Composable
private fun SeekBar(elapsed: Double, duration: Double?, enabled: Boolean, onSeek: (Double) -> Unit) {
    var dragging by remember { mutableStateOf(false) }
    var dragValue by remember { mutableFloatStateOf(0f) }
    val total = duration ?: 0.0
    val shown = if (dragging) dragValue.toDouble() else elapsed
    val label = stringResource(R.string.position)
    Column(Modifier.fillMaxWidth()) {
        Slider(
            value = if (total > 0) shown.toFloat().coerceIn(0f, total.toFloat()) else 0f,
            onValueChange = {
                dragging = true
                dragValue = it
            },
            onValueChangeFinished = {
                dragging = false
                onSeek(dragValue.toDouble())
            },
            valueRange = 0f..(if (total > 0) total.toFloat() else 1f),
            enabled = enabled && total > 0,
            modifier = Modifier.semantics {
                stateDescription = "${formatDuration(shown)} / ${formatDuration(duration)}"
                this.contentDescription = label
            },
        )
        Row {
            Text(formatDuration(shown), style = MaterialTheme.typography.labelMedium)
            Spacer(Modifier.weight(1f))
            Text(formatDuration(duration), style = MaterialTheme.typography.labelMedium)
        }
    }
}

@Composable
private fun VolumeBar(volume: Int, onVolume: (Int) -> Unit) {
    var dragging by remember { mutableStateOf(false) }
    var dragValue by remember { mutableFloatStateOf(0f) }
    val shown = if (dragging) dragValue else volume.toFloat()
    val label = stringResource(R.string.volume)
    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
        Icon(Icons.AutoMirrored.Outlined.VolumeDown, contentDescription = null)
        Slider(
            value = shown,
            onValueChange = {
                dragging = true
                dragValue = it
            },
            onValueChangeFinished = {
                dragging = false
                onVolume(dragValue.toInt())
            },
            valueRange = 0f..100f,
            modifier = Modifier
                .weight(1f)
                .semantics {
                    this.contentDescription = label
                    stateDescription = "${shown.toInt()}%"
                },
        )
        Icon(Icons.AutoMirrored.Outlined.VolumeUp, contentDescription = null)
    }
}
