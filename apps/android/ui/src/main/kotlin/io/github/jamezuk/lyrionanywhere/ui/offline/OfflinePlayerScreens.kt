package io.github.jamezuk.lyrionanywhere.ui.offline

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DownloadDone
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Repeat
import androidx.compose.material.icons.outlined.RepeatOne
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material.icons.outlined.SkipNext
import androidx.compose.material.icons.outlined.SkipPrevious
import androidx.compose.material.icons.outlined.Stop
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.art.offlineArtKey
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.library.SectionHeader
import io.github.jamezuk.lyrionanywhere.ui.player.SeekBar
import kotlinx.coroutines.delay

private fun artOf(t: OfflineTrack?): String? =
    t?.let { offlineArtKey(OfflineAlbumViewModel.refOf(it)) }

/** The position now, ticking once a second while playing. */
@Composable
fun rememberOfflinePosition(ui: OfflinePlayerUi, clock: () -> Long): Long {
    var now by remember { mutableLongStateOf(clock()) }
    val playing = ui.state.status?.playback == LocalPlayback.PLAYING
    LaunchedEffect(playing, ui.at) {
        now = clock()
        while (playing) {
            delay(1000)
            now = clock()
        }
    }
    return ui.positionAt(now)
}

/** The bar above the navigation while downloads play on this phone. */
@Composable
fun OfflineMiniPlayer(ui: OfflinePlayerUi, onOpen: () -> Unit, onToggle: () -> Unit, onNext: () -> Unit) {
    val track = ui.state.current ?: return
    Surface(tonalElevation = 3.dp, modifier = Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .testTag("offline-mini-player")
                .clickable(onClickLabel = stringResource(R.string.open_now_playing), onClick = onOpen)
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            ArtworkImage(artOf(track), 120, contentDescription = null, modifier = Modifier.size(48.dp), corner = 6.dp)
            Column(Modifier.weight(1f)) {
                Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyLarge)
                Text(
                    listOf(track.artist, stringResource(R.string.playing_downloads)).filter { it.isNotEmpty() }.joinToString(" · "),
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            val playing = ui.state.playing
            IconButton(onClick = onToggle) {
                Icon(
                    if (playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow,
                    contentDescription = stringResource(if (playing) R.string.action_pause else R.string.action_play),
                )
            }
            IconButton(onClick = onNext) {
                Icon(Icons.Outlined.SkipNext, contentDescription = stringResource(R.string.action_next))
            }
        }
    }
}

/** What the offline Now Playing screen can do. */
data class OfflineNowPlayingActions(
    val onToggle: () -> Unit = {},
    val onNext: () -> Unit = {},
    val onPrevious: () -> Unit = {},
    val onSeek: (Double) -> Unit = {},
    val onShuffle: () -> Unit = {},
    val onRepeat: () -> Unit = {},
    val onJump: (Int) -> Unit = {},
    val onStop: () -> Unit = {},
)

/** Now playing, for downloads: art, title, the seek bar, controls and the queue. */
@Composable
fun OfflineNowPlayingScreen(ui: OfflinePlayerUi, clock: () -> Long, actions: OfflineNowPlayingActions, modifier: Modifier = Modifier) {
    val s = ui.state
    val status = s.status
    val track = s.current
    if (status == null || track == null) {
        EmptyState(stringResource(R.string.offline_nothing_playing), modifier.padding(24.dp))
        return
    }
    val position = rememberOfflinePosition(ui, clock)
    LazyColumn(modifier.fillMaxSize().testTag("offline-now-playing"), contentPadding = PaddingValues(bottom = 24.dp)) {
        item {
            Column(
                Modifier.fillMaxWidth().padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                ArtworkImage(
                    artOf(track),
                    600,
                    contentDescription = stringResource(R.string.album_cover),
                    modifier = Modifier.widthIn(max = 360.dp).fillMaxWidth(0.8f),
                    corner = 12.dp,
                )
                Text(track.title, style = MaterialTheme.typography.headlineSmall, modifier = Modifier.semantics { heading() })
                Text(
                    listOf(track.artist, track.album).filter { it.isNotEmpty() }.joinToString(" · "),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Outlined.DownloadDone, contentDescription = null, modifier = Modifier.size(16.dp))
                    Text(
                        stringResource(R.string.playing_downloads_on_phone),
                        style = MaterialTheme.typography.labelMedium,
                        modifier = Modifier.padding(start = 4.dp),
                    )
                }
                SeekBar(
                    elapsed = position / 1000.0,
                    duration = (status.durationMs.toLong() / 1000.0).takeIf { it > 0 },
                    enabled = true,
                    onSeek = actions.onSeek,
                )
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    IconToggleButton(checked = status.shuffle, onCheckedChange = { actions.onShuffle() }) {
                        Icon(Icons.Outlined.Shuffle, contentDescription = stringResource(if (status.shuffle) R.string.shuffle_on else R.string.shuffle_off))
                    }
                    IconButton(onClick = actions.onPrevious) {
                        Icon(Icons.Outlined.SkipPrevious, contentDescription = stringResource(R.string.action_previous))
                    }
                    FilledIconButton(onClick = actions.onToggle, modifier = Modifier.size(72.dp)) {
                        Icon(
                            if (s.playing) Icons.Outlined.Pause else Icons.Outlined.PlayArrow,
                            contentDescription = stringResource(if (s.playing) R.string.action_pause else R.string.action_play),
                            modifier = Modifier.size(40.dp),
                        )
                    }
                    IconButton(onClick = actions.onNext) {
                        Icon(Icons.Outlined.SkipNext, contentDescription = stringResource(R.string.action_next))
                    }
                    IconToggleButton(checked = status.repeat != LocalRepeat.OFF, onCheckedChange = { actions.onRepeat() }) {
                        Icon(
                            if (status.repeat == LocalRepeat.ONE) Icons.Outlined.RepeatOne else Icons.Outlined.Repeat,
                            contentDescription = stringResource(
                                when (status.repeat) {
                                    LocalRepeat.OFF -> R.string.repeat_off
                                    LocalRepeat.ALL -> R.string.repeat_all
                                    LocalRepeat.ONE -> R.string.repeat_one
                                },
                            ),
                        )
                    }
                }
                IconButton(onClick = actions.onStop) {
                    Icon(Icons.Outlined.Stop, contentDescription = stringResource(R.string.offline_stop))
                }
                status.lastError?.let {
                    Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error)
                }
            }
            HorizontalDivider()
            SectionHeader(stringResource(R.string.section_queue))
        }
        val order = status.order.map { it.toInt() }
        items(order.size) { pos ->
            val listIndex = order[pos]
            val t = status.keys.getOrNull(listIndex)?.let { s.tracks[it] } ?: return@items
            OfflineTrackRow(t, onClick = { actions.onJump(listIndex) }, highlighted = listIndex == status.index?.toInt())
        }
    }
}
