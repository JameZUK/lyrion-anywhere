package io.github.jamezuk.lyrionanywhere.navigation

import android.content.Context
import android.content.Intent
import android.os.SystemClock
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import io.github.jamezuk.lyrionanywhere.ui.settings.SignOutDialog
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import io.github.jamezuk.lyrionanywhere.AppContainer
import io.github.jamezuk.lyrionanywhere.playback.PlaybackService
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsActions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineBannerBar
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineLibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineMiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerViewModel
import kotlinx.serialization.Serializable

@Serializable private data object OfflineLibraryRoute

@Serializable private data object OfflineDownloadsRoute

@Serializable data class OfflineAlbumRoute(val ref: String)

@Serializable data object OfflineNowPlayingRoute

/** The offline player's clock (elapsed realtime, as the media session's). */
internal val offlineClock: () -> Long = SystemClock::elapsedRealtime

/** Start the media session service (allowed while the app is in the foreground). */
internal fun startPlayback(context: Context) {
    try {
        context.startService(Intent(context, PlaybackService::class.java))
    } catch (_: IllegalStateException) {
        // Not in the foreground after all; it starts the next time the app opens.
    }
}

/**
 * Offline mode (SPEC §8.4): the server can't be reached, so the app shows the downloaded
 * library and plays it on this phone, under an "Offline" banner. When the server is back
 * the app switches back by itself (the account state and the offline mode move on).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun OfflineApp(
    container: AppContainer,
    bannerText: String,
    onRetry: () -> Unit,
    /** Sign out (before sign-in only), keeping or removing the downloads. */
    onSignOut: ((removeDownloads: Boolean) -> Unit)?,
) {
    val nav = rememberNavController()
    var askSignOut by rememberSaveable { mutableStateOf(false) }
    val playerVm = viewModel(key = "offline-player") { OfflinePlayerViewModel(container.offlinePlayer, offlineClock) }
    val player by playerVm.state.collectAsStateWithLifecycle()
    val offline by container.offline.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    LaunchedEffect(player.state.active) {
        if (player.state.active) startPlayback(context)
    }
    val playAlbum: (String) -> Unit = { ref ->
        val tracks = OfflineAlbumViewModel.albumOf(offline, ref).tracks
        if (tracks.isNotEmpty()) container.offlinePlayer.play(tracks, 0, false)
    }
    Scaffold(
        modifier = Modifier.safeDrawingPadding(),
        topBar = {
            Column {
                OfflineBannerBar(bannerText, onRetry = onRetry) {
                    if (onSignOut != null) {
                        TextButton(onClick = { askSignOut = true }) { Text(stringResource(R.string.action_sign_out)) }
                    }
                }
            }
        },
        bottomBar = {
            OfflineMiniPlayer(
                player,
                onOpen = { nav.navigate(OfflineNowPlayingRoute) },
                onToggle = playerVm::toggle,
                onNext = playerVm::next,
            )
        },
    ) { padding ->
        NavHost(nav, startDestination = OfflineLibraryRoute, modifier = Modifier.padding(padding)) {
            composable<OfflineLibraryRoute> {
                Column(Modifier.fillMaxSize()) {
                    TopAppBar(
                        title = { TopBarTitle(stringResource(R.string.offline_library)) },
                        actions = {
                            IconButton(onClick = { nav.navigate(OfflineDownloadsRoute) }) {
                                Icon(Icons.Outlined.Download, contentDescription = stringResource(R.string.title_downloads))
                            }
                        },
                    )
                    OfflineLibraryScreen(
                        offline.albums,
                        onAlbum = { nav.navigate(OfflineAlbumRoute(it)) },
                        onPlayAlbum = playAlbum,
                    )
                }
            }
            composable<OfflineDownloadsRoute> {
                val vm = viewModel { DownloadsViewModel(container.offline) }
                val ui by vm.state.collectAsStateWithLifecycle()
                OfflineScreen(stringResource(R.string.title_downloads), onBack = { nav.popBackStack() }) {
                    DownloadsScreen(
                        ui,
                        DownloadsActions(
                            onPause = vm::pause,
                            onResume = vm::resume,
                            onCancel = vm::cancel,
                            onAlbum = { nav.navigate(OfflineAlbumRoute(it)) },
                            onPlayAlbum = playAlbum,
                            onRemoveAlbum = vm::removeAlbum,
                            onRemoveAll = vm::removeAll,
                        ),
                    )
                }
            }
            composable<OfflineAlbumRoute> { entry ->
                val ref = entry.toRoute<OfflineAlbumRoute>().ref
                val vm = viewModel(key = "offline-album-$ref") { OfflineAlbumViewModel(container.offline, container.offlinePlayer, ref) }
                val ui by vm.state.collectAsStateWithLifecycle()
                OfflineScreen(ui.album?.title ?: "", onBack = { nav.popBackStack() }) {
                    OfflineAlbumScreen(
                        ui,
                        onPlay = { vm.play(it) },
                        onShuffle = { vm.play(0, shuffle = true) },
                        onRemove = {
                            vm.remove()
                            nav.popBackStack()
                        },
                    )
                }
            }
            composable<OfflineNowPlayingRoute> {
                OfflineScreen(stringResource(R.string.title_now_playing), onBack = { nav.popBackStack() }) {
                    OfflineNowPlayingScreen(player, offlineClock, offlineNowPlayingActions(playerVm))
                }
            }
        }
    }
    if (askSignOut && onSignOut != null) {
        SignOutDialog(
            downloadedTracks = offline.tracks.size,
            downloadedBytes = offline.usedBytes,
            onSignOut = { remove ->
                askSignOut = false
                container.offlinePlayer.stop()
                onSignOut(remove)
            },
            onDismiss = { askSignOut = false },
        )
    }
}

internal fun offlineNowPlayingActions(vm: OfflinePlayerViewModel) = OfflineNowPlayingActions(
    onToggle = vm::toggle,
    onNext = vm::next,
    onPrevious = vm::previous,
    onSeek = vm::seek,
    onShuffle = vm::toggleShuffle,
    onRepeat = vm::cycleRepeat,
    onJump = vm::jumpTo,
    onStop = vm::stop,
)

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun OfflineScreen(title: String, onBack: () -> Unit, content: @Composable () -> Unit) {
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = { TopBarTitle(title) },
            navigationIcon = {
                IconButton(onClick = onBack) {
                    Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.action_back))
                }
            },
        )
        content()
    }
}

/** A top bar's title: one line (ellipsised with large text), and a heading for TalkBack. */
@Composable
private fun TopBarTitle(title: String) {
    Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.semantics { heading() })
}
