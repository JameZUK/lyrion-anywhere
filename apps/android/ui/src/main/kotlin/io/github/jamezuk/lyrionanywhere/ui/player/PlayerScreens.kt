package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material.icons.outlined.Alarm
import androidx.compose.material.icons.outlined.Bedtime
import androidx.compose.material.icons.outlined.Devices
import androidx.compose.material.icons.outlined.DirectionsCar
import androidx.compose.material.icons.outlined.GraphicEq
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.PowerSettingsNew
import androidx.compose.material.icons.outlined.Repeat
import androidx.compose.material.icons.outlined.RepeatOne
import androidx.compose.material.icons.outlined.SkipNext
import androidx.compose.material.icons.outlined.SpeakerGroup
import androidx.compose.material.icons.outlined.Style
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.surfaceColorAtElevation
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.customActions
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.NowPlayingLayout
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.data.Waveform
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.HapticSteps
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.ui.components.formatDuration
import io.github.jamezuk.lyrionanywhere.ui.theme.AmbientBackground
import io.github.jamezuk.lyrionanywhere.ui.theme.ArtTinted
import io.github.jamezuk.lyrionanywhere.ui.theme.isHighContrast
import io.github.jamezuk.lyrionanywhere.ui.theme.rememberArtSeed
import io.github.jamezuk.lyrionanywhere.ui.theme.tinted
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
 * Full remote control for the selected player, laid out as the owner-approved mock-up:
 *
 * 1. a slim top row: close, "Playing on <player> ▾" (opens *Play on…*), ⋮ (car mode,
 *    alarms, all rooms, the signal path, the visualiser's style);
 * 2. the hero: a pager of Cover, Lyrics and Track info with page dots, taking all the height
 *    left (the cover is square and shrinks to fit);
 * 3. the title with the rating, the artist and album with the quality badge;
 * 4. the waveform seek bar; 5. the transport; 6. one "Up next" line;
 * 7. four actions: Sleep, Queue, Visualiser, Volume.
 *
 * Nothing on the screen scrolls (the lyrics and info scroll inside their page), so the
 * transport is always fully on screen. With [twoPane] (a phone held sideways) the hero sits
 * beside the controls; with [wide] (over 840 dp) the queue can sit in a panel beside them.
 *
 * The visualiser (from [visual], this phone only) draws over the cover; its one button
 * switches it on and off, a long press chooses the style, and a sideways swipe across it
 * changes the style. It stays off in battery saver and with animations removed. The volume
 * popover sets a remote player's volume in LMS, or this phone's media volume ([phoneVolume])
 * while this phone is the player.
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
    phoneVolume: PhoneVolume? = null,
) {
    val status = state.status
    val player = state.selected
    if (player == null || status == null) {
        // Nothing to show yet: the close button still sits at the top left.
        Box(modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
            NowPlayingPlaceholder(state, actions)
            actions.onClose?.let { close -> NowPlayingCloseButton(close, Modifier.align(Alignment.TopStart)) }
        }
        return
    }
    NowPlayingContent(state, status, player, clock, actions, modifier, twoPane, wide, visual, rememberPhoneVolume(phoneVolume))
}

/** Now playing without a player chosen, or before its status has come. */
@Composable
private fun NowPlayingPlaceholder(state: PlayerUiState, actions: NowPlayingActions, modifier: Modifier = Modifier) {
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
    }
}

/** Which of Now playing's sheets (or the volume popover) is open. */
private enum class NowPlayingPanel { NONE, MORE, STYLES, PATH, QUEUE, SLEEP, VOLUME }

@Composable
private fun NowPlayingContent(
    state: PlayerUiState,
    status: PlayerStatus,
    player: Player,
    clock: () -> Long,
    actions: NowPlayingActions,
    modifier: Modifier,
    twoPane: Boolean,
    wide: Boolean,
    visual: VisualSource?,
    phoneVolume: PhoneVolume,
) {
    val track = status.current
    var panel by rememberSaveable { mutableStateOf(NowPlayingPanel.NONE) }
    val close: () -> Unit = { panel = NowPlayingPanel.NONE }
    val togglePlay by rememberUpdatedState(actions.onTogglePlay)
    val closeScreen by rememberUpdatedState(actions.onClose)

    // This device's choices: shown at once, kept by the caller.
    val prefs = state.nowPlaying
    var page by remember(prefs.layout) { mutableStateOf(prefs.layout) }
    var visualOn by remember(prefs.visualiser) { mutableStateOf(prefs.visualiser) }
    var style by remember(prefs.visualStyle) { mutableStateOf(prefs.visualStyle) }
    var queuePanel by remember(prefs.queuePanel) { mutableStateOf(prefs.queuePanel) }
    var toast by remember { mutableStateOf<Pair<String, Int>?>(null) }
    val styleNames = VisualStyle.entries.associateWith { visualStyleName(it) }
    val offName = stringResource(R.string.visualiser_off_toast)
    fun say(text: String) {
        toast = text to ((toast?.second ?: 0) + 1)
    }
    val setPage: (NowPlayingLayout) -> Unit = {
        if (it != page) {
            page = it
            actions.onLayout(it)
        }
    }
    val setStyle: (VisualStyle) -> Unit = {
        style = it
        actions.onVisualStyle(it)
        say(styleNames.getValue(it))
    }
    val setVisual: (Boolean) -> Unit = { on ->
        visualOn = on
        actions.onVisualiser(on)
        if (on) setPage(NowPlayingLayout.ART)
        say(if (on) styleNames.getValue(style) else offName)
    }
    val block = rememberVisualBlock()
    // Only while this phone is the player (a remote player has no audio here).
    val canVisualise = state.localSelected
    val onCover = wide || twoPane || page == NowPlayingLayout.ART
    val visualShown = canVisualise && visualOn && block == VisualBlock.NONE && onCover
    val visualState = rememberVisualState(visual, running = visualShown)
    val glow = style == VisualStyle.GLOW
    // Glow's halo: shown whenever Glow is chosen for the cover; still (no turn, no pulse) in
    // battery saver and with animations removed, as the visualiser is off then.
    val haloShown = canVisualise && visualOn && glow && onCover
    val halo: (@Composable (Modifier, @Composable () -> Unit) -> Unit)? = if (haloShown) {
        { m, cover -> GlowHalo(track?.artwork, visualState, playing = state.playing, animate = visualShown, modifier = m, content = cover) }
    } else {
        null
    }
    val overlay: (@Composable () -> Unit)? = if (visualShown && !glow) {
        { VisualiserCanvas(style, visualState, setStyle) }
    } else {
        null
    }
    // Wide windows keep the cover on the left and the lyrics or info in their own pane.
    val pages = if (wide) listOf(NowPlayingLayout.ART) else NowPlayingLayouts.PAGES
    val heroPage = if (wide) NowPlayingLayout.ART else page
    val sidePage = NowPlayingLayouts.widePane(page)
    val infoShown = if (wide) sidePage == NowPlayingLayout.INFO else page == NowPlayingLayout.INFO
    LaunchedEffect(infoShown, track?.id) { if (infoShown) actions.onCredits() }

    val openQueue: () -> Unit = {
        if (wide) {
            queuePanel = !queuePanel
            actions.onQueuePanel(queuePanel)
        } else {
            panel = NowPlayingPanel.QUEUE
        }
    }
    val nextLabel = stringResource(R.string.action_next)
    val previousLabel = stringResource(R.string.action_previous)
    val peekLabel = stringResource(R.string.queue_peek)
    val latestActions by rememberUpdatedState(actions)

    val pageContent: @Composable (NowPlayingLayout, List<NowPlayingLayout>) -> Unit = { p, all ->
        val moves = pageActions(all, p, setPage)
        when (p) {
            NowPlayingLayout.ART -> CoverPage(
                artwork = track?.artwork,
                actions = listOf(
                    CustomAccessibilityAction(nextLabel) { latestActions.onNext(); true },
                    CustomAccessibilityAction(previousLabel) { latestActions.onPrevious(); true },
                    CustomAccessibilityAction(peekLabel) { openQueue(); true },
                ) + moves,
                overlay = overlay,
                halo = halo,
                onSwipeUp = openQueue,
                onSwipeDown = { closeScreen?.invoke() },
            )
            NowPlayingLayout.LYRICS -> TextPage(p, moves) { CurrentLyrics(state, status, clock, actions.onSeek) }
            NowPlayingLayout.INFO -> TextPage(p, moves) { InfoPanel(state, status, actions) }
        }
    }
    val hero: @Composable (Modifier) -> Unit = { m ->
        Box(m) {
            HeroPager(
                pages = pages,
                page = heroPage,
                onPage = setPage,
                // Across the visualiser a sideways swipe changes its style instead.
                swipe = overlay == null,
                modifier = Modifier.fillMaxSize(),
            ) { p -> pageContent(p, pages) }
            HeroToast(toast, onGone = { toast = null }, modifier = Modifier.align(Alignment.TopCenter).padding(top = 12.dp))
        }
    }
    val chip: @Composable () -> Unit = {
        PlayingOnChip(stringResource(R.string.playing_on, player.name), phone = state.localSelected, onClick = actions.onOutputs)
    }
    val volume = rememberNowPlayingVolume(state, status, phoneVolume, actions)
    val controls: @Composable (Modifier, Boolean) -> Unit = { m, roomy ->
        NowPlayingControls(
            state = state,
            status = status,
            clock = clock,
            actions = actions,
            onShowPath = { panel = NowPlayingPanel.PATH },
            onOpenQueue = openQueue,
            onLyrics = { setPage(NowPlayingLayout.LYRICS) },
            lyricsShown = page == NowPlayingLayout.LYRICS && !wide,
            roomy = roomy,
            queueShown = wide && queuePanel,
            visualiser = if (canVisualise) {
                VisualiserButtonModel(on = visualOn, style = style, block = block, onToggle = { setVisual(!visualOn) }, onStyles = { panel = NowPlayingPanel.STYLES })
            } else {
                null
            },
            volume = volume,
            onSleep = { panel = NowPlayingPanel.SLEEP },
            onVolume = { panel = if (panel == NowPlayingPanel.VOLUME) NowPlayingPanel.NONE else NowPlayingPanel.VOLUME },
            modifier = m,
        )
    }
    val top: @Composable () -> Unit = {
        NowPlayingTopRow(onClose = actions.onClose, onMore = { panel = NowPlayingPanel.MORE }, chip = chip)
    }

    ArtTinted(track?.artwork) {
    Box(modifier.fillMaxSize()) {
    AmbientBackground(track?.artwork)
    Column(
        Modifier
            .fillMaxSize()
            // No top app bar: the page runs under the status bar, its content below it.
            .windowInsetsPadding(WindowInsets.statusBars)
            .testTag("now-playing")
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
    when {
        wide -> Row(Modifier.fillMaxSize().testTag("now-playing-wide")) {
            Column(Modifier.weight(1f).fillMaxHeight().padding(horizontal = 16.dp)) {
                top()
                hero(Modifier.weight(1f).fillMaxWidth())
                controls(Modifier.fillMaxWidth(), true)
            }
            VerticalDivider()
            Column(Modifier.weight(1f).fillMaxHeight().padding(16.dp)) {
                PageDots(NowPlayingLayouts.WIDE, sidePage, setPage, Modifier.align(Alignment.CenterHorizontally))
                Box(Modifier.weight(1f).fillMaxWidth().testTag("wide-pane-${sidePage.name}")) {
                    pageContent(sidePage, NowPlayingLayouts.WIDE)
                }
            }
            if (queuePanel) {
                VerticalDivider()
                val drag = remember { QueueDrag() }
                LazyColumn(Modifier.width(360.dp).fillMaxHeight().testTag("queue-pane"), contentPadding = PaddingValues(vertical = 8.dp)) {
                    item(key = "queue-header") {
                        QueueHeader(state.queue.size, player.name, actions.onClearQueue, state.upcoming, actions.onClearUpcoming)
                    }
                    queueItems(state.queue, status.queueIndex?.toInt(), drag, actions.queueActions(), hasMore = state.queueMore, sections = state.sections)
                }
            }
        }
        twoPane -> Row(Modifier.fillMaxSize().testTag("now-playing-two-pane")) {
            hero(Modifier.weight(1f).fillMaxHeight().padding(8.dp))
            // Held sideways a phone is short: the controls may scroll if they must.
            Column(Modifier.weight(1f).fillMaxHeight().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp)) {
                top()
                controls(Modifier.fillMaxWidth(), false)
            }
        }
        else -> BoxWithConstraints(Modifier.fillMaxSize()) {
            // The lyric line under the title only where the cover can spare the room.
            val roomy = maxHeight >= ROOMY_HEIGHT
            Column(Modifier.fillMaxSize().testTag("now-playing-phone")) {
                Box(Modifier.padding(horizontal = 4.dp)) { top() }
                hero(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 16.dp))
                controls(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 8.dp), roomy)
            }
        }
    }
    }
    }
    }
    when (panel) {
        NowPlayingPanel.NONE -> {}
        NowPlayingPanel.MORE -> MoreSheet(
            state = state,
            status = status,
            actions = actions,
            styleName = if (canVisualise) styleNames.getValue(style) else null,
            onPath = { panel = NowPlayingPanel.PATH },
            onStyles = { panel = NowPlayingPanel.STYLES },
            onDismiss = close,
        )
        NowPlayingPanel.STYLES -> VisualiserStyleSheet(
            on = visualOn,
            style = style,
            onChoose = { s ->
                setStyle(s)
                if (!visualOn) setVisual(true)
            },
            onOff = { setVisual(false) },
            onDismiss = close,
        )
        NowPlayingPanel.PATH -> SignalPathSheet(state, onDismiss = close)
        NowPlayingPanel.QUEUE -> QueuePeekSheet(state, actions, onDismiss = close)
        NowPlayingPanel.SLEEP -> {
            val selected = state.selectedId
            SleepSheet(
                left = SleepTimer.left(status),
                grouped = selected != null && state.syncGroups.any { selected in it.playerIds },
                others = state.players.any { it.id != selected && it.connected },
                canEnd = track?.id != null && ((status.durationSecs ?: track.durationSecs ?: 0.0) > 0.0),
                fadeSecs = state.sleepFade,
                onSet = actions.onSleep,
                onCancel = actions.onCancelSleep,
                onDismiss = close,
            )
        }
        NowPlayingPanel.VOLUME -> VolumePopover(
            name = volume.name,
            note = volume.note,
            value = volume.percent,
            enabled = volume.enabled,
            onChange = volume.onChange,
            onDone = volume.onDone,
            onDismiss = close,
        )
    }
}

/** The phone layout's height from which the lyric line shows under the title. */
private val ROOMY_HEIGHT = 720.dp

/** The volume control's target: a player in LMS, or this phone's media volume. */
internal class VolumeModel(
    val name: String,
    val note: String,
    val percent: Int,
    val enabled: Boolean,
    val onChange: (Int) -> Unit,
    val onDone: (Int) -> Unit,
)

/**
 * What the Volume button sets: the remote player's volume in LMS (as other controllers do),
 * or, while this phone is the player, Android's media volume (as the hardware keys do; the
 * player's own level in LMS is in Settings › Sound).
 */
@Composable
private fun rememberNowPlayingVolume(state: PlayerUiState, status: PlayerStatus, phone: PhoneVolume, actions: NowPlayingActions): VolumeModel {
    if (state.localSelected) {
        val index = phoneVolumeIndex(phone)
        val max = phone.max
        return VolumeModel(
            name = stringResource(R.string.volume_this_phone),
            note = stringResource(if (phone.fixed) R.string.volume_phone_fixed else R.string.volume_phone_note),
            percent = PhoneVolumeMath.percent(index, max),
            enabled = !phone.fixed,
            onChange = { phone.setIndex(PhoneVolumeMath.index(it.toFloat(), max)) },
            onDone = { phone.setIndex(PhoneVolumeMath.index(it.toFloat(), max)) },
        )
    }
    val name = state.selected?.name.orEmpty()
    return VolumeModel(
        name = name,
        note = name,
        percent = status.volume.toInt(),
        enabled = true,
        onChange = {},
        onDone = actions.onVolume,
    )
}

/** The current track's lyrics as Now playing's Lyrics page, following the song. */
@Composable
private fun CurrentLyrics(
    state: PlayerUiState,
    status: PlayerStatus,
    clock: () -> Long,
    onSeek: (Double) -> Unit,
) {
    val track = status.current
    val lyrics = state.lyrics?.takeIf { track?.id != null && state.lyricsTrackId == track.id }
    val line = rememberLyricLine(lyrics, state, clock)
    LyricsPanel(lyrics, line, status.canSeek, onSeek)
}

/** The visualiser's one button. */
internal class VisualiserButtonModel(
    val on: Boolean,
    val style: VisualStyle,
    val block: VisualBlock,
    val onToggle: () -> Unit,
    val onStyles: () -> Unit,
)

/**
 * The visualiser's one button (for the action row): a tap switches it on or off, a long
 * press chooses the style. Off, and saying why, in battery saver or with animations removed.
 */
@Composable
internal fun VisualiserAction(m: VisualiserButtonModel, modifier: Modifier = Modifier) {
    val blocked = m.block != VisualBlock.NONE
    val on = m.on && !blocked
    val state = when (m.block) {
        VisualBlock.BATTERY_SAVER -> stringResource(R.string.visualiser_off_battery)
        VisualBlock.ANIMATIONS_OFF -> stringResource(R.string.visualiser_off_motion)
        VisualBlock.NONE -> if (on) visualStyleName(m.style) else stringResource(R.string.mode_off)
    }
    NowPlayingAction(
        icon = Icons.Outlined.GraphicEq,
        label = if (on) visualStyleName(m.style) else stringResource(R.string.visualiser),
        description = stringResource(R.string.visualiser),
        onClick = m.onToggle,
        on = on,
        enabled = !blocked,
        state = state,
        onLongClick = m.onStyles,
        onLongClickLabel = stringResource(R.string.visualiser_choose_style),
        modifier = modifier.testTag("visualiser-button"),
    )
}

/** The title, details, seek bar, transport, "Up next" and the four actions. */
@Composable
private fun NowPlayingControls(
    state: PlayerUiState,
    status: PlayerStatus,
    clock: () -> Long,
    actions: NowPlayingActions,
    onShowPath: () -> Unit,
    onOpenQueue: () -> Unit,
    onLyrics: () -> Unit,
    lyricsShown: Boolean,
    /** Room for the lyric line under the title (it would take it from the cover otherwise). */
    roomy: Boolean,
    queueShown: Boolean,
    visualiser: VisualiserButtonModel?,
    volume: VolumeModel,
    onSleep: () -> Unit,
    onVolume: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val track = status.current
    val elapsed = rememberElapsed(state, clock)
    val lyrics = state.lyrics?.takeIf { track?.id != null && state.lyricsTrackId == track.id }
    // The playing lyric line, updated as each line starts (not on the once-a-second tick).
    val lyricLine = rememberLyricLine(lyrics, state, clock)
    Column(modifier.testTag("now-playing-controls"), verticalArrangement = Arrangement.spacedBy(2.dp)) {
        NowPlayingMeta(
            title = track?.title ?: stringResource(R.string.nothing_playing),
            subtitle = listOfNotNull(track?.artist, track?.album).joinToString(" · "),
            rating = if ((track?.id ?: 0) > 0) {
                { RatingControl(state.rating, state.ratingStyle, state.ratingFailed, actions.onRate) }
            } else {
                null
            },
            badge = track?.quality?.let { q -> { QualityBadge(q, onClick = onShowPath, compact = true) } },
            below = if (roomy && state.lyricsPeek && lyrics?.synced == true && !lyricsShown) {
                lyrics.lines.getOrNull(lyricLine)?.text?.takeIf { it.isNotEmpty() }?.let { line ->
                    { LyricsPeek(line, onOpen = onLyrics, maxLines = 1) }
                }
            } else {
                null
            },
        )
        SeekBar(
            elapsed = elapsed,
            duration = status.durationSecs,
            enabled = status.canSeek && track != null,
            onSeek = actions.onSeek,
            waveform = state.waveform,
        )
        TransportRow(
            playing = state.playing,
            onTogglePlay = actions.onTogglePlay,
            onPrevious = actions.onPrevious,
            onNext = actions.onNext,
            shuffle = { ShuffleButton(status.shuffle, onCycle = actions.onShuffle, onChoose = actions.onShuffleMode) },
            repeat = { RepeatButton(status.repeat, actions.onRepeat) },
        )
        status.next?.let { next ->
            UpNextRow(next, onOpenQueue = onOpenQueue, onRemove = actions.onRemoveUpNext, modifier = Modifier.padding(top = 4.dp))
        }
        Row(Modifier.fillMaxWidth().padding(top = 4.dp).testTag("now-playing-actions"), verticalAlignment = Alignment.CenterVertically) {
            val left = SleepTimer.left(status)
            val sleepLeft = left?.let { sleepLeftText(it) }
            NowPlayingAction(
                icon = Icons.Outlined.Bedtime,
                label = sleepLeft ?: stringResource(R.string.action_sleep),
                description = if (sleepLeft != null) stringResource(R.string.sleep_timer_left, sleepLeft) else stringResource(R.string.sleep_timer),
                on = null,
                onClick = onSleep,
                modifier = Modifier.weight(1f).testTag("sleep-button"),
            )
            NowPlayingAction(
                icon = Icons.AutoMirrored.Outlined.QueueMusic,
                label = stringResource(R.string.nav_queue),
                description = stringResource(R.string.queue_peek),
                state = if (queueShown) stringResource(R.string.queue_panel_shown) else null,
                onClick = onOpenQueue,
                modifier = Modifier.weight(1f).testTag("queue-button"),
            )
            if (visualiser != null) VisualiserAction(visualiser, Modifier.weight(1f))
            NowPlayingAction(
                icon = Icons.AutoMirrored.Outlined.VolumeUp,
                label = stringResource(R.string.volume),
                description = stringResource(R.string.volume_on, volume.name),
                state = "${volume.percent}%",
                onClick = onVolume,
                modifier = Modifier.weight(1f).testTag("volume-button"),
            )
        }
    }
}

/** Repeat: Off → All → One; its name stays "Repeat", its state says which. */
@Composable
internal fun RepeatButton(repeat: RepeatMode, onCycle: () -> Unit) {
    val repeatState = stringResource(
        when (repeat) {
            RepeatMode.OFF -> R.string.mode_off
            RepeatMode.ONE -> R.string.repeat_one
            RepeatMode.ALL -> R.string.repeat_all
        },
    )
    IconToggleButton(
        checked = repeat != RepeatMode.OFF,
        onCheckedChange = { onCycle() },
        modifier = Modifier.testTag("repeat-button").semantics { stateDescription = repeatState },
    ) {
        Icon(
            if (repeat == RepeatMode.ONE) Icons.Outlined.RepeatOne else Icons.Outlined.Repeat,
            contentDescription = stringResource(R.string.repeat),
        )
    }
}

/**
 * ⋮ on Now playing: the occasional things. Car mode, alarms, every room, the signal path and
 * the visualiser's style (while this phone plays); then the track's own options and the
 * player's power.
 */
@Composable
private fun MoreSheet(
    state: PlayerUiState,
    status: PlayerStatus,
    actions: NowPlayingActions,
    styleName: String?,
    onPath: () -> Unit,
    onStyles: () -> Unit,
    onDismiss: () -> Unit,
) {
    val playerName = state.selected?.name.orEmpty()
    NowPlayingSheet(stringResource(R.string.now_playing_more), "now-playing-more-sheet", onDismiss) {
        actions.onCarMode?.let { open ->
            SheetRow(Icons.Outlined.DirectionsCar, stringResource(R.string.car_mode), stringResource(R.string.car_mode_detail), {
                onDismiss()
                open()
            }, tag = "more-car-mode")
        }
        actions.onAlarms?.let { open ->
            SheetRow(Icons.Outlined.Alarm, stringResource(R.string.alarms), stringResource(R.string.alarms_detail, playerName), {
                onDismiss()
                open()
            }, tag = "more-alarms")
        }
        actions.onRooms?.let { open ->
            SheetRow(Icons.Outlined.SpeakerGroup, stringResource(R.string.rooms_all), stringResource(R.string.rooms_all_detail), {
                onDismiss()
                open()
            }, tag = "more-rooms")
        }
        SheetRow(Icons.Outlined.GraphicEq, stringResource(R.string.signal_path), stringResource(R.string.signal_path_detail), onPath, tag = "more-signal-path")
        if (styleName != null) {
            SheetRow(Icons.Outlined.Style, stringResource(R.string.visualiser_style_title), styleName, onStyles, tag = "more-visualiser-style")
        }
        val track = status.current
        val more = actions.onMore
        if (track?.id != null && more != null) {
            SheetRow(Icons.Outlined.MoreVert, stringResource(R.string.track_options), track.title, {
                onDismiss()
                more(track)
            }, tag = "more-track")
        }
        val powerState = stringResource(if (status.powered) R.string.toggle_on else R.string.mode_off)
        ListItem(
            leadingContent = { Icon(Icons.Outlined.PowerSettingsNew, contentDescription = null) },
            headlineContent = { Text(stringResource(R.string.power_named, playerName)) },
            trailingContent = { Switch(checked = status.powered, onCheckedChange = null) },
            colors = androidx.compose.material3.ListItemDefaults.colors(containerColor = androidx.compose.ui.graphics.Color.Transparent),
            modifier = Modifier
                .toggleable(value = status.powered, role = Role.Switch, onValueChange = actions.onPower)
                .semantics { stateDescription = powerState }
                .testTag("more-power"),
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
    /** A remote player's volume in LMS (0–100). */
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
    /** Close Now playing (the chevron, back, or a swipe down on the cover). */
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
    /** Now playing's page, visualiser and queue panel were changed (to remember them). */
    val onLayout: (NowPlayingLayout) -> Unit = {},
    val onVisualiser: (Boolean) -> Unit = {},
    val onVisualStyle: (VisualStyle) -> Unit = {},
    val onQueuePanel: (Boolean) -> Unit = {},
    /** The Track info page is showing: read the current track's credits. */
    val onCredits: () -> Unit = {},
    /** Follow a credit to that person's albums in that role (null: credits aren't links). */
    val onCredit: ((io.github.jamezuk.lyrionanywhere.core.Credit) -> Unit)? = null,
    /** Open the Rooms view (Wave 2 §4.3; null: not offered). */
    val onRooms: (() -> Unit)? = null,
    /** Open car mode (Wave 2 §5.1; null: not offered). */
    val onCarMode: (() -> Unit)? = null,
)

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
    val haptics = LocalHapticFeedback.current
    Column(Modifier.fillMaxWidth()) {
        Slider(
            value = if (total > 0) shown.toFloat().coerceIn(0f, total.toFloat()) else 0f,
            onValueChange = {
                // A light tick at every tenth of the track (Wave 2 §5.4).
                val before = if (dragging) dragValue else shown.toFloat()
                if (total > 0 && HapticSteps.crossed(before / total.toFloat(), it / total.toFloat(), HapticSteps.SCRUB)) {
                    haptics.performHapticFeedback(HapticSteps.tick)
                }
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
            Text(formatDuration(shown), style = MaterialTheme.typography.labelMedium, modifier = Modifier.testTag("seek-elapsed"))
            Spacer(Modifier.weight(1f))
            // The time left (the whole length while it isn't known).
            val left = if (total > 0) "−" + formatDuration((total - shown).coerceAtLeast(0.0)) else formatDuration(duration)
            Text(left, style = MaterialTheme.typography.labelMedium, modifier = Modifier.testTag("seek-remaining"))
        }
    }
}

