package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material.icons.outlined.Alarm
import androidx.compose.material.icons.outlined.Bedtime
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalHapticFeedback
import io.github.jamezuk.lyrionanywhere.ui.theme.AmbientBackground
import io.github.jamezuk.lyrionanywhere.ui.theme.isHighContrast
import io.github.jamezuk.lyrionanywhere.ui.theme.rememberArtSeed
import io.github.jamezuk.lyrionanywhere.ui.theme.tinted
import kotlinx.coroutines.launch
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.material3.surfaceColorAtElevation
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.MoreVert
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
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.TextButton
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.data.Waveform
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import androidx.compose.material3.AssistChip
import androidx.compose.material.icons.outlined.SpeakerGroup
import androidx.compose.foundation.layout.width
import androidx.compose.ui.semantics.onClick
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtTinted
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration
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

/**
 * The bar shown above the navigation: what's playing, with play/pause, next and the output
 * button; above it, the "Playing on …" strip when another player is controlled (D20).
 *
 * Gestures (Wave 1 §4, as on the web): swipe left/right for the next/previous track (with a
 * haptic tick), swipe up or tap to open Now playing, long-press for *Play on…*. TalkBack
 * gets each as an action (Next, Previous, Play on…, and the click to open). A small moon
 * shows while a sleep timer runs, and the bar takes a faint wash of the cover's colour.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun MiniPlayer(
    state: PlayerUiState,
    onOpen: () -> Unit,
    onTogglePlay: () -> Unit,
    onNext: () -> Unit,
    onOutputs: () -> Unit = {},
    onBackToPhone: () -> Unit = {},
    onBringToPhone: (() -> Unit)? = null,
    onDismissTransfer: () -> Unit = {},
    onPrevious: () -> Unit = {},
) {
    val track = state.status?.current
    if (track == null) {
        Column {
            TransferStatus(state.transfer, onDismissTransfer)
            PlayingOnStrip(state, onBackToPhone, onBringToPhone = onBringToPhone)
            // No player yet, or nothing queued: "Play on…" must still be one tap away (a
            // first-time user has no other way to pick a player).
            if (!state.loading && !state.waitingForPhone) ChoosePlayerBar(state, onOutputs)
        }
        return
    }
    val density = LocalDensity.current
    val swipeMin = with(density) { SWIPE_MIN.toPx() }
    val swipeUp = with(density) { SWIPE_UP_MIN.toPx() }
    val haptics = LocalHapticFeedback.current
    // Stable callbacks, so recompositions don't restart a swipe in progress.
    val latestOpen by rememberUpdatedState(onOpen)
    val latestNext by rememberUpdatedState(onNext)
    val latestPrevious by rememberUpdatedState(onPrevious)
    val swiped = remember {
        { a: SwipeAction ->
            when (a) {
                SwipeAction.NEXT -> {
                    haptics.performHapticFeedback(HapticFeedbackType.GestureEnd)
                    latestNext()
                }
                SwipeAction.PREVIOUS -> {
                    haptics.performHapticFeedback(HapticFeedbackType.GestureEnd)
                    latestPrevious()
                }
                SwipeAction.OPEN -> latestOpen()
                else -> {}
            }
        }
    }
    var follow by remember { mutableStateOf<Offset?>(null) }
    val onDrag = remember { { o: Offset? -> follow = o } }
    val nextLabel = stringResource(R.string.action_next)
    val previousLabel = stringResource(R.string.action_previous)
    val playOnLabel = stringResource(R.string.mini_play_on)
    val context = LocalContext.current
    val highContrast = remember(context) { isHighContrast(context) }
    val seed = rememberArtSeed(track.artwork).takeUnless { highContrast }
    val sleepLeft = SleepTimer.left(state.status)
    Surface(
        tonalElevation = 3.dp,
        color = tinted(MaterialTheme.colorScheme.surfaceColorAtElevation(3.dp), seed),
        modifier = Modifier.fillMaxWidth(),
    ) {
      Column {
        TransferStatus(state.transfer, onDismissTransfer)
        PlayingOnStrip(state, onBackToPhone, onBringToPhone = onBringToPhone)
        Row(
            Modifier
                .testTag("mini-player")
                .miniPlayerSwipes(swipeMin, swipeUp, swiped, onDrag)
                .combinedClickable(
                    onClickLabel = stringResource(R.string.open_now_playing),
                    onLongClickLabel = playOnLabel,
                    onLongClick = {
                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                        onOutputs()
                    },
                    onClick = onOpen,
                )
                .semantics {
                    customActions = listOf(
                        CustomAccessibilityAction(nextLabel) { onNext(); true },
                        CustomAccessibilityAction(previousLabel) { onPrevious(); true },
                        CustomAccessibilityAction(playOnLabel) { onOutputs(); true },
                    )
                }
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                Modifier.weight(1f).graphicsLayer {
                    translationX = (follow?.x ?: 0f) * 0.5f
                    translationY = (follow?.y ?: 0f) * 0.4f
                },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
            ArtworkImage(track.artwork, 120, contentDescription = null, modifier = Modifier.size(48.dp), corner = 6.dp)
            Column(Modifier.weight(1f)) {
                Text(track.title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.bodyLarge)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    if (sleepLeft != null) {
                        val moon = stringResource(R.string.sleep_timer_left, sleepLeftText(sleepLeft))
                        Icon(
                            Icons.Outlined.Bedtime,
                            contentDescription = moon,
                            tint = MaterialTheme.colorScheme.primary,
                            modifier = Modifier.size(14.dp).padding(end = 2.dp).testTag("mini-moon"),
                        )
                    }
                    Text(
                        listOfNotNull(track.artist, state.selected?.name).joinToString(" · "),
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            }
            OutputButton(state, onOutputs)
            PlayPauseButton(state.playing, onTogglePlay, filled = false)
            IconButton(onClick = onNext) {
                Icon(Icons.Outlined.SkipNext, contentDescription = stringResource(R.string.action_next))
            }
        }
      }
    }
}

/**
 * The mini player's stand-in while nothing plays: says so (or that no player is chosen),
 * with the button that opens "Play on…".
 */
@Composable
internal fun ChoosePlayerBar(state: PlayerUiState, onOutputs: () -> Unit) {
    val name = state.selected?.name
    Surface(tonalElevation = 3.dp, modifier = Modifier.fillMaxWidth().testTag("mini-choose-player")) {
        Row(
            Modifier.padding(start = 16.dp, end = 8.dp, top = 4.dp, bottom = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                if (name == null) stringResource(R.string.mini_no_player) else stringResource(R.string.mini_nothing_playing, name),
                style = MaterialTheme.typography.bodyMedium,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            TextButton(onClick = onOutputs, modifier = Modifier.testTag("choose-player")) {
                Icon(Icons.Outlined.Devices, contentDescription = null, modifier = Modifier.size(18.dp))
                Text(
                    stringResource(if (name == null) R.string.action_choose_player else R.string.play_on_title),
                    Modifier.padding(start = 8.dp),
                )
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

/**
 * Full remote control for the selected player. On a phone the controls and the queue
 * scroll as one column (pull down at the top to close); with [twoPane] the queue sits
 * beside the controls (phones held sideways). With [wide] (windows over 840 dp, Wave 2
 * §4.2) the art sits on the left, the lyrics or info on the right, and the queue in a side
 * panel that can be hidden.
 *
 * Above the art: the layout buttons (Art, Lyrics, Info; a sideways swipe on the art area
 * does the same) and, while this phone is the player, the visualiser's switch and style
 * button. The visualiser (from [visual]) shows where the art is; it is off unless chosen,
 * and stays off in battery saver and with animations removed.
 */
@Composable
fun NowPlayingScreen(
    state: PlayerUiState,
    clock: () -> Long,
    actions: NowPlayingActions,
    modifier: Modifier = Modifier,
    twoPane: Boolean = false,
    wide: Boolean = false,
    visual: VisualSource? = null,
) {
    val status = state.status
    val player = state.selected
    if (player == null) {
        if (state.waitingForPhone) {
            LoadingState(modifier)
        } else {
            Column(
                modifier.fillMaxSize().padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
            ) {
                Text(
                    stringResource(R.string.choose_player_first),
                    style = MaterialTheme.typography.bodyLarge,
                    textAlign = TextAlign.Center,
                )
                Button(onClick = actions.onOutputs, modifier = Modifier.testTag("now-playing-choose-player")) {
                    Icon(Icons.Outlined.Devices, contentDescription = null, modifier = Modifier.size(18.dp))
                    Text(stringResource(R.string.action_choose_player), Modifier.padding(start = 8.dp))
                }
            }
        }
        return
    }
    if (status == null) {
        if (state.error != null) ErrorState(state.error, actions.onRetry, modifier) else LoadingState(modifier)
        return
    }
    val track = status.current
    val drag = remember { QueueDrag() }
    val queueActions = actions.queueActions()
    val currentIndex = status.queueIndex?.toInt()
    var peek by rememberSaveable { mutableStateOf(false) }
    var showPath by rememberSaveable { mutableStateOf(false) }
    val openPeek: () -> Unit = { peek = true }
    val pullMin = with(LocalDensity.current) { SWIPE_MIN.toPx() * 1.5f }
    // Kept across recompositions (the caller's lambda changes each time), so a pull in
    // progress isn't lost.
    val onClose by rememberUpdatedState(actions.onClose)
    val pull = remember(pullMin) { PullToClose(pullMin) { onClose?.invoke() } }
    val togglePlay by rememberUpdatedState(actions.onTogglePlay)
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    // "Up next" opens the queue: on a phone it is further down this list.
    val openQueue: () -> Unit = { if (!twoPane && !wide) scope.launch { list.animateScrollToItem(1) } }

    // This device's choices: shown at once, kept by the caller.
    val prefs = state.nowPlaying
    var layout by remember(prefs.layout) { mutableStateOf(prefs.layout) }
    var visualOn by remember(prefs.visualiser) { mutableStateOf(prefs.visualiser) }
    var style by remember(prefs.visualStyle) { mutableStateOf(prefs.visualStyle) }
    var queuePanel by remember(prefs.queuePanel) { mutableStateOf(prefs.queuePanel) }
    val setLayout: (NowPlayingLayout) -> Unit = {
        layout = it
        actions.onLayout(it)
    }
    val setStyle: (VisualStyle) -> Unit = {
        style = it
        actions.onVisualStyle(it)
    }
    val block = rememberVisualBlock()
    // Only while this phone is the player (a remote player keeps the ambient background).
    val visualShown = state.localSelected && visualOn && block == VisualBlock.NONE && (wide || layout == NowPlayingLayout.ART)
    val visualState = rememberVisualState(visual, running = visualShown)
    val glow = visualShown && style == VisualStyle.GLOW
    val visualiserControls: (@Composable () -> Unit)? = if (state.localSelected) {
        {
            VisualiserControls(
                on = visualOn,
                style = style,
                block = block,
                onToggle = {
                    visualOn = it
                    actions.onVisualiser(it)
                },
                onStyle = setStyle,
            )
        }
    } else {
        null
    }
    val visualiserView: (@Composable () -> Unit)? = if (visualShown && !glow) {
        { VisualiserCanvas(style, visualState, setStyle) }
    } else {
        null
    }
    val infoShown = if (wide) NowPlayingLayouts.widePane(layout) == NowPlayingLayout.INFO else layout == NowPlayingLayout.INFO
    val lyricsShown = if (wide) NowPlayingLayouts.widePane(layout) == NowPlayingLayout.LYRICS else layout == NowPlayingLayout.LYRICS
    LaunchedEffect(infoShown, track?.id) { if (infoShown) actions.onCredits() }
    val lyricsPanel: @Composable () -> Unit = { CurrentLyrics(state, status, clock, actions.onSeek) }
    val infoPanel: @Composable () -> Unit = {
        InfoPanel(state, status, actions, onShowPath = { showPath = true }, onOpenQueue = { if (wide && !queuePanel) peek = true else openQueue() })
    }
    val artArea: @Composable () -> Unit = {
        if (wide) {
            visualiserControls?.let { c -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.End) { c() } }
        } else {
            LayoutRow(layout, setLayout, visualiserControls)
        }
        ArtArea(
            ArtAreaModel(
                layout = if (wide) NowPlayingLayout.ART else layout,
                onLayout = if (wide) ({ l -> setLayout(NowPlayingLayouts.widePane(l)) }) else setLayout,
                artwork = track?.artwork,
                visualiser = visualiserView,
                lyrics = lyricsPanel,
                info = infoPanel,
                onNext = actions.onNext,
                onPrevious = actions.onPrevious,
                onPeek = openPeek,
            ),
            Modifier.fillMaxWidth(0.85f),
        )
    }
    ArtTinted(track?.artwork) {
    Box(modifier.fillMaxSize()) {
    AmbientBackground(track?.artwork, pulse = if (glow) ({ visualState.glow }) else null)
    Column(
        Modifier
            .fillMaxSize()
            // Space or the media key plays and pauses. Handled after the focused control (not
            // in preview), so Space on a focused button still presses that button.
            .onKeyEvent { e ->
                if (isPlayPauseKey(e)) {
                    togglePlay()
                    true
                } else {
                    false
                }
            },
    ) {
    TransferStatus(state.transfer, actions.onDismissTransfer)
    PlayingOnStrip(state, actions.onBackToPhone, onBringToPhone = actions.onBringToPhone)
    if (wide) {
        Row(Modifier.fillMaxSize().testTag("now-playing-wide")) {
            Column(
                Modifier.weight(1f).fillMaxHeight().verticalScroll(rememberScrollState()).padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                NowPlayingControls(state, status, player.name, clock, actions, openQueue, openPeek, { showPath = true }, artArea, lyricsShown)
            }
            VerticalDivider()
            Column(Modifier.weight(1f).fillMaxHeight().padding(16.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    LayoutSelector(
                        NowPlayingLayouts.widePane(layout),
                        setLayout,
                        Modifier.weight(1f, fill = false),
                        NowPlayingLayouts.WIDE,
                    )
                    Spacer(Modifier.weight(1f))
                    val queueState = stringResource(if (queuePanel) R.string.queue_panel_shown else R.string.queue_panel_hidden)
                    IconToggleButton(
                        checked = queuePanel,
                        onCheckedChange = {
                            queuePanel = it
                            actions.onQueuePanel(it)
                        },
                        modifier = Modifier.testTag("queue-panel-toggle").semantics { stateDescription = queueState },
                    ) {
                        Icon(Icons.AutoMirrored.Outlined.QueueMusic, contentDescription = stringResource(R.string.queue_panel))
                    }
                }
                val minSwipe = with(LocalDensity.current) { SWIPE_MIN.toPx() }
                val pane = NowPlayingLayouts.widePane(layout)
                Box(
                    Modifier
                        .weight(1f)
                        .fillMaxWidth()
                        .testTag("wide-pane-${pane.name}")
                        .layoutSwipes(minSwipe) { forward -> setLayout(NowPlayingLayouts.next(pane, forward, NowPlayingLayouts.WIDE)) },
                ) {
                    if (pane == NowPlayingLayout.LYRICS) lyricsPanel() else infoPanel()
                }
            }
            if (queuePanel) {
                VerticalDivider()
                LazyColumn(Modifier.width(360.dp).fillMaxHeight().testTag("queue-pane"), contentPadding = PaddingValues(vertical = 8.dp)) {
                    item(key = "queue-header") {
                        QueueHeader(state.queue.size, player.name, actions.onClearQueue, state.upcoming, actions.onClearUpcoming)
                    }
                    queueItems(state.queue, currentIndex, drag, queueActions, hasMore = state.queueMore, sections = state.sections)
                }
            }
        }
    } else if (twoPane) {
        Row(Modifier.fillMaxSize().testTag("now-playing-two-pane")) {
            Column(
                Modifier.weight(1f).fillMaxHeight().verticalScroll(rememberScrollState()).padding(16.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                NowPlayingControls(state, status, player.name, clock, actions, openQueue, openPeek, { showPath = true }, artArea, lyricsShown)
            }
            VerticalDivider()
            LazyColumn(Modifier.weight(1f).fillMaxHeight().testTag("queue-pane"), contentPadding = PaddingValues(vertical = 8.dp)) {
                item(key = "queue-header") {
                    QueueHeader(state.queue.size, player.name, actions.onClearQueue, state.upcoming, actions.onClearUpcoming)
                }
                queueItems(state.queue, currentIndex, drag, queueActions, hasMore = state.queueMore, sections = state.sections)
            }
        }
    } else {
        LazyColumn(
            Modifier.fillMaxSize().nestedScroll(pull).testTag("now-playing-list"),
            state = list,
            horizontalAlignment = Alignment.CenterHorizontally,
            contentPadding = PaddingValues(16.dp),
        ) {
            item(key = "controls") { NowPlayingControls(state, status, player.name, clock, actions, openQueue, openPeek, { showPath = true }, artArea, lyricsShown) }
            item(key = "queue-header") {
                HorizontalDivider()
                QueueHeader(state.queue.size, player.name, actions.onClearQueue, state.upcoming, actions.onClearUpcoming)
            }
            queueItems(state.queue, currentIndex, drag, queueActions, hasMore = state.queueMore, sections = state.sections)
        }
    }
    }
    }
    }
    if (peek) QueuePeekSheet(state, actions, onDismiss = { peek = false })
    if (showPath) SignalPathSheet(state, onDismiss = { showPath = false })
}

/** The current track's lyrics as Now playing's Lyrics layout, following the song. */
@Composable
private fun CurrentLyrics(
    state: PlayerUiState,
    status: io.github.jamezuk.lyrionanywhere.core.PlayerStatus,
    clock: () -> Long,
    onSeek: (Double) -> Unit,
) {
    val track = status.current
    val lyrics = state.lyrics?.takeIf { track?.id != null && state.lyricsTrackId == track.id }
    val line = rememberLyricLine(lyrics, state, clock)
    LyricsPanel(lyrics, line, status.canSeek, onSeek)
}

/** Artwork, track details, seek bar and buttons. */
@Composable
private fun NowPlayingControls(
    state: PlayerUiState,
    status: io.github.jamezuk.lyrionanywhere.core.PlayerStatus,
    playerName: String,
    clock: () -> Long,
    actions: NowPlayingActions,
    onOpenQueue: () -> Unit = {},
    onPeek: () -> Unit = {},
    onShowPath: () -> Unit = {},
    /** The art area (layout buttons, art, lyrics, info or the visualiser). */
    art: @Composable () -> Unit = {},
    /** The lyrics are showing as a layout (so no lyric line under the title). */
    lyricsShown: Boolean = false,
) {
    val track = status.current
    var showSleep by rememberSaveable { mutableStateOf(false) }
    var showLyrics by rememberSaveable { mutableStateOf(false) }
    // Art and title cross-fade between tracks (Compose follows the system's animation
    // scale, so "Remove animations" turns this into a cut).
    val trackKey = track?.id ?: track?.title
    val elapsed = rememberElapsed(state, clock)
    val lyrics = state.lyrics?.takeIf { track?.id != null && state.lyricsTrackId == track.id }
    // The playing lyric line, updated as each line starts (not on the once-a-second tick).
    val lyricLine = rememberLyricLine(lyrics, state, clock)
    val nextLabel = stringResource(R.string.action_next)
    val previousLabel = stringResource(R.string.action_previous)
    Column(
        Modifier.widthIn(max = 560.dp).fillMaxWidth(),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        art()
        AnimatedContent(
            targetState = trackKey to (track?.title ?: stringResource(R.string.nothing_playing)),
            transitionSpec = { fadeIn(tween(300)) togetherWith fadeOut(tween(200)) },
            label = "title",
        ) { (_, title) ->
            Text(
                title,
                style = MaterialTheme.typography.headlineSmall,
                textAlign = TextAlign.Center,
                modifier = Modifier.fillMaxWidth().semantics { heading() },
            )
        }
        Text(
            listOfNotNull(track?.artist, track?.album).joinToString(" — "),
            style = MaterialTheme.typography.bodyLarge,
            textAlign = TextAlign.Center,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        track?.quality?.let { q -> QualityBadge(q, onClick = onShowPath) }
        if ((track?.id ?: 0) > 0) RatingControl(state.rating, state.ratingStyle, state.ratingFailed, actions.onRate)
        if (state.lyricsPeek && lyrics?.synced == true && !lyricsShown) {
            val line = lyrics.lines.getOrNull(lyricLine)?.text
            if (!line.isNullOrEmpty()) LyricsPeek(line, onOpen = { showLyrics = true })
        }
        SeekBar(
            elapsed = elapsed,
            duration = status.durationSecs,
            enabled = status.canSeek && track != null,
            onSeek = actions.onSeek,
            waveform = state.waveform,
        )
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceEvenly,
            modifier = Modifier.fillMaxWidth(),
        ) {
            ShuffleButton(status.shuffle, onCycle = actions.onShuffle, onChoose = actions.onShuffleMode)
            IconButton(onClick = actions.onPrevious) {
                Icon(Icons.Outlined.SkipPrevious, contentDescription = previousLabel)
            }
            PlayPauseButton(state.playing, actions.onTogglePlay, filled = true)
            IconButton(onClick = actions.onNext) {
                Icon(Icons.Outlined.SkipNext, contentDescription = nextLabel)
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
        status.next?.let { next ->
            UpNextRow(next, onOpenQueue = onOpenQueue, onRemove = actions.onRemoveUpNext)
        }
        // The volume has a row of its own, so it keeps a usable width on a narrow phone; the
        // other actions sit in the row below.
        VolumeBar(status.volume.toInt(), actions.onVolume)
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceEvenly,
            modifier = Modifier.fillMaxWidth().testTag("now-playing-actions"),
        ) {
            IconButton(onClick = onPeek, modifier = Modifier.testTag("queue-peek-button")) {
                Icon(Icons.AutoMirrored.Outlined.QueueMusic, contentDescription = stringResource(R.string.queue_peek))
            }
            if (lyrics != null) LyricsButton(onOpen = { showLyrics = true })
            actions.onAlarms?.let { open ->
                IconButton(onClick = open, modifier = Modifier.testTag("alarms-button")) {
                    Icon(Icons.Outlined.Alarm, contentDescription = stringResource(R.string.alarms))
                }
            }
            SleepButton(SleepTimer.left(status), onOpen = { showSleep = true }, onCancel = actions.onCancelSleep)
            OutputButton(state, actions.onOutputs)
            if (track?.id != null && actions.onMore != null) {
                IconButton(onClick = { actions.onMore.invoke(track) }) {
                    Icon(Icons.Outlined.MoreVert, contentDescription = stringResource(R.string.action_more))
                }
            }
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            PlayerChip(playerName, actions.onRooms ?: actions.onOutputs, rooms = actions.onRooms != null, Modifier.weight(1f))
            // A constant name, with on/off as its state (a toggle's label mustn't flip).
            val powerState = stringResource(if (status.powered) R.string.toggle_on else R.string.mode_off)
            IconToggleButton(
                checked = status.powered,
                onCheckedChange = actions.onPower,
                modifier = Modifier.semantics { stateDescription = powerState },
            ) {
                Icon(
                    Icons.Outlined.PowerSettingsNew,
                    contentDescription = stringResource(R.string.power_named, playerName),
                )
            }
        }
        if (state.localSelected) {
            SignalPathPanel(state.local.signalPath, quality = state.local.streamQuality, metered = state.local.metered)
        }
    }
    if (showSleep) {
        val selected = state.selectedId
        SleepSheet(
            left = SleepTimer.left(status),
            grouped = selected != null && state.syncGroups.any { selected in it.playerIds },
            others = state.players.any { it.id != selected && it.connected },
            canEnd = track?.id != null && ((status.durationSecs ?: track.durationSecs ?: 0.0) > 0.0),
            fadeSecs = state.sleepFade,
            onSet = actions.onSleep,
            onCancel = actions.onCancelSleep,
            onDismiss = { showSleep = false },
        )
    }
    if (showLyrics && lyrics != null) {
        LyricsScreen(
            lyrics = lyrics,
            title = track?.title.orEmpty(),
            elapsedSecs = elapsed,
            currentLine = lyricLine,
            canSeek = status.canSeek,
            onSeek = actions.onSeek,
            onDismiss = { showLyrics = false },
        )
    }
}

/** Space or the play/pause media key, on key down (not on release, or it would toggle twice). */
internal fun isPlayPauseKey(e: KeyEvent): Boolean =
    e.type == KeyEventType.KeyDown && (e.key == Key.Spacebar || e.key == Key.MediaPlayPause)

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
    /** Open "Play on…" (D20). */
    val onOutputs: () -> Unit = {},
    val onBackToPhone: () -> Unit = {},
    /** "More…" for the current track. */
    val onMore: ((Track) -> Unit)? = null,
    /** Queue editing. */
    val onRemove: (Int) -> Unit = {},
    val onMove: (Int, Int) -> Unit = { _, _ -> },
    val onClearQueue: () -> Unit = {},
    /** Transfer: "Bring to this phone" (null: not offered), and hiding its result. */
    val onBringToPhone: (() -> Unit)? = null,
    val onDismissTransfer: () -> Unit = {},
    /** Close Now playing (pulling it down from the top). */
    val onClose: (() -> Unit)? = null,
    /** The sleep timer: set it, or turn it off. */
    val onSleep: (SleepChoice, SleepScope) -> Unit = { _, _ -> },
    val onCancelSleep: () -> Unit = {},
    /** Take the "Up next" track out of the queue (with Undo). */
    val onRemoveUpNext: () -> Unit = {},
    /** Rate the current track (0–100; 0 clears). */
    val onRate: (Int) -> Unit = {},
    /** Open the selected player's alarms (null: not offered). */
    val onAlarms: (() -> Unit)? = null,
    /** The queue list reached its end and there is more: read the next page. */
    val onLoadMoreQueue: () -> Unit = {},
    /** Move a queue entry to just after the current track (Wave 2 §3.4). */
    val onPlayNextInQueue: (Int) -> Unit = {},
    /** Take out every track after the current one (after asking). */
    val onClearUpcoming: () -> Unit = {},
    /** One shuffle mode (the shuffle button's long-press choice). */
    val onShuffleMode: (ShuffleMode) -> Unit = {},
    /** Now playing's layout, visualiser and queue panel were changed (to remember them). */
    val onLayout: (NowPlayingLayout) -> Unit = {},
    val onVisualiser: (Boolean) -> Unit = {},
    val onVisualStyle: (VisualStyle) -> Unit = {},
    val onQueuePanel: (Boolean) -> Unit = {},
    /** The Info layout is showing: read the current track's credits. */
    val onCredits: () -> Unit = {},
    /** Follow a credit to that person's albums in that role (null: credits aren't links). */
    val onCredit: ((io.github.jamezuk.lyrionanywhere.core.Credit) -> Unit)? = null,
    /** Open the Rooms view (Wave 2 §4.3; null: not offered). */
    val onRooms: (() -> Unit)? = null,
)

/**
 * The player's name as a chip: it opens every room at a glance ("All rooms"), or "Play on…"
 * where there is no Rooms view.
 */
@Composable
private fun PlayerChip(name: String, onClick: () -> Unit, rooms: Boolean, modifier: Modifier = Modifier) {
    val label = stringResource(if (rooms) R.string.rooms_all else R.string.play_on_title)
    Box(modifier) {
        AssistChip(
            onClick = onClick,
            label = { Text(name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
            leadingIcon = { Icon(Icons.Outlined.SpeakerGroup, contentDescription = null, modifier = Modifier.size(18.dp)) },
            modifier = Modifier
                .testTag("player-chip")
                .semantics { onClick(label = label) { onClick(); true } },
        )
    }
}

/**
 * The seek bar. With the track's waveform (from the gateway's analysis) the slider's track
 * is the waveform, played part highlighted; without one it is the plain slider. Either
 * way it is the same Material slider, so dragging, keys and TalkBack work alike.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun SeekBar(
    elapsed: Double,
    duration: Double?,
    enabled: Boolean,
    onSeek: (Double) -> Unit,
    waveform: Waveform? = null,
) {
    var dragging by remember { mutableStateOf(false) }
    var dragValue by remember { mutableFloatStateOf(0f) }
    val total = duration ?: 0.0
    val shown = if (dragging) dragValue.toDouble() else elapsed
    val position = stringResource(R.string.position)
    val waveformLabel = stringResource(R.string.waveform_seek)
    val label = if (waveform != null) "$position. $waveformLabel" else position
    val range = 0f..(if (total > 0) total.toFloat() else 1f)
    val interaction = remember { MutableInteractionSource() }
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
            valueRange = range,
            enabled = enabled && total > 0,
            interactionSource = interaction,
            modifier = Modifier.semantics {
                stateDescription = "${formatDuration(shown)} / ${formatDuration(duration)}"
                this.contentDescription = label
            },
            thumb = { SliderDefaults.Thumb(interactionSource = interaction, enabled = enabled && total > 0) },
            track = { s ->
                if (waveform != null) {
                    WaveformTrack(waveform, WaveformMath.fraction(s.value.toDouble(), range.endInclusive.toDouble()))
                } else {
                    SliderDefaults.Track(sliderState = s, enabled = enabled && total > 0)
                }
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
                .testTag("volume-slider")
                .semantics {
                    this.contentDescription = label
                    stateDescription = "${shown.toInt()}%"
                },
        )
        Icon(Icons.AutoMirrored.Outlined.VolumeUp, contentDescription = null)
    }
}
