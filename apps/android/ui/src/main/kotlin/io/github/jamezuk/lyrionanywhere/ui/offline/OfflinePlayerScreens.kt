package io.github.jamezuk.lyrionanywhere.ui.offline

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.QueueMusic
import androidx.compose.material.icons.automirrored.outlined.VolumeUp
import androidx.compose.material.icons.outlined.Pause
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.Repeat
import androidx.compose.material.icons.outlined.RepeatOne
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material.icons.outlined.SkipNext
import androidx.compose.material.icons.outlined.Stop
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.CustomAccessibilityAction
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.art.offlineArtKey
import io.github.jamezuk.lyrionanywhere.ui.components.ArtworkImage
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.player.CoverPage
import io.github.jamezuk.lyrionanywhere.ui.player.GlowHalo
import io.github.jamezuk.lyrionanywhere.ui.player.HeroToast
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingAction
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingCloseButton
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingMeta
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingSheet
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingTopRow
import io.github.jamezuk.lyrionanywhere.ui.player.PhoneVolume
import io.github.jamezuk.lyrionanywhere.ui.player.PhoneVolumeMath
import io.github.jamezuk.lyrionanywhere.ui.player.PlayingOnChip
import io.github.jamezuk.lyrionanywhere.ui.player.SeekBar
import io.github.jamezuk.lyrionanywhere.ui.player.TransportRow
import io.github.jamezuk.lyrionanywhere.ui.player.UpNextLine
import io.github.jamezuk.lyrionanywhere.ui.player.VisualBlock
import io.github.jamezuk.lyrionanywhere.ui.player.VisualSource
import io.github.jamezuk.lyrionanywhere.ui.player.VisualiserAction
import io.github.jamezuk.lyrionanywhere.ui.player.VisualiserButtonModel
import io.github.jamezuk.lyrionanywhere.ui.player.VisualiserCanvas
import io.github.jamezuk.lyrionanywhere.ui.player.VisualiserStyleSheet
import io.github.jamezuk.lyrionanywhere.ui.player.VolumePopover
import io.github.jamezuk.lyrionanywhere.ui.player.phoneVolumeIndex
import io.github.jamezuk.lyrionanywhere.ui.player.rememberPhoneVolume
import io.github.jamezuk.lyrionanywhere.ui.player.rememberVisualBlock
import io.github.jamezuk.lyrionanywhere.ui.player.rememberVisualState
import io.github.jamezuk.lyrionanywhere.ui.player.visualStyleName
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
    /** The visualiser on or off, and its style (Wave 2 §5.7; remembered by the caller). */
    val onVisualiser: (Boolean) -> Unit = {},
    val onVisualStyle: (VisualStyle) -> Unit = {},
)

/**
 * Now playing, for downloads: the same layout as the online Now playing (top row, the cover
 * as large as fits, title, seek bar, transport, "Up next", and the actions along the bottom),
 * nothing scrolling. With a [visual] source the visualiser draws over the cover, as online:
 * one button, off unless chosen, and off in battery saver and with animations removed. The
 * volume is this phone's media volume ([phoneVolume]).
 */
@Composable
fun OfflineNowPlayingScreen(
    ui: OfflinePlayerUi,
    clock: () -> Long,
    actions: OfflineNowPlayingActions,
    modifier: Modifier = Modifier,
    visual: VisualSource? = null,
    /** Close Now playing (the chevron at the top left); null for none. */
    onClose: (() -> Unit)? = null,
    phoneVolume: PhoneVolume? = null,
) {
    val s = ui.state
    val status = s.status
    val track = s.current
    if (status == null || track == null) {
        Box(modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
            EmptyState(stringResource(R.string.offline_nothing_playing), Modifier.padding(24.dp))
            onClose?.let { NowPlayingCloseButton(it, Modifier.align(Alignment.TopStart)) }
        }
        return
    }
    val position = rememberOfflinePosition(ui, clock)
    // This device's visualiser choices: shown at once, kept by the caller.
    val prefs = ui.nowPlaying
    var visualOn by remember(prefs.visualiser) { mutableStateOf(prefs.visualiser) }
    var style by remember(prefs.visualStyle) { mutableStateOf(prefs.visualStyle) }
    var toast by remember { mutableStateOf<Pair<String, Int>?>(null) }
    var queueOpen by rememberSaveable { mutableStateOf(false) }
    var stylesOpen by rememberSaveable { mutableStateOf(false) }
    var volumeOpen by rememberSaveable { mutableStateOf(false) }
    val styleNames = VisualStyle.entries.associateWith { visualStyleName(it) }
    val offName = stringResource(R.string.visualiser_off_toast)
    fun say(text: String) {
        toast = text to ((toast?.second ?: 0) + 1)
    }
    val setStyle: (VisualStyle) -> Unit = {
        style = it
        actions.onVisualStyle(it)
        say(styleNames.getValue(it))
    }
    val setVisual: (Boolean) -> Unit = { on ->
        visualOn = on
        actions.onVisualiser(on)
        say(if (on) styleNames.getValue(style) else offName)
    }
    val block = rememberVisualBlock()
    val visualShown = visual != null && visualOn && block == VisualBlock.NONE
    val visualState = rememberVisualState(visual, running = visualShown)
    val glow = style == VisualStyle.GLOW
    // Glow's halo around the cover; still in battery saver and with animations removed.
    val haloShown = visual != null && visualOn && glow
    val art = artOf(track)
    val halo: (@Composable (Modifier, @Composable () -> Unit) -> Unit)? = if (haloShown) {
        { m, cover -> GlowHalo(art, visualState, playing = s.playing, animate = visualShown, modifier = m, content = cover) }
    } else {
        null
    }
    val overlay: (@Composable () -> Unit)? = if (visualShown && !glow) {
        { VisualiserCanvas(style, visualState, setStyle) }
    } else {
        null
    }
    val order = status.order.map { it.toInt() }
    val here = order.indexOf(status.index?.toInt() ?: -1)
    val nextTrack = order.getOrNull(here + 1)?.let { status.keys.getOrNull(it) }?.let { s.tracks[it] }
    val phone = rememberPhoneVolume(phoneVolume)
    val index = phoneVolumeIndex(phone)
    val nextLabel = stringResource(R.string.action_next)
    val previousLabel = stringResource(R.string.action_previous)
    val peekLabel = stringResource(R.string.queue_peek)
    Box(modifier.fillMaxSize()) {
    Column(Modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars).testTag("offline-now-playing")) {
        NowPlayingTopRow(onClose = onClose, onMore = null, modifier = Modifier.padding(horizontal = 4.dp)) {
            PlayingOnChip(stringResource(R.string.playing_downloads_on_phone), phone = true, onClick = null)
        }
        Box(Modifier.weight(1f).fillMaxWidth().padding(horizontal = 16.dp)) {
            CoverPage(
                artwork = art,
                actions = listOf(
                    CustomAccessibilityAction(nextLabel) { actions.onNext(); true },
                    CustomAccessibilityAction(previousLabel) { actions.onPrevious(); true },
                    CustomAccessibilityAction(peekLabel) { queueOpen = true; true },
                ),
                overlay = overlay,
                halo = halo,
                onSwipeUp = { queueOpen = true },
                onSwipeDown = onClose,
            )
            HeroToast(toast, onGone = { toast = null }, modifier = Modifier.align(Alignment.TopCenter).padding(top = 12.dp))
        }
        Column(Modifier.fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            NowPlayingMeta(
                title = track.title,
                subtitle = listOf(track.artist, track.album).filter { it.isNotEmpty() }.joinToString(" · "),
                below = status.lastError?.let { e ->
                    {
                        Text(
                            e,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.error,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.semantics { liveRegion = LiveRegionMode.Polite },
                        )
                    }
                },
            )
            SeekBar(
                elapsed = position / 1000.0,
                duration = (status.durationMs.toLong() / 1000.0).takeIf { it > 0 },
                enabled = true,
                onSeek = actions.onSeek,
            )
            TransportRow(
                playing = s.playing,
                onTogglePlay = actions.onToggle,
                onPrevious = actions.onPrevious,
                onNext = actions.onNext,
                shuffle = {
                    // Constant names, with the mode as their state (as on the online player).
                    val shuffleState = stringResource(if (status.shuffle) R.string.toggle_on else R.string.mode_off)
                    IconToggleButton(
                        checked = status.shuffle,
                        onCheckedChange = { actions.onShuffle() },
                        modifier = Modifier.semantics { stateDescription = shuffleState },
                    ) {
                        Icon(Icons.Outlined.Shuffle, contentDescription = stringResource(R.string.shuffle))
                    }
                },
                repeat = {
                    val repeatState = stringResource(
                        when (status.repeat) {
                            LocalRepeat.OFF -> R.string.mode_off
                            LocalRepeat.ALL -> R.string.repeat_all
                            LocalRepeat.ONE -> R.string.repeat_one
                        },
                    )
                    IconToggleButton(
                        checked = status.repeat != LocalRepeat.OFF,
                        onCheckedChange = { actions.onRepeat() },
                        modifier = Modifier.semantics { stateDescription = repeatState },
                    ) {
                        Icon(
                            if (status.repeat == LocalRepeat.ONE) Icons.Outlined.RepeatOne else Icons.Outlined.Repeat,
                            contentDescription = stringResource(R.string.repeat),
                        )
                    }
                },
            )
            nextTrack?.let { n ->
                UpNextLine(n.title, n.artist, onOpenQueue = { queueOpen = true }, onRemove = null, modifier = Modifier.padding(top = 4.dp))
            }
            Row(Modifier.fillMaxWidth().padding(top = 4.dp).testTag("now-playing-actions"), verticalAlignment = Alignment.CenterVertically) {
                NowPlayingAction(
                    icon = Icons.AutoMirrored.Outlined.QueueMusic,
                    label = stringResource(R.string.nav_queue),
                    description = stringResource(R.string.queue_peek),
                    onClick = { queueOpen = true },
                    modifier = Modifier.weight(1f).testTag("queue-button"),
                )
                if (visual != null) {
                    VisualiserAction(
                        VisualiserButtonModel(on = visualOn, style = style, block = block, onToggle = { setVisual(!visualOn) }, onStyles = { stylesOpen = true }),
                        Modifier.weight(1f),
                    )
                }
                val percent = PhoneVolumeMath.percent(index, phone.max)
                NowPlayingAction(
                    icon = Icons.AutoMirrored.Outlined.VolumeUp,
                    label = stringResource(R.string.volume),
                    description = stringResource(R.string.volume_on, stringResource(R.string.volume_this_phone)),
                    state = "$percent%",
                    onClick = { volumeOpen = !volumeOpen },
                    modifier = Modifier.weight(1f).testTag("volume-button"),
                )
                NowPlayingAction(
                    icon = Icons.Outlined.Stop,
                    label = stringResource(R.string.action_stop),
                    description = stringResource(R.string.offline_stop),
                    onClick = actions.onStop,
                    modifier = Modifier.weight(1f).testTag("offline-stop"),
                )
            }
        }
    }
    }
    if (queueOpen) {
        NowPlayingSheet(stringResource(R.string.section_queue), "offline-queue", onDismiss = { queueOpen = false }) {
            order.forEach { listIndex ->
                val t = status.keys.getOrNull(listIndex)?.let { s.tracks[it] } ?: return@forEach
                OfflineTrackRow(t, onClick = { actions.onJump(listIndex) }, highlighted = listIndex == status.index?.toInt())
            }
        }
    }
    if (stylesOpen) {
        VisualiserStyleSheet(
            on = visualOn,
            style = style,
            onChoose = { st ->
                setStyle(st)
                if (!visualOn) setVisual(true)
            },
            onOff = { setVisual(false) },
            onDismiss = { stylesOpen = false },
        )
    }
    if (volumeOpen) {
        VolumePopover(
            name = stringResource(R.string.volume_this_phone),
            note = stringResource(if (phone.fixed) R.string.volume_phone_fixed else R.string.volume_phone_note),
            value = PhoneVolumeMath.percent(index, phone.max),
            enabled = !phone.fixed,
            onChange = { phone.setIndex(PhoneVolumeMath.index(it.toFloat(), phone.max)) },
            onDone = { phone.setIndex(PhoneVolumeMath.index(it.toFloat(), phone.max)) },
            onDismiss = { volumeOpen = false },
        )
    }
}
