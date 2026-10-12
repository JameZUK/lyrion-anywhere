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
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
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
) {
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Surface(Modifier.fillMaxSize().testTag("lyrics-view")) {
            val current = lyricLineAt(lyrics, elapsedSecs)
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
