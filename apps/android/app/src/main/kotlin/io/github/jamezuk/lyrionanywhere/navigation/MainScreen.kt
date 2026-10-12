package io.github.jamezuk.lyrionanywhere.navigation

import android.content.Context
import android.content.Intent
import android.content.res.Resources
import android.os.SystemClock
import androidx.compose.foundation.layout.Column
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteType
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffoldDefaults
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfo
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.LibraryMusic
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Shuffle
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.viewmodel.compose.viewModel
import androidx.navigation.NavDestination.Companion.hasRoute
import androidx.navigation.NavDestination.Companion.hierarchy
import androidx.navigation.NavGraph.Companion.findStartDestination
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.currentBackStackEntryAsState
import androidx.navigation.compose.rememberNavController
import androidx.navigation.toRoute
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarResult
import androidx.compose.runtime.CompositionLocalProvider
import io.github.jamezuk.lyrionanywhere.AppContainer
import io.github.jamezuk.lyrionanywhere.BuildConfig
import io.github.jamezuk.lyrionanywhere.LyrionApp
import io.github.jamezuk.lyrionanywhere.ui.components.ListDetail
import io.github.jamezuk.lyrionanywhere.ui.components.LocalWindowLayout
import io.github.jamezuk.lyrionanywhere.ui.components.WindowLayout
import io.github.jamezuk.lyrionanywhere.ui.player.TransferState
import io.github.jamezuk.lyrionanywhere.ui.player.errorWithDetail
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import io.github.jamezuk.lyrionanywhere.core.LibraryFilter
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.libraryVersion
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.WaveformCache
import io.github.jamezuk.lyrionanywhere.playback.PlaybackService
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.EmptyState
import io.github.jamezuk.lyrionanywhere.ui.components.errorText
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumActions
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumGrid
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumSortMenu
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistActions
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistList
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistScreen
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistViewChips
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.CollectionHeader
import io.github.jamezuk.lyrionanywhere.ui.library.FavouritesScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FavouritesViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.FolderScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FolderViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.GenreNameViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.GenresScreen
import io.github.jamezuk.lyrionanywhere.ui.library.GenresViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.HomeActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeEditActions
import io.github.jamezuk.lyrionanywhere.ui.library.HomeFeedViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.HomeMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.HistorySettings
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryViewModel
import io.github.jamezuk.lyrionanywhere.core.RunnerUpdate
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryDestination
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistScreen
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistsScreen
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.SearchResultsList
import io.github.jamezuk.lyrionanywhere.ui.library.SearchViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.YearsScreen
import io.github.jamezuk.lyrionanywhere.ui.library.YearsViewModel
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryController
import io.github.jamezuk.lyrionanywhere.ui.discovery.DiscoveryEvent
import io.github.jamezuk.lyrionanywhere.ui.menus.AppsScreen
import io.github.jamezuk.lyrionanywhere.ui.menus.AppsViewModel
import io.github.jamezuk.lyrionanywhere.ui.menus.ItemSheetContent
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuBrowser
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuEvent
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuNavigator
import io.github.jamezuk.lyrionanywhere.ui.menus.MenuViewModel
import io.github.jamezuk.lyrionanywhere.ui.menus.More
import io.github.jamezuk.lyrionanywhere.ui.menus.MoreRequest
import io.github.jamezuk.lyrionanywhere.ui.player.DefaultPlayerSetting
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnActions
import io.github.jamezuk.lyrionanywhere.ui.player.PlayOnList
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.LicencesScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.actions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMessage
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsActions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadsViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineAlbumViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineMiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineNowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflinePlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.offline.StorageSettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.offline.StorageSettingsViewModel
import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.core.content.ContextCompat
import kotlinx.coroutines.launch
import kotlinx.serialization.Serializable

@Serializable data object HomeRoute

@Serializable data object LibraryRoute

@Serializable data object SearchRoute

@Serializable data object NewMusicRoute

@Serializable data object AlbumsRoute

@Serializable data object ArtistsRoute

@Serializable data object GenresRoute

@Serializable data class GenreRoute(val id: Long, val name: String)

@Serializable data object YearsRoute

@Serializable data class YearRoute(val year: Int)

@Serializable data object CompilationsRoute

@Serializable data object RandomRoute

/** A music folder; 0 for the top level. */
@Serializable data class FolderRoute(val id: Long = 0, val name: String = "")

@Serializable data object PlaylistsRoute

@Serializable data class PlaylistRoute(val id: Long, val name: String)

/** Favourites; an empty item id for the top level. */
@Serializable data class FavouritesRoute(val itemId: String = "", val title: String = "")

@Serializable data object AppsRoute

@Serializable data class AppMenuRoute(val cmd: String, val name: String)

@Serializable data class PluginMenuRoute(val id: String, val name: String)

@Serializable data class PinnedMenuRoute(val id: Long, val name: String)

@Serializable data object HistoryRoute

@Serializable data class AlbumRoute(val id: Long)

@Serializable data class ArtistRoute(val id: Long, val name: String)

@Serializable data object NowPlayingRoute

@Serializable data object SettingsRoute

@Serializable data object LicencesRoute

@Serializable data object SoundRoute

@Serializable data object DownloadsRoute

@Serializable data object StorageRoute

private data class TopLevel(val route: Any, val label: Int, val icon: ImageVector)

private val TOP_LEVEL = listOf(
    TopLevel(HomeRoute, R.string.nav_home, Icons.Outlined.Home),
    TopLevel(LibraryRoute, R.string.nav_library, Icons.Outlined.LibraryMusic),
    TopLevel(SearchRoute, R.string.nav_search, Icons.Outlined.Search),
)

private val clock: () -> Long = SystemClock::elapsedRealtime

private fun messageText(res: Resources, m: PlayerMessage): String = when (m) {
    is PlayerMessage.Queued -> res.getString(
        when (m.placement) {
            Placement.PLAY_NOW -> R.string.queued_play_now
            Placement.PLAY_NEXT -> R.string.queued_next
            Placement.ADD_TO_QUEUE -> R.string.queued_add
        },
    )
    PlayerMessage.NoPlayer -> res.getString(R.string.no_player_selected)
    is PlayerMessage.Failed -> errorText(res, m.error)
    is PlayerMessage.Removed ->
        if (m.title.isBlank()) res.getString(R.string.queue_removed_track) else res.getString(R.string.queue_removed, m.title.take(80))
    is PlayerMessage.QueueFailed ->
        res.getString(if (m.undo) R.string.queue_undo_failed else R.string.queue_failed) + "\n" + errorWithDetail(res, m.error)
    is PlayerMessage.Moved -> res.getString(R.string.transfer_done, m.targetName)
}

/** The signed-in app: adaptive navigation (bar, rail or drawer by window size). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainScreen(container: AppContainer, signedIn: AuthState.SignedInState) {
    val nav = rememberNavController()
    val nowPlayingHub = (LocalContext.current.applicationContext as? LyrionApp)?.nowPlaying
    // One player controller for every screen, per sign-in. Which player it controls is
    // decided in one place (PlayerChoice, D20); screens only ask it to play things.
    val player = viewModel(key = "player-${signedIn.who.device.id}") {
        PlayerViewModel(
            container.players,
            container.events,
            container.settings,
            localPlayer = container.localPlayer,
            waveforms = WaveformCache(container.analysis),
            nowPlaying = nowPlayingHub,
            clock = clock,
        )
    }
    LifecycleStartEffect(player) {
        player.start()
        onStopOrDispose { player.stop() }
    }
    val playerState by player.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    LaunchedEffect(playerState.local.enabled) {
        if (playerState.local.enabled) startPlaybackService(context)
    }
    // Downloads playing on this phone (they can be played online too).
    val offlineVm = viewModel(key = "offline-player") { OfflinePlayerViewModel(container.offlinePlayer, offlineClock) }
    val offlineUi by offlineVm.state.collectAsStateWithLifecycle()
    LaunchedEffect(offlineUi.state.active) {
        if (offlineUi.state.active) startPlaybackService(context)
    }
    val offlineState by container.offline.state.collectAsStateWithLifecycle()
    val askNotifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
    var askedNotifications by rememberSaveable { mutableStateOf(false) }
    /** Before the first download: the progress notification needs permission (Android 13+). */
    fun beforeDownload() {
        if (askedNotifications || Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return
        askedNotifications = true
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            askNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        }
    }
    val downloads = container.downloads
    val downloadAlbum: (Long) -> Unit = { id ->
        beforeDownload()
        downloads.album(id)
    }
    val downloadPlaylist: (Long) -> Unit = { id ->
        beforeDownload()
        downloads.playlist(id)
    }
    val downloadTrack: (Long) -> Unit = { id ->
        beforeDownload()
        downloads.track(id)
    }
    val scope = rememberCoroutineScope()
    LaunchedEffect(signedIn.who.device.id) { container.libraries.refresh() }
    val libraryId = container.libraries.chosen
    val libraries by container.libraries.available.collectAsStateWithLifecycle()
    val chosenLibrary by libraryId.collectAsStateWithLifecycle(null)

    val snackbar = remember { SnackbarHostState() }
    val res = LocalResources.current
    LaunchedEffect(downloads) {
        downloads.messages.collect { m ->
            snackbar.showSnackbar(
                when (m) {
                    is DownloadMessage.Queued ->
                        if (m.count == 0) {
                            res.getString(R.string.download_nothing)
                        } else {
                            res.getQuantityString(R.plurals.downloads_queued, m.count, m.count)
                        }
                    is DownloadMessage.Failed -> errorText(res, m.error)
                },
                duration = SnackbarDuration.Short,
            )
        }
    }
    LaunchedEffect(player) {
        player.messages.collect { m ->
            val undo = (m as? PlayerMessage.Removed)?.canUndo == true
            val result = snackbar.showSnackbar(
                messageText(res, m),
                actionLabel = if (undo) res.getString(R.string.action_undo) else null,
                duration = SnackbarDuration.Short,
            )
            if (undo && result == SnackbarResult.ActionPerformed) player.undoRemove((m as PlayerMessage.Removed).token)
        }
    }

    // "More…" sheet and "Play on…" sheet.
    var more by remember { mutableStateOf<MoreRequest?>(null) }
    var outputs by rememberSaveable { mutableStateOf(false) }
    val openMore: (MoreRequest) -> Unit = { more = it }
    val openOutputs = {
        player.loadOutputs()
        outputs = true
    }
    fun handleMenuEvent(e: MenuEvent, onClosed: () -> Unit = {}) {
        when (e) {
            is MenuEvent.Done -> scope.launch { snackbar.showSnackbar(res.getString(R.string.menu_done, e.label)) }
            MenuEvent.NowPlaying -> {
                onClosed()
                nav.navigate(NowPlayingRoute)
            }
            is MenuEvent.Failed -> scope.launch { snackbar.showSnackbar(errorText(res, e.error)) }
            MenuEvent.NoPlayer -> scope.launch { snackbar.showSnackbar(res.getString(R.string.no_player_selected)) }
            MenuEvent.Closed -> onClosed()
            is MenuEvent.Saved -> scope.launch {
                snackbar.showSnackbar(res.getString(if (e.station) R.string.menu_saved_station else R.string.menu_pinned))
            }
            is MenuEvent.CantSave -> scope.launch {
                snackbar.showSnackbar(res.getString(R.string.menu_cant_save, errorText(res, e.error)))
            }
        }
    }

    // A station started from Home keeps going: each status update may add another batch.
    LaunchedEffect(playerState.selectedId, playerState.status, playerState.queue) {
        val id = playerState.selectedId ?: return@LaunchedEffect
        val status = playerState.status ?: return@LaunchedEffect
        val update = container.me.onStatus(
            id,
            status.queueLength.toInt(),
            status.queueIndex?.toInt() ?: 0,
            status.current?.id,
            playerState.queue.mapNotNull { it.track.id },
        )
        if (update is RunnerUpdate.Stopped) {
            snackbar.showSnackbar(res.getString(R.string.station_broken, update.reason))
        }
    }

    // "Start radio" and "More like this" (A6): what the server offers for an item.
    fun discoveryText(e: DiscoveryEvent): String = when (e) {
        is DiscoveryEvent.RadioStarted ->
            res.getString(if (e.endless) R.string.radio_started_endless else R.string.radio_started)
        DiscoveryEvent.Played -> res.getString(R.string.similar_playing)
        is DiscoveryEvent.Failed -> errorText(res, e.error)
        DiscoveryEvent.NoPlayer -> res.getString(R.string.no_player_selected)
    }

    // Album, artist and playlist pages: full screen, or beside their list on wide windows.
    val albumPage: @Composable (Long) -> Unit = { albumId ->
        val vm = viewModel(key = "album-$albumId") { AlbumViewModel(container.library, albumId) }
        val ui by vm.state.collectAsStateWithLifecycle()
        val discovery = remember(albumId) {
            DiscoveryController(scope, container.discovery, container.menus, player::target, withShelf = true)
        }
        val found by discovery.state.collectAsStateWithLifecycle()
        LaunchedEffect(discovery, playerState.selectedId) {
            discovery.load(io.github.jamezuk.lyrionanywhere.core.RadioSeed.Album(albumId))
        }
        LaunchedEffect(discovery) {
            discovery.events.collect { e -> snackbar.showSnackbar(discoveryText(e)) }
        }
        AlbumScreen(
            state = ui,
            actions = AlbumActions(
                onRetry = vm::load,
                onPlay = { placement, index -> player.enqueue(Selection.Album(albumId), placement, index) },
                onShuffle = { player.playShuffled(Selection.Album(albumId)) },
                onPlayTrack = { track ->
                    val tracks = (ui as? Loadable.Ready)?.value?.tracks.orEmpty()
                    val index = tracks.indexOf(track).takeIf { it >= 0 }
                    player.enqueue(Selection.Album(albumId), Placement.PLAY_NOW, index)
                },
                onArtist = { id, name -> nav.navigate(ArtistRoute(id, name)) },
                onMore = openMore,
                onDownload = { downloadAlbum(albumId) },
                downloaded = offlineState.downloadedIds,
                discovery = found,
                onStartRadio = discovery::startRadio,
                onSimilar = discovery::play,
            ),
        )
    }
    val artistPage: @Composable (Long, String) -> Unit = { artistId, name ->
        val vm = viewModel(key = "artist-$artistId") { ArtistViewModel(container.library, artistId) }
        val ui by vm.state.collectAsStateWithLifecycle()
        ArtistScreen(
            id = artistId,
            name = name,
            state = ui,
            actions = ArtistActions(
                onRetry = vm::load,
                onAlbum = { nav.navigate(AlbumRoute(it.id)) },
                onPlay = { player.enqueue(Selection.Artist(artistId), it) },
                onShuffle = { player.playShuffled(Selection.Artist(artistId)) },
                onMore = openMore,
            ),
        )
    }
    val playlistPage: @Composable (Long, String) -> Unit = { playlistId, name ->
        val vm = viewModel(key = "playlist-$playlistId") { PlaylistViewModel(container.library, playlistId) }
        val ui by vm.state.collectAsStateWithLifecycle()
        val sel = Selection.Playlist(playlistId)
        PlaylistScreen(
            ui,
            onRetry = vm::reload,
            onPlay = { placement, index -> player.enqueue(sel, placement, index) },
            onShuffle = { player.playShuffled(sel) },
            onMorePlaylist = {
                openMore(More.playlist(io.github.jamezuk.lyrionanywhere.core.SavedPlaylist(playlistId, name)))
            },
            onMore = openMore,
            onDownload = { downloadPlaylist(playlistId) },
            downloaded = offlineState.downloadedIds,
        )
    }

    val backStack by nav.currentBackStackEntryAsState()
    val destination = backStack?.destination
    // Which navigation the window gets: a bottom bar on phones, a rail or drawer on larger
    // screens. The bottom bar handles the gesture/navigation inset itself; otherwise the
    // content (and the mini player) must keep clear of it.
    val suiteType = NavigationSuiteScaffoldDefaults.calculateFromAdaptiveInfo(currentWindowAdaptiveInfo())
    val bottomBarNav = suiteType == NavigationSuiteType.NavigationBar
    NavigationSuiteScaffold(
        layoutType = suiteType,
        navigationSuiteItems = {
            TOP_LEVEL.forEach { item ->
                val selected = destination?.hierarchy?.any { it.hasRoute(item.route::class) } == true
                item(
                    selected = selected,
                    onClick = { nav.navigateTopLevel(item.route) },
                    icon = { Icon(item.icon, contentDescription = null) },
                    label = { Text(stringResource(item.label)) },
                )
            }
        },
    ) {
        Scaffold(
            snackbarHost = { SnackbarHost(snackbar) },
            // Each screen's top app bar handles the status bar, and the navigation bar (when
            // there is one) the bottom: counting them here as well doubled the empty space.
            contentWindowInsets = if (bottomBarNav) {
                WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal)
            } else {
                WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Bottom)
            },
            bottomBar = {
              Column(if (bottomBarNav) Modifier else Modifier.navigationBarsPadding()) {
                val onNowPlaying = destination?.hasRoute(NowPlayingRoute::class) == true ||
                    destination?.hasRoute(OfflineNowPlayingRoute::class) == true
                if (!onNowPlaying && offlineUi.state.active) {
                    // Downloads have this phone: their own mini player.
                    OfflineMiniPlayer(
                        offlineUi,
                        onOpen = { nav.navigate(OfflineNowPlayingRoute) },
                        onToggle = offlineVm::toggle,
                        onNext = offlineVm::next,
                    )
                } else if (!onNowPlaying) {
                    MiniPlayer(
                        state = playerState,
                        onOpen = { nav.navigate(NowPlayingRoute) },
                        onTogglePlay = player::togglePlay,
                        onNext = player::next,
                        onOutputs = openOutputs,
                        onBackToPhone = player::backToPhone,
                        onBringToPhone = player::bringToPhone,
                        onDismissTransfer = player::dismissTransfer,
                    )
                }
              }
            },
        ) { padding ->
            val openAlbum: (Long) -> Unit = { nav.navigate(AlbumRoute(it)) }
            BoxWithConstraints(Modifier.padding(padding)) {
            val layout = WindowLayout.of(maxWidth.value, maxHeight.value)
            CompositionLocalProvider(LocalWindowLayout provides layout) {
            NavHost(nav, startDestination = HomeRoute) {
                composable<HomeRoute> {
                    val vm = viewModel { HomeFeedViewModel(container.library, libraryId, container.me, player::target) }
                    val feed by vm.state.collectAsStateWithLifecycle()
                    LaunchedEffect(vm) {
                        vm.messages.collect { m ->
                            snackbar.showSnackbar(
                                when (m) {
                                    is HomeMessage.Failed -> errorText(res, m.error)
                                    is HomeMessage.StationBroken -> res.getString(R.string.station_broken, m.reason)
                                    HomeMessage.NoPlayer -> res.getString(R.string.station_no_player)
                                    is HomeMessage.StationStarted -> res.getString(R.string.station_started, m.name)
                                },
                            )
                        }
                    }
                    Screen(
                        title = stringResource(R.string.nav_home),
                        actions = {
                            IconButton(onClick = { nav.navigate(SettingsRoute) }) {
                                Icon(Icons.Outlined.Settings, contentDescription = stringResource(R.string.title_settings))
                            }
                        },
                    ) {
                        HomeScreen(
                            feed = feed,
                            actions = HomeActions(
                                onAlbum = { openAlbum(it.id) },
                                onMore = openMore,
                                onSeeAll = { d -> if (d == null) nav.navigate(NewMusicRoute) else nav.openLibrary(d) },
                                onFavourite = { f ->
                                    if (f.isFolder) nav.navigate(FavouritesRoute(f.id, f.name)) else player.playFavourite(f.id, Placement.PLAY_NOW)
                                },
                                onPlaylist = { nav.navigate(PlaylistRoute(it.id, it.name)) },
                                onReshuffle = vm::reshuffle,
                                onRetry = vm::refresh,
                                onStation = vm::startStation,
                                onCreateStation = vm::createStation,
                                onDeleteStation = vm::deleteStation,
                                onMenu = { nav.navigate(PinnedMenuRoute(it.id, it.name)) },
                                onGenre = { id, name -> nav.navigate(GenreRoute(id, name)) },
                                onEdit = vm::edit,
                                genreChoices = vm::genreChoices,
                            ),
                            edit = HomeEditActions(
                                onMove = vm::move,
                                onHidden = vm::setHidden,
                                onRemove = vm::remove,
                                onAdd = vm::add,
                                onReset = vm::resetDraft,
                                onSave = vm::save,
                                onCancel = vm::cancelEdit,
                            ),
                        )
                    }
                }
                composable<LibraryRoute> {
                    Screen(title = stringResource(R.string.nav_library)) {
                        LibraryScreen(
                            libraries = libraries,
                            chosenLibrary = chosenLibrary,
                            onChooseLibrary = { id -> scope.launch { container.libraries.choose(id) } },
                            onNewMusic = { nav.navigate(NewMusicRoute) },
                            onOpen = { nav.openLibrary(it) },
                        )
                    }
                }
                composable<SearchRoute> {
                    val vm = viewModel { SearchViewModel(container.library) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.nav_search)) {
                        val focusManager = LocalFocusManager.current
                        Column {
                            OutlinedTextField(
                                value = ui.query,
                                onValueChange = vm::onQueryChange,
                                label = { Text(stringResource(R.string.search_field)) },
                                singleLine = true,
                                leadingIcon = { Icon(Icons.Outlined.Search, contentDescription = null) },
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                                // Results come as you type: Search closes the keyboard to show them.
                                keyboardActions = KeyboardActions(onSearch = { focusManager.clearFocus() }),
                                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                            )
                            SearchResultsList(
                                ui,
                                onRetry = vm::retry,
                                onArtist = { nav.navigate(ArtistRoute(it.id, it.name)) },
                                onAlbum = { openAlbum(it.id) },
                                onTrack = { t -> t.id?.let { player.enqueue(Selection.Track(it), Placement.PLAY_NOW) } },
                                onMore = openMore,
                            )
                        }
                    }
                }
                composable<NewMusicRoute> {
                    val vm = viewModel { AlbumsViewModel(container.library, sort = AlbumSort.NEWEST, libraryId = libraryId) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.new_music), onBack = { nav.popBackStack() }) {
                        AlbumGrid(ui, onAlbum = { openAlbum(it.id) }, onLoadMore = vm::loadMore, onMore = openMore)
                    }
                }
                composable<AlbumsRoute> {
                    val vm = viewModel { AlbumsViewModel(container.library, libraryId = libraryId) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val sort by vm.sort.collectAsStateWithLifecycle()
                    var chosen by rememberSaveable { mutableStateOf<Long?>(null) }
                    Screen(
                        title = stringResource(R.string.lib_albums),
                        onBack = { nav.popBackStack() },
                        actions = { AlbumSortMenu(sort, vm::setSort) },
                    ) {
                        if (LocalWindowLayout.current.listDetail) {
                            ListDetail(
                                list = { AlbumGrid(ui, onAlbum = { chosen = it.id }, onLoadMore = vm::loadMore, onMore = openMore) },
                                detail = chosen?.let { id -> { albumPage(id) } },
                                placeholder = { EmptyState(stringResource(R.string.choose_album)) },
                            )
                        } else {
                            AlbumGrid(ui, onAlbum = { openAlbum(it.id) }, onLoadMore = vm::loadMore, onMore = openMore)
                        }
                    }
                }
                composable<CompilationsRoute> {
                    val vm = viewModel {
                        AlbumsViewModel(container.library, LibraryFilter(compilation = true), libraryId = libraryId)
                    }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val sort by vm.sort.collectAsStateWithLifecycle()
                    Screen(
                        title = stringResource(R.string.lib_compilations),
                        onBack = { nav.popBackStack() },
                        actions = { AlbumSortMenu(sort, vm::setSort) },
                    ) {
                        AlbumGrid(
                            ui,
                            onAlbum = { openAlbum(it.id) },
                            onLoadMore = vm::loadMore,
                            onMore = openMore,
                            empty = stringResource(R.string.no_compilations),
                        )
                    }
                }
                composable<RandomRoute> {
                    val vm = viewModel {
                        AlbumsViewModel(container.library, sort = AlbumSort.RANDOM, libraryId = libraryId, batch = RANDOM_BATCH)
                    }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(
                        title = stringResource(R.string.lib_random),
                        onBack = { nav.popBackStack() },
                        actions = {
                            IconButton(onClick = vm::reload) {
                                Icon(Icons.Outlined.Shuffle, contentDescription = stringResource(R.string.shuffle_again))
                            }
                        },
                    ) {
                        AlbumGrid(ui, onAlbum = { openAlbum(it.id) }, onLoadMore = vm::loadMore, onMore = openMore)
                    }
                }
                composable<ArtistsRoute> {
                    val vm = viewModel { ArtistsViewModel(container.library, libraryId = libraryId) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val views by vm.views.collectAsStateWithLifecycle()
                    val view by vm.view.collectAsStateWithLifecycle()
                    var chosen by rememberSaveable { mutableStateOf<Pair<Long, String>?>(null) }
                    Screen(title = stringResource(R.string.lib_artists), onBack = { nav.popBackStack() }) {
                        val list = @Composable { onArtist: (io.github.jamezuk.lyrionanywhere.core.Artist) -> Unit ->
                            ArtistList(
                                ui,
                                onArtist = onArtist,
                                onLoadMore = vm::loadMore,
                                onMore = openMore,
                                header = { ArtistViewChips(views, view, vm::setView) },
                            )
                        }
                        if (LocalWindowLayout.current.listDetail) {
                            ListDetail(
                                list = { list { chosen = it.id to it.name } },
                                detail = chosen?.let { (id, name) -> { artistPage(id, name) } },
                                placeholder = { EmptyState(stringResource(R.string.choose_artist)) },
                            )
                        } else {
                            list { nav.navigate(ArtistRoute(it.id, it.name)) }
                        }
                    }
                }
                composable<GenresRoute> {
                    val vm = viewModel { GenresViewModel(container.library, libraryId) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val query by vm.query.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.lib_genres), onBack = { nav.popBackStack() }) {
                        GenresScreen(
                            ui,
                            query = query,
                            onQuery = vm::onQueryChange,
                            onGenre = { nav.navigate(GenreRoute(it.id, it.name)) },
                            onLoadMore = vm::loadMore,
                            onMore = openMore,
                        )
                    }
                }
                composable<GenreRoute> { entry ->
                    val route = entry.toRoute<GenreRoute>()
                    val nameVm = viewModel { GenreNameViewModel(container.library, route.id) }
                    val name by nameVm.state.collectAsStateWithLifecycle()
                    val title = (name as? Loadable.Ready)?.value ?: route.name
                    val filter = LibraryFilter(genreId = route.id)
                    val albums = viewModel { AlbumsViewModel(container.library, filter, libraryId = libraryId) }
                    val artists = viewModel { ArtistsViewModel(container.library, filter, libraryId) }
                    var tab by rememberSaveable { mutableIntStateOf(0) }
                    val sort by albums.sort.collectAsStateWithLifecycle()
                    Screen(
                        title = title,
                        onBack = { nav.popBackStack() },
                        actions = { if (tab == 0) AlbumSortMenu(sort, albums::setSort) },
                    ) {
                        val header = @Composable {
                            CollectionHeader(
                                title = title,
                                onPlay = { player.enqueue(Selection.Genre(route.id), it) },
                                onMore = { openMore(More.genre(route.id, title)) },
                                tabs = listOf(stringResource(R.string.tab_albums), stringResource(R.string.tab_artists)),
                                tab = tab,
                                onTab = { tab = it },
                            )
                        }
                        if (tab == 0) {
                            val ui by albums.state.collectAsStateWithLifecycle()
                            AlbumGrid(
                                ui,
                                onAlbum = { openAlbum(it.id) },
                                onLoadMore = albums::loadMore,
                                onMore = openMore,
                                empty = stringResource(R.string.no_albums_genre),
                                header = header,
                            )
                        } else {
                            val ui by artists.state.collectAsStateWithLifecycle()
                            ArtistList(
                                ui,
                                onArtist = { nav.navigate(ArtistRoute(it.id, it.name)) },
                                onLoadMore = artists::loadMore,
                                onMore = openMore,
                                header = header,
                            )
                        }
                    }
                }
                composable<YearsRoute> {
                    val vm = viewModel { YearsViewModel(container.library, libraryId) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.lib_years), onBack = { nav.popBackStack() }) {
                        YearsScreen(ui, onYear = { nav.navigate(YearRoute(it.toInt())) }, onLoadMore = vm::loadMore, onMore = openMore)
                    }
                }
                composable<YearRoute> { entry ->
                    val route = entry.toRoute<YearRoute>()
                    val year = route.year.coerceIn(0, 9999).toUInt()
                    val title = if (year == 0u) stringResource(R.string.no_year) else year.toString()
                    val vm = viewModel {
                        AlbumsViewModel(container.library, LibraryFilter(year = year), libraryId = libraryId)
                    }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val sort by vm.sort.collectAsStateWithLifecycle()
                    Screen(title = title, onBack = { nav.popBackStack() }, actions = { AlbumSortMenu(sort, vm::setSort) }) {
                        AlbumGrid(
                            ui,
                            onAlbum = { openAlbum(it.id) },
                            onLoadMore = vm::loadMore,
                            onMore = openMore,
                            empty = stringResource(R.string.no_albums_year),
                            header = {
                                CollectionHeader(
                                    title = title,
                                    onPlay = { player.enqueue(Selection.Year(year), it) },
                                    onMore = { openMore(More.year(year, title)) },
                                )
                            },
                        )
                    }
                }
                composable<FolderRoute> { entry ->
                    val route = entry.toRoute<FolderRoute>()
                    val folderId = route.id.takeIf { it > 0 }
                    val vm = viewModel { FolderViewModel(container.library, folderId) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(
                        title = route.name.ifEmpty { stringResource(R.string.lib_folder) },
                        onBack = { nav.popBackStack() },
                    ) {
                        FolderScreen(
                            ui,
                            isTop = folderId == null,
                            onFolder = { nav.navigate(FolderRoute(it.id, it.name)) },
                            onPlay = { f, placement ->
                                val sel = if (f.kind == io.github.jamezuk.lyrionanywhere.core.FolderKind.TRACK) {
                                    Selection.Track(f.id)
                                } else {
                                    Selection.Folder(f.id)
                                }
                                player.enqueue(sel, placement)
                            },
                            onPlayAll = folderId?.let { id -> { p: Placement -> player.enqueue(Selection.Folder(id), p) } },
                            onLoadMore = vm::loadMore,
                            onMore = openMore,
                        )
                    }
                }
                composable<PlaylistsRoute> {
                    val vm = viewModel { PlaylistsViewModel(container.library) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    var chosen by rememberSaveable { mutableStateOf<Pair<Long, String>?>(null) }
                    Screen(title = stringResource(R.string.lib_playlists), onBack = { nav.popBackStack() }) {
                        if (LocalWindowLayout.current.listDetail) {
                            ListDetail(
                                list = {
                                    PlaylistsScreen(ui, onPlaylist = { chosen = it.id to it.name }, onLoadMore = vm::loadMore, onMore = openMore)
                                },
                                detail = chosen?.let { (id, name) -> { playlistPage(id, name) } },
                                placeholder = { EmptyState(stringResource(R.string.choose_playlist)) },
                            )
                        } else {
                            PlaylistsScreen(
                                ui,
                                onPlaylist = { nav.navigate(PlaylistRoute(it.id, it.name)) },
                                onLoadMore = vm::loadMore,
                                onMore = openMore,
                            )
                        }
                    }
                }
                composable<PlaylistRoute> { entry ->
                    val route = entry.toRoute<PlaylistRoute>()
                    Screen(title = route.name, onBack = { nav.popBackStack() }) {
                        playlistPage(route.id, route.name)
                    }
                }
                composable<FavouritesRoute> { entry ->
                    val route = entry.toRoute<FavouritesRoute>()
                    val vm = viewModel { FavouritesViewModel(container.library, route.itemId.ifEmpty { null }) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val title = route.title.ifEmpty { stringResource(R.string.lib_favourites) }
                    Screen(title = title, onBack = { nav.popBackStack() }) {
                        FavouritesScreen(
                            ui,
                            onRetry = vm::reload,
                            onFolder = { nav.navigate(FavouritesRoute(it.id, it.name)) },
                            onPlay = { f, placement -> player.playFavourite(f.id, placement) },
                        )
                    }
                }
                composable<AppsRoute> {
                    val vm = viewModel { AppsViewModel(container.menus, player::target, container.suitability) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.lib_apps), onBack = { nav.popBackStack() }) {
                        AppsScreen(
                            ui,
                            onOpen = { tile ->
                                val (kind, id) = tile.key.split(':', limit = 2)
                                if (kind == "app") nav.navigate(AppMenuRoute(id, tile.name)) else nav.navigate(PluginMenuRoute(id, tile.name))
                            },
                            onRetry = vm::load,
                            onToggleShowAll = vm::toggleShowAll,
                        )
                    }
                }
                composable<AppMenuRoute> { entry ->
                    val route = entry.toRoute<AppMenuRoute>()
                    val vm = viewModel { MenuViewModel(container.menus, container.me, player::target) }
                    LaunchedEffect(route) { vm.openApp(route.cmd, route.name) }
                    MenuRouteScreen(route.name, vm, onBack = { nav.popBackStack() }, onEvent = { handleMenuEvent(it) { nav.popBackStack() } })
                }
                composable<PluginMenuRoute> { entry ->
                    val route = entry.toRoute<PluginMenuRoute>()
                    val vm = viewModel { MenuViewModel(container.menus, container.me, player::target) }
                    LaunchedEffect(route, playerState.selectedId) { vm.openPlugin(route.id, playerState.selectedId) }
                    MenuRouteScreen(route.name, vm, onBack = { nav.popBackStack() }, onEvent = { handleMenuEvent(it) { nav.popBackStack() } })
                }
                composable<PinnedMenuRoute> { entry ->
                    val route = entry.toRoute<PinnedMenuRoute>()
                    val vm = viewModel { MenuViewModel(container.menus, container.me, player::target) }
                    LaunchedEffect(route, playerState.selectedId) { vm.openPinned(route.id, route.name, playerState.selectedId) }
                    MenuRouteScreen(route.name, vm, onBack = { nav.popBackStack() }, onEvent = { handleMenuEvent(it) { nav.popBackStack() } })
                }
                composable<HistoryRoute> {
                    val vm = viewModel { HistoryViewModel(container.me) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.section_history), onBack = { nav.popBackStack() }) {
                        HistoryScreen(ui, onMore = vm::loadMore, onRetry = vm::load)
                    }
                }
                composable<AlbumRoute> { entry ->
                    val route = entry.toRoute<AlbumRoute>()
                    Screen(title = stringResource(R.string.title_album), onBack = { nav.popBackStack() }) {
                        albumPage(route.id)
                    }
                }
                composable<ArtistRoute> { entry ->
                    val route = entry.toRoute<ArtistRoute>()
                    Screen(title = route.name, onBack = { nav.popBackStack() }) {
                        artistPage(route.id, route.name)
                    }
                }
                composable<NowPlayingRoute> {
                    LaunchedEffect(playerState.selectedId, playerState.status?.queueIndex, playerState.status?.queueLength) {
                        player.loadQueue()
                    }
                    Screen(title = stringResource(R.string.title_now_playing), onBack = { nav.popBackStack() }) {
                        NowPlayingScreen(
                            state = playerState,
                            clock = clock,
                            twoPane = LocalWindowLayout.current.nowPlayingTwoPane,
                            actions = NowPlayingActions(
                                onTogglePlay = player::togglePlay,
                                onNext = player::next,
                                onPrevious = player::previous,
                                onSeek = player::seek,
                                onVolume = player::setVolume,
                                onShuffle = player::cycleShuffle,
                                onRepeat = player::cycleRepeat,
                                onPower = player::setPower,
                                onJump = player::jumpTo,
                                onRetry = player::refresh,
                                onOutputs = openOutputs,
                                onBackToPhone = player::backToPhone,
                                onMore = { t -> More.track(t)?.let(openMore) },
                                onRemove = player::removeFromQueue,
                                onMove = player::moveInQueue,
                                onClearQueue = player::clearQueue,
                                onBringToPhone = player::bringToPhone,
                                onDismissTransfer = player::dismissTransfer,
                                onClose = { nav.popBackStack() },
                            ),
                        )
                    }
                }
                composable<DownloadsRoute> {
                    val vm = viewModel { DownloadsViewModel(container.offline) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.title_downloads), onBack = { nav.popBackStack() }) {
                        DownloadsScreen(
                            ui,
                            DownloadsActions(
                                onPause = vm::pause,
                                onResume = vm::resume,
                                onCancel = vm::cancel,
                                onClearFinished = vm::clearFinished,
                                onAlbum = { nav.navigate(OfflineAlbumRoute(it)) },
                                onPlayAlbum = { ref ->
                                    val tracks = OfflineAlbumViewModel.albumOf(offlineState, ref).tracks
                                    if (tracks.isNotEmpty()) container.offlinePlayer.play(tracks, 0, false)
                                },
                                onRemoveAlbum = vm::removeAlbum,
                                onRemoveAll = vm::removeAll,
                                onStorage = { nav.navigate(StorageRoute) },
                            ),
                        )
                    }
                }
                composable<StorageRoute> {
                    val vm = viewModel { StorageSettingsViewModel(container.offline) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val refreshing by vm.refreshing.collectAsStateWithLifecycle()
                    val error by vm.error.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.title_storage), onBack = { nav.popBackStack() }) {
                        StorageSettingsScreen(
                            ui,
                            refreshing,
                            error,
                            onChange = vm::update,
                            onRefreshSmart = vm::refreshSmart,
                            onDownloads = { nav.navigate(DownloadsRoute) },
                        )
                    }
                }
                composable<OfflineAlbumRoute> { entry ->
                    val ref = entry.toRoute<OfflineAlbumRoute>().ref
                    val vm = viewModel(key = "offline-album-$ref") {
                        OfflineAlbumViewModel(container.offline, container.offlinePlayer, ref)
                    }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = ui.album?.title.orEmpty(), onBack = { nav.popBackStack() }) {
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
                    Screen(title = stringResource(R.string.title_now_playing), onBack = { nav.popBackStack() }) {
                        OfflineNowPlayingScreen(offlineUi, offlineClock, offlineNowPlayingActions(offlineVm))
                    }
                }
                composable<SettingsRoute> {
                    val vm = viewModel { SettingsViewModel(container.account, container.localPlayer, container.offline) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val historyVm = viewModel { HistoryViewModel(container.me) }
                    val historyUi by historyVm.state.collectAsStateWithLifecycle()
                    LaunchedEffect(historyVm) {
                        historyVm.messages.collect { m ->
                            snackbar.showSnackbar(
                                when (m) {
                                    HistoryMessage.Cleared -> res.getString(R.string.history_cleared)
                                    is HistoryMessage.Failed -> errorText(res, m.error)
                                },
                            )
                        }
                    }
                    Screen(title = stringResource(R.string.title_settings), onBack = { nav.popBackStack() }) {
                        SettingsScreen(
                            state = ui,
                            appVersion = BuildConfig.VERSION_NAME,
                            libraryVersion = remember { libraryVersion() },
                            onSignOut = vm::signOut,
                            onRename = vm::rename,
                            onLicences = { nav.navigate(LicencesRoute) },
                            onSound = { nav.navigate(SoundRoute) },
                            onDownloads = { nav.navigate(DownloadsRoute) },
                            onStorage = { nav.navigate(StorageRoute) },
                            defaultPlayer = { DefaultPlayerSetting(playerState, player::setDefaultPlayer) },
                            history = {
                                HistorySettings(
                                    historyUi,
                                    onEnabled = historyVm::setEnabled,
                                    onView = { nav.navigate(HistoryRoute) },
                                    onClear = historyVm::clear,
                                )
                            },
                        )
                    }
                }
                composable<SoundRoute> {
                    val vm = viewModel { SoundViewModel(container.localPlayer) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val actions = remember(vm) { vm.actions() }
                    Screen(title = stringResource(R.string.title_sound), onBack = { nav.popBackStack() }) {
                        SoundScreen(state = ui, actions = actions)
                    }
                }
                composable<LicencesRoute> {
                    Screen(title = stringResource(R.string.title_licences), onBack = { nav.popBackStack() }) {
                        LicencesScreen()
                    }
                }
            }
            }
            }
        }
    }

    // "More…": quick play actions, then LMS's own context menu for the item.
    more?.let { request ->
        val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = false)
        val navigator = remember(request) { MenuNavigator(scope, container.menus, player::target).also { it.saver = container.me } }
        val seed = when (request.kind) {
            io.github.jamezuk.lyrionanywhere.core.InfoKind.TRACK -> io.github.jamezuk.lyrionanywhere.core.RadioSeed.Track(request.id)
            io.github.jamezuk.lyrionanywhere.core.InfoKind.ALBUM -> io.github.jamezuk.lyrionanywhere.core.RadioSeed.Album(request.id)
            io.github.jamezuk.lyrionanywhere.core.InfoKind.ARTIST -> io.github.jamezuk.lyrionanywhere.core.RadioSeed.Artist(request.id)
            else -> null
        }
        val discovery = remember(request) {
            DiscoveryController(
                scope,
                container.discovery,
                container.menus,
                player::target,
                withShelf = request.kind == io.github.jamezuk.lyrionanywhere.core.InfoKind.TRACK,
            )
        }
        val found by discovery.state.collectAsStateWithLifecycle()
        LaunchedEffect(discovery) {
            seed?.let(discovery::load)
            discovery.events.collect { e ->
                if (e is DiscoveryEvent.RadioStarted || e is DiscoveryEvent.Played) more = null
                snackbar.showSnackbar(discoveryText(e))
            }
        }
        LaunchedEffect(request) {
            when (val a = container.menus.infoMenu(request.kind, request.id)) {
                is ApiResult.Ok -> navigator.start(request.target.title, a.value)
                is ApiResult.Err -> navigator.startWith(request.target.title, emptyList())
            }
        }
        LaunchedEffect(navigator) { navigator.events.collect { e -> handleMenuEvent(e) { more = null } } }
        ModalBottomSheet(onDismissRequest = { more = null }, sheetState = sheetState) {
            ItemSheetContent(
                target = request.target.copy(canPlay = request.selection != null),
                navigator = navigator,
                onPlay = { placement ->
                    request.selection?.let { player.enqueue(it, placement) }
                    more = null
                },
                onClose = { more = null },
                onDownload = when (request.kind) {
                    io.github.jamezuk.lyrionanywhere.core.InfoKind.TRACK -> ({
                        downloadTrack(request.id)
                        more = null
                    })
                    io.github.jamezuk.lyrionanywhere.core.InfoKind.ALBUM -> ({
                        downloadAlbum(request.id)
                        more = null
                    })
                    io.github.jamezuk.lyrionanywhere.core.InfoKind.PLAYLIST -> ({
                        downloadPlaylist(request.id)
                        more = null
                    })
                    else -> null
                },
                discovery = found.takeIf { seed != null },
                onStartRadio = discovery::startRadio,
                onSimilar = discovery::play,
            )
        }
    }

    // "Play on…" (D20): this phone, other players and sync groups.
    if (outputs) {
        ModalBottomSheet(onDismissRequest = { outputs = false }) {
            Column(Modifier.verticalScroll(rememberScrollState()).padding(bottom = 24.dp)) {
                PlayOnList(
                    playerState,
                    PlayOnActions(
                        onControl = { id ->
                            player.select(id)
                            outputs = false
                        },
                        onBackToPhone = {
                            player.backToPhone()
                            outputs = false
                        },
                        onMoveHere = player::moveTo,
                        onBringToPhone = player::bringToPhone,
                        onSync = player::setSynced,
                        onDismissTransfer = player::dismissTransfer,
                    ),
                )
            }
        }
        // A move that worked closes the sheet (its progress and errors show in it meanwhile).
        var wasMoving by remember { mutableStateOf(false) }
        LaunchedEffect(playerState.transfer) {
            val moving = playerState.transfer is TransferState.Running
            if (wasMoving && playerState.transfer == TransferState.Idle) outputs = false
            wasMoving = moving
        }
    }
}

/** Random albums come as one batch: paging a random order would only repeat albums. */
private const val RANDOM_BATCH = 60

@Composable
private fun MenuRouteScreen(title: String, vm: MenuViewModel, onBack: () -> Unit, onEvent: (MenuEvent) -> Unit) {
    val missing by vm.missing.collectAsStateWithLifecycle()
    LaunchedEffect(vm) { vm.navigator.events.collect(onEvent) }
    Screen(title = title, onBack = { if (!vm.navigator.back()) onBack() }) {
        if (missing) EmptyState(stringResource(R.string.menu_missing)) else MenuBrowser(vm.navigator)
    }
}

private fun NavHostController.openLibrary(d: LibraryDestination) = navigate(
    when (d) {
        LibraryDestination.ALBUMS -> AlbumsRoute
        LibraryDestination.ARTISTS -> ArtistsRoute
        LibraryDestination.GENRES -> GenresRoute
        LibraryDestination.YEARS -> YearsRoute
        LibraryDestination.COMPILATIONS -> CompilationsRoute
        LibraryDestination.RANDOM -> RandomRoute
        LibraryDestination.FOLDER -> FolderRoute()
        LibraryDestination.PLAYLISTS -> PlaylistsRoute
        LibraryDestination.FAVOURITES -> FavouritesRoute()
        LibraryDestination.APPS -> AppsRoute
        LibraryDestination.DOWNLOADS -> DownloadsRoute
    },
)

/** Start the media session service (allowed while the app is in the foreground). */
private fun startPlaybackService(context: Context) {
    try {
        context.startService(Intent(context, PlaybackService::class.java))
    } catch (_: IllegalStateException) {
        // Not in the foreground after all; it starts the next time the app opens.
    }
}

private fun NavHostController.navigateTopLevel(route: Any) {
    navigate(route) {
        popUpTo(graph.findStartDestination().id) { saveState = true }
        launchSingleTop = true
        restoreState = true
    }
}

/** A screen with its own top bar (and back arrow on detail screens). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun Screen(
    title: String,
    onBack: (() -> Unit)? = null,
    actions: @Composable () -> Unit = {},
    content: @Composable () -> Unit,
) {
    Column(Modifier.fillMaxSize()) {
        TopAppBar(
            title = {
                Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.semantics { heading() })
            },
            navigationIcon = {
                if (onBack != null) {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Outlined.ArrowBack, contentDescription = stringResource(R.string.action_back))
                    }
                }
            },
            actions = { actions() },
        )
        content()
    }
}
