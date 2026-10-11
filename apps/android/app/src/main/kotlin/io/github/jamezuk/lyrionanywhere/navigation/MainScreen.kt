package io.github.jamezuk.lyrionanywhere.navigation

import android.content.Context
import android.content.Intent
import android.os.SystemClock
import androidx.compose.ui.platform.LocalContext
import io.github.jamezuk.lyrionanywhere.playback.PlaybackService
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.outlined.Album
import androidx.compose.material.icons.outlined.Home
import androidx.compose.material.icons.outlined.Person
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalResources
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
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
import io.github.jamezuk.lyrionanywhere.AppContainer
import io.github.jamezuk.lyrionanywhere.BuildConfig
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.libraryVersion
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.WaveformCache
import io.github.jamezuk.lyrionanywhere.ui.R
import io.github.jamezuk.lyrionanywhere.ui.components.errorText
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumGrid
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumScreen
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumSortMenu
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistList
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistScreen
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.SearchResultsList
import io.github.jamezuk.lyrionanywhere.ui.library.SearchViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.player.MiniPlayer
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingActions
import io.github.jamezuk.lyrionanywhere.ui.player.NowPlayingScreen
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.LicencesScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.actions
import kotlinx.serialization.Serializable

@Serializable
data object HomeRoute

@Serializable
data object AlbumsRoute

@Serializable
data object ArtistsRoute

@Serializable
data object SearchRoute

@Serializable
data class AlbumRoute(val id: Long)

@Serializable
data class ArtistRoute(val id: Long, val name: String)

@Serializable
data object NowPlayingRoute

@Serializable
data object SettingsRoute

@Serializable
data object LicencesRoute

@Serializable
data object SoundRoute

private data class TopLevel(val route: Any, val label: Int, val icon: ImageVector)

private val TOP_LEVEL = listOf(
    TopLevel(HomeRoute, R.string.nav_home, Icons.Outlined.Home),
    TopLevel(AlbumsRoute, R.string.nav_albums, Icons.Outlined.Album),
    TopLevel(ArtistsRoute, R.string.nav_artists, Icons.Outlined.Person),
    TopLevel(SearchRoute, R.string.nav_search, Icons.Outlined.Search),
)

private val clock: () -> Long = SystemClock::elapsedRealtime

/** The signed-in app: adaptive navigation (bar, rail or drawer by window size). */
@Composable
fun MainScreen(container: AppContainer, signedIn: AuthState.SignedInState) {
    val nav = rememberNavController()
    // One player controller for every screen, per sign-in.
    val player = viewModel(key = "player-${signedIn.who.device.id}") {
        PlayerViewModel(
            container.players,
            container.events,
            container.settings,
            localPlayer = container.localPlayer,
            waveforms = WaveformCache(container.analysis),
            clock = clock,
        )
    }
    LifecycleStartEffect(player) {
        player.start()
        onStopOrDispose { player.stop() }
    }
    val playerState by player.state.collectAsStateWithLifecycle()
    // "Play on this phone": the media session (notification, lock screen, headset
    // buttons) runs in its own service, started while the app is in front.
    val context = LocalContext.current
    LaunchedEffect(playerState.local.enabled) {
        if (playerState.local.enabled) startPlaybackService(context)
    }
    val snackbar = remember { SnackbarHostState() }
    val res = LocalResources.current
    LaunchedEffect(player) {
        player.messages.collect { m ->
            val text = when (m) {
                is PlayerMessage.Queued -> res.getString(
                    when (m.placement) {
                        Placement.PLAY_NOW -> R.string.queued_play_now
                        Placement.PLAY_NEXT -> R.string.queued_next
                        Placement.ADD_TO_QUEUE -> R.string.queued_add
                    },
                )
                PlayerMessage.NoPlayer -> res.getString(R.string.no_player_selected)
                is PlayerMessage.Failed -> errorText(res, m.error)
            }
            snackbar.showSnackbar(text)
        }
    }

    val backStack by nav.currentBackStackEntryAsState()
    val destination = backStack?.destination
    NavigationSuiteScaffold(
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
            bottomBar = {
                if (destination?.hasRoute(NowPlayingRoute::class) != true) {
                    MiniPlayer(
                        state = playerState,
                        onOpen = { nav.navigate(NowPlayingRoute) },
                        onTogglePlay = player::togglePlay,
                        onNext = player::next,
                    )
                }
            },
        ) { padding ->
            NavHost(nav, startDestination = HomeRoute, modifier = Modifier.padding(padding)) {
                composable<HomeRoute> {
                    Screen(
                        title = stringResource(R.string.nav_home),
                        actions = {
                            IconButton(onClick = { nav.navigate(SettingsRoute) }) {
                                Icon(Icons.Outlined.Settings, contentDescription = stringResource(R.string.title_settings))
                            }
                        },
                    ) {
                        HomeScreen(
                            state = playerState,
                            onSelect = player::select,
                            onOpenNowPlaying = { nav.navigate(NowPlayingRoute) },
                            onRetry = player::refresh,
                            onPlayOnPhone = player::setPlayOnPhone,
                        )
                    }
                }
                composable<AlbumsRoute> {
                    val vm = viewModel { AlbumsViewModel(container.library) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val sort by vm.sort.collectAsStateWithLifecycle()
                    Screen(
                        title = stringResource(R.string.nav_albums),
                        actions = { AlbumSortMenu(sort, vm::setSort) },
                    ) {
                        AlbumGrid(ui, onAlbum = { nav.navigate(AlbumRoute(it.id)) }, onLoadMore = vm::loadMore)
                    }
                }
                composable<ArtistsRoute> {
                    val vm = viewModel { ArtistsViewModel(container.library) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.nav_artists)) {
                        ArtistList(
                            ui,
                            onArtist = { nav.navigate(ArtistRoute(it.id, it.name)) },
                            onLoadMore = vm::loadMore,
                        )
                    }
                }
                composable<SearchRoute> {
                    val vm = viewModel { SearchViewModel(container.library) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.nav_search)) {
                        Column {
                            OutlinedTextField(
                                value = ui.query,
                                onValueChange = vm::onQueryChange,
                                label = { Text(stringResource(R.string.search_field)) },
                                singleLine = true,
                                leadingIcon = { Icon(Icons.Outlined.Search, contentDescription = null) },
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                                modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp),
                            )
                            SearchResultsList(
                                ui,
                                onRetry = vm::retry,
                                onArtist = { nav.navigate(ArtistRoute(it.id, it.name)) },
                                onAlbum = { nav.navigate(AlbumRoute(it.id)) },
                                onTrack = { t ->
                                    t.id?.let { player.enqueue(Selection.Track(it), Placement.PLAY_NOW) }
                                },
                            )
                        }
                    }
                }
                composable<AlbumRoute> { entry ->
                    val route = entry.toRoute<AlbumRoute>()
                    val vm = viewModel { AlbumViewModel(container.library, route.id) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.title_album), onBack = { nav.popBackStack() }) {
                        AlbumScreen(
                            state = ui,
                            onRetry = vm::load,
                            onPlay = { placement, index ->
                                player.enqueue(Selection.Album(route.id), placement, index)
                            },
                            onPlayTrack = { track ->
                                val tracks = (ui as? Loadable.Ready)
                                    ?.value?.tracks.orEmpty()
                                val index = tracks.indexOf(track).takeIf { it >= 0 }
                                player.enqueue(Selection.Album(route.id), Placement.PLAY_NOW, index)
                            },
                            onArtist = { id, name -> nav.navigate(ArtistRoute(id, name)) },
                        )
                    }
                }
                composable<ArtistRoute> { entry ->
                    val route = entry.toRoute<ArtistRoute>()
                    val vm = viewModel { ArtistViewModel(container.library, route.id) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = route.name, onBack = { nav.popBackStack() }) {
                        ArtistScreen(
                            name = route.name,
                            state = ui,
                            onRetry = vm::load,
                            onAlbum = { nav.navigate(AlbumRoute(it.id)) },
                            onPlayAll = { placement -> player.enqueue(Selection.Artist(route.id), placement) },
                        )
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
                            ),
                        )
                    }
                }
                composable<SettingsRoute> {
                    val vm = viewModel { SettingsViewModel(container.account, container.localPlayer) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.title_settings), onBack = { nav.popBackStack() }) {
                        SettingsScreen(
                            state = ui,
                            appVersion = BuildConfig.VERSION_NAME,
                            libraryVersion = remember { libraryVersion() },
                            onSignOut = vm::signOut,
                            onLicences = { nav.navigate(LicencesRoute) },
                            onSound = { nav.navigate(SoundRoute) },
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
            title = { Text(title, maxLines = 1) },
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
