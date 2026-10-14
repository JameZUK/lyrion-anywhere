package io.github.jamezuk.lyrionanywhere.ui.offline

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.data.VisualStyle
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingCloseButton
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PhoneVolume
import io.github.jamezuk.lyrionanywhere.ui.player.VisualSource
import kotlinx.coroutines.delay

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

/**
 * The bar above the navigation while downloads play on this phone: the main mini player
 * (Wave 3.5 item 14: one face, the downloads engine its own), saying "Downloads".
 */
@Composable
fun OfflineMiniPlayer(
    ui: OfflinePlayerUi,
    onOpen: () -> Unit,
    onToggle: () -> Unit,
    onNext: () -> Unit,
    onPrevious: () -> Unit = {},
    onOutputs: () -> Unit = {},
) {
    val state = DownloadsPlayer.state(ui, stringResource(R.string.playing_downloads)) ?: return
    MiniPlayer(state, onOpen = onOpen, onTogglePlay = onToggle, onNext = onNext, onOutputs = onOutputs, onPrevious = onPrevious)
}

/** What Now playing does for downloads (sent to the downloads engine). */
data class OfflineNowPlayingActions(
    val onToggle: () -> Unit = {},
    val onNext: () -> Unit = {},
    val onPrevious: () -> Unit = {},
    val onSeek: (Double) -> Unit = {},
    val onShuffle: () -> Unit = {},
    val onRepeat: () -> Unit = {},
    /** Play from a track: its place in the engine's list. */
    val onJump: (Int) -> Unit = {},
    val onStop: () -> Unit = {},
    /** The visualiser on or off, and its style (Wave 2 §5.7; remembered by the caller). */
    val onVisualiser: (Boolean) -> Unit = {},
    val onVisualStyle: (VisualStyle) -> Unit = {},
    /** Open *Play on…*, where there is a server to offer players. */
    val onOutputs: () -> Unit = {},
)

/**
 * Now playing, for downloads (Wave 3.5 item 14): the main Now playing, fed by
 * [DownloadsPlayer] and driving the downloads engine. The rating, sleep timer and queue edits
 * (LMS's) are left out; Stop is in ⋮. With a [visual] source the visualiser draws over the
 * cover, as for this phone's player; the volume is this phone's media volume ([phoneVolume]).
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
    twoPane: Boolean = false,
    wide: Boolean = false,
    /** What isn't the downloads' own (the other players, for *Play on…*). */
    base: io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState = io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState(),
    onMore: ((io.github.jamezuk.lyrionanywhere.core.Track) -> Unit)? = null,
    onCarMode: (() -> Unit)? = null,
) {
    val state = DownloadsPlayer.state(ui, stringResource(R.string.playing_downloads), base)
    if (state == null) {
        Box(modifier.fillMaxSize().windowInsetsPadding(WindowInsets.statusBars)) {
            EmptyState(stringResource(R.string.offline_nothing_playing), Modifier.padding(24.dp))
            onClose?.let { NowPlayingCloseButton(it, Modifier.align(Alignment.TopStart)) }
        }
        return
    }
    val shuffled = state.status?.shuffle != ShuffleMode.OFF
    NowPlayingScreen(
        state = state,
        clock = clock,
        modifier = modifier,
        twoPane = twoPane,
        wide = wide,
        visual = visual,
        phoneVolume = phoneVolume,
        actions = NowPlayingActions(
            onTogglePlay = actions.onToggle,
            onNext = actions.onNext,
            onPrevious = actions.onPrevious,
            onSeek = actions.onSeek,
            onVolume = {},
            onShuffle = actions.onShuffle,
            onRepeat = actions.onRepeat,
            onPower = {},
            onJump = DownloadsPlayer.queueActions(actions.onJump, state.queue).onJump,
            onRetry = {},
            onOutputs = actions.onOutputs,
            onClose = onClose,
            onMore = onMore,
            onShuffleMode = { m -> if ((m != ShuffleMode.OFF) != shuffled) actions.onShuffle() },
            onVisualiser = actions.onVisualiser,
            onVisualStyle = actions.onVisualStyle,
            onCarMode = onCarMode,
            onStop = {
                actions.onStop()
                onClose?.invoke()
            },
        ),
    )
}
