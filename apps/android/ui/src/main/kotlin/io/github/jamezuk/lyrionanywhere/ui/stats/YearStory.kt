package io.github.jamezuk.lyrionanywhere.ui.stats

import android.content.res.Resources
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.ArrowForward
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.layer.GraphicsLayer
import androidx.compose.ui.graphics.layer.drawLayer
import androidx.compose.ui.graphics.rememberGraphicsLayer
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.launch

/**
 * "Your year in sound" (docs/plans/wave3-design.md §1): full-screen cards, swiped or stepped
 * through with Previous and Next (and TalkBack's actions on each card). Nothing moves on its
 * own. Back closes. "Share" draws the current card to an image on the phone and hands it to
 * [sharer]; nothing is uploaded.
 */
@Composable
fun YearStoryScreen(
    state: StoryUiState,
    onClose: () -> Unit,
    onRetry: () -> Unit,
    sharer: CardSharer,
    modifier: Modifier = Modifier,
    onHistorySettings: () -> Unit = {},
) {
    BackHandler(onBack = onClose)
    Box(
        modifier
            .fillMaxSize()
            .background(MaterialTheme.colorScheme.surface)
            .safeDrawingPadding()
            .testTag("story"),
    ) {
        when {
            state.loading -> {
                val label = stringResource(R.string.loading)
                Column(Modifier.fillMaxSize()) {
                    StoryTopBar(state.year, null, onClose, onShare = null)
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        CircularProgressIndicator(Modifier.semantics { contentDescription = label })
                    }
                }
            }
            state.error != null -> Column(Modifier.fillMaxSize()) {
                StoryTopBar(state.year, null, onClose, onShare = null)
                if (state.unavailable) {
                    StoryNote(stringResource(R.string.stats_unavailable))
                } else {
                    ErrorState(state.error, onRetry)
                }
            }
            !state.enabled -> Column(Modifier.fillMaxSize()) {
                StoryTopBar(state.year, null, onClose, onShare = null)
                StoryNote(stringResource(R.string.stats_off))
                Button(onClick = onHistorySettings, modifier = Modifier.padding(horizontal = 24.dp)) {
                    Text(stringResource(R.string.stats_off_action))
                }
            }
            state.cards.isEmpty() -> Column(Modifier.fillMaxSize()) {
                StoryTopBar(state.year, null, onClose, onShare = null)
                StoryNote(stringResource(R.string.story_empty, state.year))
            }
            else -> Story(state, onClose, sharer)
        }
    }
}

@Composable
private fun StoryNote(text: String) {
    Text(
        text,
        style = MaterialTheme.typography.bodyLarge,
        modifier = Modifier.padding(24.dp).testTag("story-note").semantics { liveRegion = LiveRegionMode.Polite },
    )
}

@Composable
private fun StoryTopBar(year: Int, position: String?, onClose: () -> Unit, onShare: (() -> Unit)?) {
    Row(Modifier.fillMaxWidth().padding(4.dp), verticalAlignment = Alignment.CenterVertically) {
        IconButton(onClick = onClose, modifier = Modifier.testTag("story-close")) {
            Icon(Icons.Outlined.Close, contentDescription = stringResource(R.string.story_close))
        }
        Column(Modifier.weight(1f).padding(horizontal = 8.dp)) {
            Text(
                stringResource(R.string.story_title, year),
                style = MaterialTheme.typography.titleMedium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.semantics { heading() },
            )
            if (position != null) {
                Text(
                    position,
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.testTag("story-position").semantics { liveRegion = LiveRegionMode.Polite },
                )
            }
        }
        if (onShare != null) {
            IconButton(onClick = onShare, modifier = Modifier.testTag("story-share")) {
                Icon(Icons.Outlined.Share, contentDescription = stringResource(R.string.story_share))
            }
        }
    }
}

@Composable
private fun Story(state: StoryUiState, onClose: () -> Unit, sharer: CardSharer) {
    val cards = state.cards
    val pager = rememberPagerState { cards.size }
    val scope = rememberCoroutineScope()
    // Each card on screen records itself into a graphics layer, so the one shown can be
    // drawn to a bitmap for sharing.
    val layers = remember { mutableMapOf<Int, GraphicsLayer>() }
    var shareFailed by remember { mutableStateOf(false) }
    val page = pager.currentPage
    val res = LocalResources.current
    val previousLabel = stringResource(R.string.story_previous)
    val nextLabel = stringResource(R.string.story_next)
    fun go(to: Int) {
        scope.launch { pager.animateScrollToPage(to.coerceIn(0, cards.lastIndex)) }
    }
    Column(Modifier.fillMaxSize()) {
        StoryTopBar(
            state.year,
            stringResource(R.string.story_position, page + 1, cards.size),
            onClose,
            onShare = {
                shareFailed = false
                scope.launch {
                    val layer = layers[pager.currentPage]
                    try {
                        val image = layer?.toImageBitmap()
                        if (image == null || image.width == 0 || image.height == 0) shareFailed = true else sharer.share(image, state.year)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        shareFailed = true
                    }
                }
            },
        )
        HorizontalPager(
            state = pager,
            modifier = Modifier.weight(1f).fillMaxWidth().testTag("story-pager"),
            key = { it },
        ) { index ->
            val layer = rememberGraphicsLayer()
            DisposableEffect(index, layer) {
                layers[index] = layer
                onDispose { if (layers[index] === layer) layers.remove(index) }
            }
            val actions = buildList {
                if (index > 0) add(CustomAccessibilityAction(previousLabel) { go(index - 1); true })
                if (index < cards.lastIndex) add(CustomAccessibilityAction(nextLabel) { go(index + 1); true })
            }
            Box(
                Modifier
                    .fillMaxSize()
                    .padding(horizontal = 16.dp, vertical = 8.dp)
                    .drawWithContent {
                        layer.record { this@drawWithContent.drawContent() }
                        drawLayer(layer)
                    },
                contentAlignment = Alignment.Center,
            ) {
                StoryCardView(
                    cards[index],
                    state.year,
                    res,
                    Modifier
                        .testTag("story-card-$index")
                        .semantics(mergeDescendants = true) { customActions = actions },
                )
            }
        }
        if (shareFailed) {
            Text(
                stringResource(R.string.story_share_failed),
                color = MaterialTheme.colorScheme.error,
                modifier = Modifier.padding(horizontal = 16.dp).testTag("story-share-failed").semantics { liveRegion = LiveRegionMode.Polite },
            )
        }
        Row(
            Modifier.fillMaxWidth().padding(16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            OutlinedButton(onClick = { go(page - 1) }, enabled = page > 0, modifier = Modifier.weight(1f).testTag("story-previous")) {
                Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(previousLabel, Modifier.padding(start = 8.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            Button(onClick = { go(page + 1) }, enabled = page < cards.lastIndex, modifier = Modifier.weight(1f).testTag("story-next")) {
                Text(nextLabel, Modifier.padding(end = 8.dp), maxLines = 1, overflow = TextOverflow.Ellipsis)
                Icon(Icons.AutoMirrored.Outlined.ArrowForward, contentDescription = null, modifier = Modifier.size(18.dp))
            }
        }
    }
}

/** One card, drawn as it is shared: its own background, and the app's name and the year at the foot. */
@Composable
private fun StoryCardView(card: StoryCard, year: Int, res: Resources, modifier: Modifier = Modifier) {
    val on = MaterialTheme.colorScheme.onPrimaryContainer
    Column(
        modifier
            .fillMaxSize()
            .clip(RoundedCornerShape(28.dp))
            .background(MaterialTheme.colorScheme.primaryContainer)
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Spacer(Modifier.weight(1f))
        when (card) {
            is StoryCard.Hours -> {
                Title(stringResource(R.string.story_hours_title))
                Big(listenTime(res, card.seconds))
                Body(stringResource(R.string.story_hours_body, playsText(res, card.plays), daysText(res, card.days)))
            }
            is StoryCard.TopArtistCard -> {
                Title(stringResource(R.string.story_top_artist_title))
                Big(artistLabel(card.artist.name))
                Body(stringResource(R.string.stats_plays_and_time, playsText(res, card.artist.plays), listenTime(res, card.artist.seconds)))
            }
            is StoryCard.TopAlbum -> {
                Title(stringResource(R.string.story_top_album_title))
                ArtworkImage(card.album?.artwork, 600, contentDescription = null, modifier = Modifier.widthIn(max = 200.dp).fillMaxWidth(0.6f).padding(vertical = 12.dp), corner = 12.dp)
                Big(card.album?.title ?: stringResource(R.string.stats_unknown_album), small = true)
                card.album?.artist?.let { Body(it) }
                Body(stringResource(R.string.stats_plays_and_time, playsText(res, card.entry.plays), listenTime(res, card.entry.seconds)))
            }
            is StoryCard.TopTrack -> {
                Title(stringResource(R.string.story_top_track_title))
                ArtworkImage(card.track?.artwork, 600, contentDescription = null, modifier = Modifier.widthIn(max = 200.dp).fillMaxWidth(0.6f).padding(vertical = 12.dp), corner = 12.dp)
                Big(card.track?.title ?: stringResource(R.string.stats_unknown_track), small = true)
                card.track?.artist?.let { Body(it) }
                Body(playsText(res, card.entry.plays))
            }
            is StoryCard.TopFive -> {
                Title(stringResource(R.string.story_top_five_title))
                card.artists.forEachIndexed { i, a ->
                    Text(
                        stringResource(R.string.stats_rank, i + 1, artistLabel(a.name)),
                        style = MaterialTheme.typography.titleLarge,
                        color = on,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(vertical = 4.dp),
                    )
                }
            }
            is StoryCard.Discovery -> {
                Title(stringResource(R.string.story_discovery_title))
                Big(artistLabel(card.artist.name))
                Body(stringResource(R.string.story_discovery_body, dayText(card.artist.firstPlayed)))
            }
            is StoryCard.BusiestHour -> {
                Title(stringResource(R.string.story_hour_title))
                Big(stringResource(R.string.story_hour_value, hourText(card.hour), hourText(card.hour + 1)))
                Body(playsText(res, card.plays))
            }
            is StoryCard.LongestStreak -> {
                Title(stringResource(R.string.story_streak_title))
                Big(daysText(res, card.streak.days.toLong()))
                Body(stringResource(R.string.story_streak_body, dayText(card.streak.from), dayText(card.streak.to)))
            }
            is StoryCard.Summary -> {
                Title(stringResource(R.string.story_summary_title, year))
                Big(listenTime(res, card.seconds))
                Body(playsText(res, card.plays))
                Body(stringResource(R.string.story_summary_artists, card.artists.toString(), card.albums.toString()))
                card.topArtist?.let { Body(stringResource(R.string.story_summary_top, artistLabel(it))) }
            }
        }
        Spacer(Modifier.weight(1f))
        Text(
            stringResource(R.string.story_footer, year),
            style = MaterialTheme.typography.labelLarge,
            color = on,
        )
    }
}

@Composable
private fun Title(text: String) = Text(
    text,
    style = MaterialTheme.typography.titleLarge,
    color = MaterialTheme.colorScheme.onPrimaryContainer,
    textAlign = TextAlign.Center,
    modifier = Modifier.padding(bottom = 8.dp),
)

@Composable
private fun Big(text: String, small: Boolean = false) = Text(
    text,
    style = if (small) MaterialTheme.typography.headlineMedium else MaterialTheme.typography.displaySmall,
    color = MaterialTheme.colorScheme.onPrimaryContainer,
    textAlign = TextAlign.Center,
    maxLines = 3,
    overflow = TextOverflow.Ellipsis,
)

@Composable
private fun Body(text: String) = Text(
    text,
    style = MaterialTheme.typography.bodyLarge,
    color = MaterialTheme.colorScheme.onPrimaryContainer,
    textAlign = TextAlign.Center,
    maxLines = 3,
    overflow = TextOverflow.Ellipsis,
    modifier = Modifier.padding(top = 4.dp),
)
