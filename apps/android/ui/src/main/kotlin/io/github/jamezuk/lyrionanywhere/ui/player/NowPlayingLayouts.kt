package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Album
import androidx.compose.material.icons.outlined.GraphicEq
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Lyrics
import androidx.compose.material3.Card
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.TrackLyrics
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.library.CreditsSection
import kotlin.math.abs

/** Now playing's layouts (Wave 2 §4.2): the order a swipe moves through them. */
object NowPlayingLayouts {
    /** The layout after (or before) [l]; it stops at either end, as a pager does. */
    fun next(l: NowPlayingLayout, forward: Boolean, choices: List<NowPlayingLayout> = NowPlayingLayout.entries): NowPlayingLayout {
        val i = choices.indexOf(l).coerceAtLeast(0)
        return choices[(i + if (forward) 1 else -1).coerceIn(0, choices.size - 1)]
    }

    /**
     * Wide windows show the art (or visualiser) on the left at all times, so the right
     * pane offers Lyrics and Info only; Art there means Info.
     */
    fun widePane(l: NowPlayingLayout): NowPlayingLayout = if (l == NowPlayingLayout.LYRICS) NowPlayingLayout.LYRICS else NowPlayingLayout.INFO

    val WIDE = listOf(NowPlayingLayout.LYRICS, NowPlayingLayout.INFO)
}

/** A layout's name. */
@Composable
fun layoutName(l: NowPlayingLayout): String = stringResource(
    when (l) {
        NowPlayingLayout.ART -> R.string.layout_art
        NowPlayingLayout.LYRICS -> R.string.layout_lyrics
        NowPlayingLayout.INFO -> R.string.layout_info
    },
)

/**
 * The layout buttons (a segmented control): one press for each layout, the chosen one
 * selected. The alternative to swiping across the art area.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LayoutSelector(
    layout: NowPlayingLayout,
    onLayout: (NowPlayingLayout) -> Unit,
    modifier: Modifier = Modifier,
    choices: List<NowPlayingLayout> = NowPlayingLayout.entries,
) {
    SingleChoiceSegmentedButtonRow(modifier.testTag("layout-buttons")) {
        choices.forEachIndexed { i, l ->
            val name = stringResource(R.string.layout_button, layoutName(l))
            SegmentedButton(
                selected = l == layout,
                onClick = { onLayout(l) },
                shape = SegmentedButtonDefaults.itemShape(i, choices.size),
                icon = {},
                modifier = Modifier.testTag("layout-${l.name}").semantics { contentDescription = name },
            ) {
                Icon(
                    when (l) {
                        NowPlayingLayout.ART -> Icons.Outlined.Album
                        NowPlayingLayout.LYRICS -> Icons.Outlined.Lyrics
                        NowPlayingLayout.INFO -> Icons.Outlined.Info
                    },
                    contentDescription = null,
                    modifier = Modifier.size(20.dp),
                )
            }
        }
    }
}

/** Sideways swipes on a panel move between layouts (the cover has its own, [SwipeableArtwork]). */
internal fun Modifier.layoutSwipes(minPx: Float, onSwipe: (forward: Boolean) -> Unit): Modifier = pointerInput(minPx) {
    var dx = 0f
    detectHorizontalDragGestures(
        onDragStart = { dx = 0f },
        onDragEnd = {
            if (abs(dx) >= maxOf(minPx, size.width * Swipes.ARTWORK_SHARE)) onSwipe(dx < 0)
        },
    ) { change, amount ->
        dx += amount
        change.consume()
    }
}

/** What the art area shows and can do. */
internal class ArtAreaModel(
    val layout: NowPlayingLayout,
    val onLayout: (NowPlayingLayout) -> Unit,
    val artwork: String?,
    /** The visualiser instead of the cover (Art layout, this phone playing, switched on). */
    val visualiser: (@Composable () -> Unit)?,
    val lyrics: (@Composable () -> Unit),
    val info: (@Composable () -> Unit),
    val onNext: () -> Unit,
    val onPrevious: () -> Unit,
    val onPeek: () -> Unit,
    /** Glow's halo around the cover (Art layout, Glow chosen), wrapping it; null for none. */
    val halo: (@Composable (Modifier, @Composable () -> Unit) -> Unit)? = null,
)

/**
 * The art area of Now playing on a phone (Wave 2 §4.2): the cover (or the visualiser), the
 * lyrics, or the track's info, as chosen. A sideways swipe moves between layouts (across the
 * visualiser it changes the visualiser's style instead); the layout buttons and TalkBack
 * actions do the same. The cover keeps Next, Previous and the queue peek as actions, and a
 * swipe up for the peek.
 */
@Composable
internal fun ArtArea(m: ArtAreaModel, modifier: Modifier = Modifier) {
    val min = with(LocalDensity.current) { SWIPE_MIN.toPx() }
    val latest by rememberUpdatedState(m)
    val swipe: (Boolean) -> Unit = { forward -> latest.onLayout(NowPlayingLayouts.next(latest.layout, forward)) }
    val nextLabel = stringResource(R.string.action_next)
    val previousLabel = stringResource(R.string.action_previous)
    val peekLabel = stringResource(R.string.queue_peek)
    val coverLabel = stringResource(R.string.album_cover)
    val showLyrics = stringResource(R.string.layout_show_lyrics)
    val showInfo = stringResource(R.string.layout_show_info)
    val showArt = stringResource(R.string.layout_show_art)
    val layoutActions = NowPlayingLayout.entries.filter { it != m.layout }.map { l ->
        val label = when (l) {
            NowPlayingLayout.ART -> showArt
            NowPlayingLayout.LYRICS -> showLyrics
            NowPlayingLayout.INFO -> showInfo
        }
        CustomAccessibilityAction(label) { latest.onLayout(l); true }
    }
    when (m.layout) {
        NowPlayingLayout.ART -> {
            val vis = m.visualiser
            if (vis != null) {
                // Decorative and hidden from screen readers; its controls are buttons.
                Box(modifier.fillMaxWidth()) { vis() }
            } else {
                val halo = m.halo
                val cover: @Composable (Modifier) -> Unit = { outer ->
                SwipeableArtwork(
                    onNext = { swipe(true) },
                    onPrevious = { swipe(false) },
                    onSwipeUp = m.onPeek,
                    modifier = outer
                        .fillMaxWidth()
                        .testTag("artwork")
                        // One node, labelled as the cover (with or without an image), carrying
                        // Next, Previous, the queue peek and the other layouts.
                        .semantics(mergeDescendants = true) {
                            contentDescription = coverLabel
                            customActions = listOf(
                                CustomAccessibilityAction(nextLabel) { latest.onNext(); true },
                                CustomAccessibilityAction(previousLabel) { latest.onPrevious(); true },
                                CustomAccessibilityAction(peekLabel) { latest.onPeek(); true },
                            ) + layoutActions
                        },
                ) {
                    Crossfade(targetState = m.artwork, animationSpec = tween(350), label = "artwork") { art ->
                        ArtworkImage(art, 800, contentDescription = null, modifier = Modifier.fillMaxWidth(), corner = 16.dp)
                    }
                }
                }
                if (halo != null) halo(modifier) { cover(Modifier) } else cover(modifier)
            }
        }
        NowPlayingLayout.LYRICS, NowPlayingLayout.INFO -> {
            val label = layoutName(m.layout)
            Box(
                modifier
                    .fillMaxWidth()
                    .aspectRatio(1f)
                    .testTag("layout-panel-${m.layout.name}")
                    .layoutSwipes(min, swipe)
                    .semantics {
                        contentDescription = label
                        customActions = layoutActions
                    },
            ) {
                if (m.layout == NowPlayingLayout.LYRICS) m.lyrics() else m.info()
            }
        }
    }
}

/** The Lyrics layout: the lyrics large, following the song; a note when there are none. */
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
 * The Info layout (Wave 2 §4.2): the quality badge and a button for the signal path (both
 * open it as a sheet), the credits (each a link
 * to that person's albums in that role), the rating, and an "Up next" card.
 */
@Composable
internal fun InfoPanel(
    state: PlayerUiState,
    status: PlayerStatus,
    actions: NowPlayingActions,
    onShowPath: () -> Unit,
    onOpenQueue: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val track = status.current
    Column(
        modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(vertical = 8.dp).testTag("info-panel"),
        verticalArrangement = Arrangement.spacedBy(12.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        track?.quality?.let { q -> QualityBadge(q, onClick = onShowPath) }
        // The full signal path (with the EQ profile in use) opens as a sheet, on demand.
        OutlinedButton(onClick = onShowPath, modifier = Modifier.testTag("signal-path-button")) {
            Icon(Icons.Outlined.GraphicEq, contentDescription = null, modifier = Modifier.size(18.dp))
            Text(stringResource(R.string.signal_path_show), Modifier.padding(start = 8.dp))
        }
        if ((track?.id ?: 0) > 0) RatingControl(state.rating, state.ratingStyle, state.ratingFailed, actions.onRate)
        val credits = state.credits?.takeIf { track?.id != null && state.creditsTrackId == track.id }.orEmpty()
        val onCredit = actions.onCredit
        if (onCredit != null && credits.isNotEmpty()) CreditsSection(credits, onCredit)
        Card(Modifier.fillMaxWidth().testTag("up-next-card")) {
            Column(Modifier.padding(vertical = 8.dp)) {
                Text(
                    stringResource(R.string.up_next),
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.padding(horizontal = 16.dp),
                )
                val next = status.next
                if (next != null) {
                    UpNextRow(next, onOpenQueue = onOpenQueue, onRemove = actions.onRemoveUpNext)
                } else {
                    Text(
                        stringResource(R.string.up_next_none),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }
        }
    }
}

/** The row above the art area: the layout buttons, and the visualiser's controls when it can show. */
@Composable
internal fun LayoutRow(
    layout: NowPlayingLayout,
    onLayout: (NowPlayingLayout) -> Unit,
    visualiserControls: (@Composable () -> Unit)?,
    modifier: Modifier = Modifier,
    choices: List<NowPlayingLayout> = NowPlayingLayout.entries,
    /** Room kept at the start for Now playing's close button (over this row). */
    leading: androidx.compose.ui.unit.Dp = 0.dp,
) {
    Row(modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
        if (leading > 0.dp) androidx.compose.foundation.layout.Spacer(Modifier.size(leading))
        LayoutSelector(layout, onLayout, Modifier.weight(1f, fill = false), choices)
        visualiserControls?.invoke()
    }
}
