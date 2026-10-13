package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration
import io.github.jamezuk.lyrionanywhere.ui.library.CreditsSection

/** Now playing's pages (the hero's pager): their order, and moving through them. */
object NowPlayingLayouts {
    /** The page after (or before) [l]; it stops at either end, as a pager does. */
    fun next(l: NowPlayingLayout, forward: Boolean, choices: List<NowPlayingLayout> = NowPlayingLayout.entries): NowPlayingLayout {
        val i = choices.indexOf(l).coerceAtLeast(0)
        return choices[(i + if (forward) 1 else -1).coerceIn(0, choices.size - 1)]
    }

    /** Every page, in the pager's order: Cover, Lyrics, Track info. */
    val PAGES = NowPlayingLayout.entries.toList()

    /**
     * Wide windows show the cover on the left at all times, so the right pane offers Lyrics
     * and Track info only; Cover there means Track info.
     */
    fun widePane(l: NowPlayingLayout): NowPlayingLayout = if (l == NowPlayingLayout.LYRICS) NowPlayingLayout.LYRICS else NowPlayingLayout.INFO

    /** The wide window's right pane's pages. */
    val WIDE = listOf(NowPlayingLayout.LYRICS, NowPlayingLayout.INFO)
}

/**
 * A text page of the hero (Lyrics, Track info): one node TalkBack can reach, named after the
 * page and carrying the way to the others; its content scrolls inside it.
 */
@Composable
internal fun TextPage(page: NowPlayingLayout, actions: List<CustomAccessibilityAction>, content: @Composable () -> Unit) {
    val label = layoutName(page)
    Box(
        Modifier
            .fillMaxSize()
            .padding(horizontal = 8.dp)
            .testTag("page-panel-${page.name}")
            .semantics {
                contentDescription = label
                customActions = actions
            },
    ) { content() }
}

/** The Lyrics page: the lyrics large, following the song; a note when there are none. */
@Composable
internal fun LyricsPanel(lyrics: TrackLyrics?, line: Int, canSeek: Boolean, onSeek: (Double) -> Unit, modifier: Modifier = Modifier) {
    if (lyrics == null || lyrics.lines.isEmpty()) {
        Box(modifier.fillMaxSize().padding(24.dp), contentAlignment = Alignment.Center) {
            Text(
                stringResource(R.string.lyrics_none),
                style = MaterialTheme.typography.bodyLarge,
                textAlign = TextAlign.Center,
                modifier = Modifier.testTag("lyrics-none"),
            )
        }
        return
    }
    LyricsList(lyrics, line, canSeek, onSeek, modifier.fillMaxSize(), bottomPadding = 48.dp)
}

/**
 * The Track info page: the track's details (album and year, artist, genre, format, length)
 * and its credits, each a link to that person's albums in that role. It scrolls inside the
 * page; the signal path opens from the quality badge or ⋮.
 */
@Composable
internal fun InfoPanel(state: PlayerUiState, status: PlayerStatus, actions: NowPlayingActions, modifier: Modifier = Modifier) {
    val track = status.current
    Column(
        modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(vertical = 8.dp).testTag("info-panel"),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (track != null) {
            val rows = buildList {
                track.album?.let { a -> add(R.string.info_album to listOfNotNull(a, track.year?.takeIf { it > 0 }?.toString()).joinToString(" · ")) }
                track.artist?.let { add(R.string.info_artist to it) }
                track.genre?.takeIf { it.isNotBlank() }?.let { add(R.string.info_genre to it) }
                (track.quality?.label ?: track.format)?.takeIf { it.isNotBlank() }?.let { add(R.string.info_format to it) }
                val length = status.durationSecs ?: track.durationSecs
                if (length != null && length > 0) add(R.string.info_length to formatDuration(length))
            }
            rows.forEach { (label, value) -> InfoRow(stringResource(label), value) }
        }
        val credits = state.credits?.takeIf { track?.id != null && state.creditsTrackId == track.id }.orEmpty()
        val onCredit = actions.onCredit
        if (onCredit != null && credits.isNotEmpty()) CreditsSection(credits, onCredit)
    }
}

@Composable
private fun InfoRow(label: String, value: String) {
    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp).semantics(mergeDescendants = true) {}) {
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.width(88.dp),
        )
        Text(value, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
    }
}
