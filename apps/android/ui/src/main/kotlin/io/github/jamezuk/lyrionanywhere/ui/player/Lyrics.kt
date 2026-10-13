package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Lyrics
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import kotlinx.coroutines.delay
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.data.LyricsMath
import io.github.jamezuk.lyrionanywhere.ui.R

/** How long the view leaves the scroll position alone after the listener scrolled, ms. */
private const val MANUAL_SCROLL_MS = 4_000L

/** The line playing at [elapsedSecs], or -1 (plain lyrics, before the first line, none). */
internal fun lyricLineAt(lyrics: TrackLyrics?, elapsedSecs: Double): Int =
    LyricsMath.lineAt(lyrics, (elapsedSecs * 1000).toLong())

/**
 * How long until the line playing at [posMs] changes (the next line starts), ms; null when
 * it never will (plain lyrics, none, or the last line is playing).
 */
internal fun msToNextLine(lyrics: TrackLyrics?, posMs: Long): Long? {
    if (lyrics == null || !lyrics.synced) return null
    val next = lyrics.lines.getOrNull(LyricsMath.lineAt(lyrics, posMs) + 1) ?: return null
    return (next.timeMs.toLong() - posMs).coerceAtLeast(0)
}

/** The longest wait between checks of the lyric line, ms (keeps it right after a seek). */
private const val LYRIC_MAX_WAIT_MS = 1_000L

/** A little past a line's start, so the check lands after it rather than just before. */
private const val LYRIC_MARGIN_MS = 15L

/**
 * The lyric line playing now, updated as each line starts: it waits until the next line's
 * start (at most [LYRIC_MAX_WAIT_MS]) rather than ticking once a second, so the highlight
 * moves with the singing.
 */
@Composable
internal fun rememberLyricLine(lyrics: TrackLyrics?, state: PlayerUiState, clock: () -> Long): Int {
    var line by remember(lyrics) { mutableIntStateOf(lyricLineAt(lyrics, PlayerViewModel.elapsedAt(state, clock()))) }
    val latest by rememberUpdatedState(state)
    LaunchedEffect(lyrics, state.playing, state.statusAt, state.status?.elapsedSecs) {
        while (true) {
            val pos = (PlayerViewModel.elapsedAt(latest, clock()) * 1000).toLong()
            line = LyricsMath.lineAt(lyrics, pos)
            if (!latest.playing) break
            val wait = msToNextLine(lyrics, pos) ?: break
            delay((wait + LYRIC_MARGIN_MS).coerceAtMost(LYRIC_MAX_WAIT_MS))
        }
    }
    return line
}

/**
 * The current lyric line, small, on Now playing (docs/plans/wave1-design.md §9). Tapping it
 * opens all the lyrics.
 */
@Composable
internal fun LyricsPeek(line: String, onOpen: () -> Unit) {
    val label = stringResource(R.string.lyrics_peek_label, line)
    Text(
        line,
        style = MaterialTheme.typography.bodyMedium,
        fontStyle = FontStyle.Italic,
        textAlign = TextAlign.Center,
        maxLines = 2,
        overflow = TextOverflow.Ellipsis,
        modifier = Modifier
            .fillMaxWidth()
            .clickable(role = Role.Button, onClick = onOpen)
            .semantics { contentDescription = label }
            .padding(vertical = 4.dp)
            .testTag("lyrics-peek"),
    )
}

/** The button that opens the lyrics (shown when the track has some). */
@Composable
internal fun LyricsButton(onOpen: () -> Unit) {
    IconButton(onClick = onOpen, modifier = Modifier.testTag("lyrics-button")) {
        Icon(Icons.Outlined.Lyrics, contentDescription = stringResource(R.string.lyrics))
    }
}

/**
 * All the lyrics, full screen. Synced lyrics highlight the playing line and keep it in view
 * (unless the listener scrolled in the last few seconds); tapping a line seeks there. Plain
 * lyrics just scroll. Back or Close returns to Now playing.
 */
@Composable
internal fun LyricsScreen(
    lyrics: TrackLyrics,
    title: String,
    elapsedSecs: Double,
    canSeek: Boolean,
    onSeek: (Double) -> Unit,
    onDismiss: () -> Unit,
    now: () -> Long = System::currentTimeMillis,
    /** The playing line, when the caller follows it more finely than [elapsedSecs] ticks. */
    currentLine: Int? = null,
) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize().testTag("lyrics-view")) {
            val current = currentLine ?: lyricLineAt(lyrics, elapsedSecs)
            val list = rememberLazyListState()
            val dragged by list.interactionSource.collectIsDraggedAsState()
            var manualAt by remember { mutableLongStateOf(Long.MIN_VALUE / 2) }
            LaunchedEffect(dragged) { if (dragged) manualAt = now() }
            LaunchedEffect(current) {
                if (current >= 0 && !dragged && now() - manualAt > MANUAL_SCROLL_MS) {
                    // Keep a couple of lines above the playing one.
                    list.animateScrollToItem((current - 2).coerceAtLeast(0))
                }
            }
            Column(Modifier.fillMaxSize()) {
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(start = 16.dp, end = 4.dp, top = 4.dp)) {
                    Text(
                        title.ifEmpty { stringResource(R.string.lyrics) },
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f).semantics { heading() },
                    )
                    IconButton(onClick = onDismiss, modifier = Modifier.testTag("lyrics-close")) {
                        Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.lyrics_close))
                    }
                }
                HorizontalDivider()
                LazyColumn(
                    state = list,
                    modifier = Modifier.fillMaxSize().testTag("lyrics-list"),
                    contentPadding = PaddingValues(start = 20.dp, end = 20.dp, top = 24.dp, bottom = 240.dp),
                    verticalArrangement = Arrangement.spacedBy(if (lyrics.synced) 8.dp else 0.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    itemsIndexed(lyrics.lines) { i, line ->
                        val mod = Modifier.widthIn(max = 640.dp).fillMaxWidth()
                        if (lyrics.synced) {
                            val playing = i == current
                            Text(
                                line.text.ifEmpty { "♪" },
                                style = MaterialTheme.typography.headlineSmall,
                                fontWeight = FontWeight.SemiBold,
                                color = if (playing) MaterialTheme.colorScheme.onSurface else MaterialTheme.colorScheme.onSurfaceVariant,
                                modifier = mod
                                    .clickable(enabled = canSeek, role = Role.Button) { onSeek(line.timeMs.toDouble() / 1000.0) }
                                    .semantics { selected = playing }
                                    .padding(vertical = 4.dp)
                                    .testTag("lyrics-line-$i"),
                            )
                        } else {
                            Text(
                                line.text,
                                style = MaterialTheme.typography.bodyLarge,
                                modifier = mod.testTag("lyrics-line-$i"),
                            )
                        }
                    }
                    item {
                        Text(
                            stringResource(if (lyrics.fromPlugin) R.string.lyrics_from_plugin else R.string.lyrics_from_tags),
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.widthIn(max = 640.dp).fillMaxWidth().padding(top = 24.dp),
                        )
                    }
                }
            }
        }
    }
}

/** Settings: show the current lyric line on Now playing. */
@Composable
fun LyricsPeekSetting(on: Boolean, onChange: (Boolean) -> Unit) {
    ListItem(
        leadingContent = { Icon(Icons.Outlined.Lyrics, contentDescription = null) },
        headlineContent = { Text(stringResource(R.string.settings_lyrics_peek)) },
        supportingContent = { Text(stringResource(R.string.settings_lyrics_peek_detail)) },
        trailingContent = { Switch(checked = on, onCheckedChange = null) },
        modifier = Modifier
            .toggleable(value = on, role = Role.Switch, onValueChange = onChange)
            .testTag("lyrics-peek-setting"),
    )
}
