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
import androidx.compose.ui.platform.testTag
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
import io.github.jamezuk.lyrionanywhere.ui.alarms.AlarmsViewModel
import io.github.jamezuk.lyrionanywhere.ui.alarms.AlarmsScreen
import io.github.jamezuk.lyrionanywhere.ui.components.ListDetail
import io.github.jamezuk.lyrionanywhere.ui.components.MainTab
import io.github.jamezuk.lyrionanywhere.ui.components.HomeSettingsButton
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreen
import io.github.jamezuk.lyrionanywhere.ui.player.QueueScreenActions
import io.github.jamezuk.lyrionanywhere.ui.player.SaveQueueDialogView
import io.github.jamezuk.lyrionanywhere.ui.player.saveQueueToday
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
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.LyricsCache
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
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsActions
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FilterAlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorHost
import io.github.jamezuk.lyrionanywhere.ui.library.FilterEditorViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersActions
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersScreen
import io.github.jamezuk.lyrionanywhere.ui.library.FiltersViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.filterMessageText
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryMessage
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.HistorySettings
import io.github.jamezuk.lyrionanywhere.ui.settings.HistoryViewModel
import io.github.jamezuk.lyrionanywhere.ui.stats.AndroidCardSharer
import io.github.jamezuk.lyrionanywhere.ui.stats.ListeningCard
import io.github.jamezuk.lyrionanywhere.ui.stats.ListeningCardViewModel
import io.github.jamezuk.lyrionanywhere.ui.stats.ShareImages
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsActions
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsScreen
import io.github.jamezuk.lyrionanywhere.ui.stats.StatsViewModel
import io.github.jamezuk.lyrionanywhere.ui.stats.YearStoryScreen
import io.github.jamezuk.lyrionanywhere.ui.stats.YearStoryViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.HomeScreen
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryDestination
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.library.Loadable
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistScreen
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistsScreen
import io.github.jamezuk.lyrionanywhere.ui.library.PlaylistsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.SearchViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.SearchScreen
import io.github.jamezuk.lyrionanywhere.ui.library.SearchActions
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryPrefsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryAlbumsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.LibraryArtistsViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.AlbumBrowser
import io.github.jamezuk.lyrionanywhere.ui.library.ArtistBrowser
import io.github.jamezuk.lyrionanywhere.ui.library.ContributorViewModel
import io.github.jamezuk.lyrionanywhere.ui.library.PagedState
import io.github.jamezuk.lyrionanywhere.ui.library.roleLabel
import io.github.jamezuk.lyrionanywhere.ui.offline.OfflineLibraryScreen
import io.github.jamezuk.lyrionanywhere.ui.components.ErrorState
import io.github.jamezuk.lyrionanywhere.ui.components.ConnectionFrame
import io.github.jamezuk.lyrionanywhere.ui.components.connectionMessage
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import androidx.compose.ui.Alignment
import io.github.jamezuk.lyrionanywhere.ui.components.LoadingState
import io.github.jamezuk.lyrionanywhere.data.LibraryChip
import io.github.jamezuk.lyrionanywhere.core.ArtistRole
import androidx.compose.material3.TextButton
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
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsActions
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsScreen
import io.github.jamezuk.lyrionanywhere.ui.player.RoomsViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.VisualiserSetting
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.SleepFadeSetting
import io.github.jamezuk.lyrionanywhere.ui.player.LyricsPeekSetting
import io.github.jamezuk.lyrionanywhere.ui.player.RatingsSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.StreamingQualitySetting
import io.github.jamezuk.lyrionanywhere.ui.settings.ReadAheadSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.ReplayCacheSetting
import io.github.jamezuk.lyrionanywhere.ui.settings.LicencesScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SettingsViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundScreen
import io.github.jamezuk.lyrionanywhere.ui.settings.SoundViewModel
import io.github.jamezuk.lyrionanywhere.ui.settings.actions
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadMessage
import io.github.jamezuk.lyrionanywhere.ui.offline.ContinueOnPrompt
import io.github.jamezuk.lyrionanywhere.ui.offline.continueChoices
import io.github.jamezuk.lyrionanywhere.ui.offline.defaultContinueChoice
import io.github.jamezuk.lyrionanywhere.data.offline.HandOffResult
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
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
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

/** An app's own results for a library search ("Search on"). */
@Serializable data class AppSearchRoute(val cmd: String, val name: String, val query: String)

@Serializable data class PinnedMenuRoute(val id: Long, val name: String)

@Serializable data object HistoryRoute

/** "Your listening" (Wave 3 §1). */
@Serializable data object StatsRoute

/** "Your year in sound" for [year]: full screen. */
@Serializable data class YearStoryRoute(val year: Int)

/** One smart filter's albums (Wave 2 §5.6). */
@Serializable data class FilterRoute(val id: Long, val name: String)

@Serializable data class AlarmsRoute(val player: String, val name: String)

@Serializable data class AlbumRoute(val id: Long)

@Serializable data class ArtistRoute(val id: Long, val name: String)

/** A contributor's albums in one role (a credit followed, or "Appears as"); [role] is an [ArtistRole] name. */
@Serializable data class ContributorRoute(val id: Long, val name: String, val role: String)

@Serializable data object NowPlayingRoute

/** Every room at a glance (Wave 2 §4.3). */
@Serializable data object RoomsRoute

/** The driving screen (Wave 2 §5.1). */
@Serializable data object CarModeRoute

@Serializable data object SettingsRoute

@Serializable data object LicencesRoute

@Serializable data object SoundRoute

@Serializable data object DownloadsRoute

@Serializable data object StorageRoute

/** The Queue tab: the selected player's queue. */
@Serializable data object QueueRoute

/** Each tab's route (the four tabs, as the web app: Home, Library, Search, Queue). */
internal fun MainTab.route(): Any = when (this) {
    MainTab.HOME -> HomeRoute
    MainTab.LIBRARY -> LibraryRoute
    MainTab.SEARCH -> SearchRoute
    MainTab.QUEUE -> QueueRoute
}

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
    is PlayerMessage.SleepSet -> res.getString(R.string.sleep_set, sleepLeft(res, m.seconds))
    PlayerMessage.SleepOff -> res.getString(R.string.sleep_cancelled)
    PlayerMessage.SleepUnknown -> res.getString(R.string.sleep_unknown)
    is PlayerMessage.StationStopped -> res.getString(R.string.station_broken, m.reason)
    PlayerMessage.QueueChanged -> res.getString(R.string.queue_changed)
    PlayerMessage.ClearStopped -> res.getString(R.string.queue_clear_stopped)
    is PlayerMessage.SavedPlaylist -> res.getString(R.string.save_queue_saved, m.name.take(80))
}

/** "45 s", "27 min", "1 h 5 min" (as the web app and Now playing). */
private fun sleepLeft(res: Resources, seconds: Int): String {
    val s = seconds.coerceAtLeast(0)
    if (s < 60) return res.getString(R.string.sleep_left_seconds, s)
    val m = (s + 59) / 60
    if (m < 60) return res.getString(R.string.sleep_left_minutes, m)
    return if (m % 60 == 0) res.getString(R.string.sleep_left_whole_hours, m / 60) else res.getString(R.string.sleep_left_hours, m / 60, m % 60)
}

/** The signed-in app: adaptive navigation (bar, rail or drawer by window size). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MainScreen(
    container: AppContainer,
    signedIn: AuthState.SignedInState,
    /** The link to the server: the "can't reach" banner and the reconnecting pill. */
    connection: ConnectionUi = ConnectionUi(),
) {
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
            lyrics = LyricsCache(container.lyrics),
            ratings = container.ratings,
            nowPlaying = nowPlayingHub,
            me = container.me,
            clock = clock,
            library = container.library,
            downloaded = container.offline.state.map { it.downloadedIds },
        )
    }
    // Which controls work follows the link to the server (offline controls).
    LaunchedEffect(player, connection) { player.setConnection(connection) }
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
    val offlineVm = viewModel(key = "offline-player") { OfflinePlayerViewModel(container.offlinePlayer, offlineClock, container.settings) }
    val offlineUi by offlineVm.state.collectAsStateWithLifecycle()
    LaunchedEffect(offlineUi.state.active) {
        if (offlineUi.state.active) startPlaybackService(context)
    }
    // The volume keys step a remote player's volume while the app is in front; this phone's
    // own player (and downloads playing on it) keep the system's volume.
    val volumeKeys = container.volumeKeys
    androidx.compose.runtime.DisposableEffect(player, offlineUi.state.active) {
        volumeKeys.handler = if (offlineUi.state.active) null else { up -> player.volumeKey(up) }
        onDispose { volumeKeys.handler = null }
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
            val saved = m as? PlayerMessage.SavedPlaylist
            val result = snackbar.showSnackbar(
                messageText(res, m),
                actionLabel = when {
                    undo -> res.getString(R.string.action_undo)
                    saved != null -> res.getString(R.string.save_queue_open)
                    else -> null
                },
                duration = SnackbarDuration.Short,
            )
            if (undo && result == SnackbarResult.ActionPerformed) player.undoRemove((m as PlayerMessage.Removed).token)
            if (saved != null && result == SnackbarResult.ActionPerformed) nav.navigate(PlaylistRoute(saved.id, saved.name))
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

    // "Start radio" and "More like this" (A6): what the server offers for an item.
    fun discoveryText(e: DiscoveryEvent): String = when (e) {
        is DiscoveryEvent.RadioStarted ->
            res.getString(if (e.endless) R.string.radio_started_endless else R.string.radio_started)
        DiscoveryEvent.Played -> res.getString(R.string.similar_playing)
        is DiscoveryEvent.Failed -> errorText(res, e.error)
        DiscoveryEvent.NoPlayer -> res.getString(R.string.no_player_selected)
        DiscoveryEvent.NoRadio -> res.getString(R.string.radio_none)
    }

    // Album, artist and playlist pages: full screen, or beside their list on wide windows.
    val albumPage: @Composable (Long) -> Unit = { albumId ->
        val vm = viewModel(key = "album-$albumId") { AlbumViewModel(container.library, albumId) }
        val ui by vm.state.collectAsStateWithLifecycle()
        val review by vm.review.collectAsStateWithLifecycle()
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
                onQueueTrack = { t, placement -> t.id?.let { player.enqueue(Selection.Track(it), placement) } },
            ),
            review = review,
        )
    }
    val artistPage: @Composable (Long, String) -> Unit = { artistId, name ->
        val vm = viewModel(key = "artist-$artistId") { ArtistViewModel(container.library, artistId) }
        val ui by vm.state.collectAsStateWithLifecycle()
        val info by vm.info.collectAsStateWithLifecycle()
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
                onRole = { role -> nav.navigate(ContributorRoute(artistId, name, role.name)) },
            ),
            info = info,
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
            onQueueTrack = { t, placement -> t.id?.let { player.enqueue(Selection.Track(it), placement) } },
        )
    }

    val backStack by nav.currentBackStackEntryAsState()
    val destination = backStack?.destination
    // Which navigation the window gets: a bottom bar on phones, a rail or drawer on larger
    // screens. The bottom bar handles the gesture/navigation inset itself; otherwise the
    // content (and the mini player) must keep clear of it.
    val suiteType = NavigationSuiteScaffoldDefaults.calculateFromAdaptiveInfo(currentWindowAdaptiveInfo())
    val bottomBarNav = suiteType == NavigationSuiteType.NavigationBar
    // Car mode (Wave 2 §5.1): opened by its button, or by itself for a car (Settings).
    val inCar = destination?.hasRoute(CarModeRoute::class) == true
    // "Your year in sound" has the whole screen too.
    val inStory = destination?.hasRoute(YearStoryRoute::class) == true
    val car = rememberCarMode(container, scope, inCar, onOpen = { nav.navigate(CarModeRoute) { launchSingleTop = true } }, onLeave = { nav.popBackStack() })
    // "Start radio" and "Sleep 30 min" from the launcher (Wave 2 §5.5).
    ShortcutRequests(container, player, snackbar) { e -> discoveryText(e) }
    NavigationSuiteScaffold(
        // Car mode has the whole screen.
        layoutType = if (inCar || inStory) NavigationSuiteType.None else suiteType,
        navigationSuiteItems = {
            MainTab.entries.forEach { tab ->
                val route = tab.route()
                val selected = destination?.hierarchy?.any { it.hasRoute(route::class) } == true
                item(
                    selected = selected,
                    onClick = { nav.navigateTopLevel(route) },
                    icon = { Icon(tab.icon, contentDescription = null) },
                    label = { Text(stringResource(tab.label)) },
                    modifier = Modifier.testTag(tab.tag),
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
                    destination?.hasRoute(OfflineNowPlayingRoute::class) == true || inCar || inStory
                // Back online with downloads queued: "Continue on <player>?" (SPEC §8.4).
                if (!inCar && !inStory) {
                    val handOff by container.continueOn.state.collectAsStateWithLifecycle()
                    if (handOff.offered || handOff.result != null) {
                        val phoneName = stringResource(R.string.this_phone)
                        val choices = remember(playerState.players, playerState.local.playerId, phoneName) {
                            continueChoices(playerState.players, playerState.local.playerId, phoneName, phoneAvailable = true)
                        }
                        ContinueOnPrompt(
                            state = handOff,
                            choices = choices,
                            default = defaultContinueChoice(choices, playerState.selectedId, playerState.local.playerId),
                            onContinue = { c ->
                                container.continueOn.continueOn(c.id, c.name) { r, id ->
                                    if (r is HandOffResult.Moved && id != null) {
                                        if (id == playerState.local.playerId) player.backToPhone() else player.select(id)
                                    }
                                }
                            },
                            onNotNow = container.continueOn::dismiss,
                            onDismissResult = container.continueOn::clearResult,
                        )
                    }
                }
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
                        onPrevious = player::previous,
                    )
                }
              }
            },
        ) { padding ->
            val openAlbum: (Long) -> Unit = { nav.navigate(AlbumRoute(it)) }
            // The link to the server: one line at the top that takes its own space, so it
            // never covers the controls (offline controls). Car mode and stories say nothing.
            val playingHere = playerState.local.status?.playback == io.github.jamezuk.lyrionanywhere.core.LocalPlayback.PLAYING ||
                offlineUi.state.playing
            val connectionLine = connectionMessage(connection, playingHere).takeIf { !inCar && !inStory }
            val toDownloads = { nav.navigate(DownloadsRoute) { launchSingleTop = true } }
                .takeIf { destination?.hasRoute(DownloadsRoute::class) != true }
            ConnectionFrame(connectionLine, toDownloads, Modifier.padding(padding)) {
            BoxWithConstraints(Modifier.fillMaxSize()) {
            val layout = WindowLayout.of(maxWidth.value, maxHeight.value)
            CompositionLocalProvider(LocalWindowLayout provides layout) {
            NavHost(nav, startDestination = HomeRoute) {
                composable<HomeRoute> {
                    val vm = viewModel {
                        HomeFeedViewModel(container.library, libraryId, container.me, player::target, container.filters, container.players)
                    }
                    val feed by vm.state.collectAsStateWithLifecycle()
                    LaunchedEffect(vm) {
                        vm.messages.collect { m ->
                            snackbar.showSnackbar(
                                when (m) {
                                    is HomeMessage.Failed -> errorText(res, m.error)
                                    is HomeMessage.StationBroken -> res.getString(R.string.station_broken, m.reason)
                                    HomeMessage.NoPlayer -> res.getString(R.string.station_no_player)
                                    is HomeMessage.StationStarted -> res.getString(R.string.station_started, m.name)
                                    is HomeMessage.Filter -> filterMessageText(res, m.message)
                                },
                            )
                        }
                    }
                    val listening = viewModel { ListeningCardViewModel(container.stats) }
                    val listeningUi by listening.state.collectAsStateWithLifecycle()
                    LaunchedEffect(listening) { listening.refresh() }
                    Screen(
                        title = stringResource(R.string.nav_home),
                        actions = {
                            HomeSettingsButton(onClick = { nav.navigate(SettingsRoute) })
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
                                onFilter = { id, name -> nav.navigate(FilterRoute(id, name)) },
                                onPlayFilter = vm::playFilter,
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
                            footer = {
                                ListeningCard(
                                    listeningUi,
                                    onOpen = { nav.navigate(StatsRoute) },
                                    onStory = { nav.navigate(YearStoryRoute(it)) },
                                )
                            },
                        )
                    }
                }
                composable<LibraryRoute> {
                    val prefsVm = viewModel { LibraryPrefsViewModel(container.settings) }
                    val prefs by prefsVm.prefs.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.nav_library)) {
                        val p = prefs
                        if (p == null) {
                            LoadingState()
                        } else {
                            LibraryScreen(
                                libraries = libraries,
                                chosenLibrary = chosenLibrary,
                                onChooseLibrary = { id -> scope.launch { container.libraries.choose(id) } },
                                onNewMusic = { nav.navigate(NewMusicRoute) },
                                onOpen = { d ->
                                    // The types with a chip open as that chip (Wave 2 §3.2).
                                    when (d) {
                                        LibraryDestination.ALBUMS -> prefsVm.toggleChip(LibraryChip.ALBUMS)
                                        LibraryDestination.ARTISTS -> prefsVm.toggleChip(LibraryChip.ARTISTS)
                                        LibraryDestination.PLAYLISTS -> prefsVm.toggleChip(LibraryChip.PLAYLISTS)
                                        LibraryDestination.FAVOURITES -> prefsVm.toggleChip(LibraryChip.FAVOURITES)
                                        LibraryDestination.DOWNLOADS -> prefsVm.toggleChip(LibraryChip.DOWNLOADED)
                                        else -> nav.openLibrary(d)
                                    }
                                },
                                chip = p.chip,
                                onChip = prefsVm::toggleChip,
                                chipContent = { chip ->
                                    when (chip) {
                                        LibraryChip.ALBUMS -> {
                                            val vm = viewModel(key = "library-albums") {
                                                LibraryAlbumsViewModel(container.library, p.albumOrder, libraryId, container.me)
                                            }
                                            val ui by vm.state.collectAsStateWithLifecycle()
                                            val sort by vm.order.collectAsStateWithLifecycle()
                                            AlbumBrowser(
                                                ui,
                                                sort = sort,
                                                grid = p.albumGrid,
                                                onSort = { s ->
                                                    vm.setOrder(s)
                                                    prefsVm.setOrder(s)
                                                },
                                                onGrid = prefsVm::setGrid,
                                                onEnsure = vm::ensure,
                                                onRetry = vm::retry,
                                                onAlbum = { openAlbum(it.id) },
                                                onMore = openMore,
                                                onQueue = { a, placement -> player.enqueue(Selection.Album(a.id), placement) },
                                            )
                                        }
                                        LibraryChip.ARTISTS -> {
                                            val vm = viewModel(key = "library-artists") { LibraryArtistsViewModel(container.library, libraryId) }
                                            val ui by vm.state.collectAsStateWithLifecycle()
                                            val views by vm.views.collectAsStateWithLifecycle()
                                            val view by vm.view.collectAsStateWithLifecycle()
                                            ArtistBrowser(
                                                ui,
                                                views = views,
                                                view = view,
                                                onView = vm::setView,
                                                onEnsure = vm::ensure,
                                                onRetry = vm::retry,
                                                onArtist = { nav.navigate(ArtistRoute(it.id, it.name)) },
                                                onMore = openMore,
                                            )
                                        }
                                        LibraryChip.PLAYLISTS -> {
                                            val vm = viewModel(key = "library-playlists") { PlaylistsViewModel(container.library) }
                                            val ui by vm.state.collectAsStateWithLifecycle()
                                            PlaylistsScreen(
                                                ui,
                                                onPlaylist = { nav.navigate(PlaylistRoute(it.id, it.name)) },
                                                onLoadMore = vm::loadMore,
                                                onMore = openMore,
                                            )
                                        }
                                        LibraryChip.FAVOURITES -> {
                                            val vm = viewModel(key = "library-favourites") { FavouritesViewModel(container.library, null) }
                                            val ui by vm.state.collectAsStateWithLifecycle()
                                            FavouritesScreen(
                                                ui,
                                                onRetry = vm::reload,
                                                onFolder = { nav.navigate(FavouritesRoute(it.id, it.name)) },
                                                onPlay = { f, placement -> player.playFavourite(f.id, placement) },
                                            )
                                        }
                                        LibraryChip.FILTERS -> {
                                            val vm = viewModel(key = "library-filters") {
                                                FiltersViewModel(container.filters, container.players, player::target)
                                            }
                                            val ui by vm.state.collectAsStateWithLifecycle()
                                            LaunchedEffect(vm) { vm.messages.collect { snackbar.showSnackbar(filterMessageText(res, it)) } }
                                            // Back from a filter's page (edited or deleted there): read the list again.
                                            LaunchedEffect(Unit) { vm.load() }
                                            // The filter being edited: 0 for a new one, null when the builder is closed.
                                            var editing by rememberSaveable { mutableStateOf<Long?>(null) }
                                            var session by rememberSaveable { mutableIntStateOf(0) }
                                            FiltersScreen(
                                                ui,
                                                FiltersActions(
                                                    onOpen = { nav.navigate(FilterRoute(it.id, it.name)) },
                                                    onNew = {
                                                        session++
                                                        editing = 0L
                                                    },
                                                    onEdit = {
                                                        session++
                                                        editing = it.id
                                                    },
                                                    onPlay = vm::play,
                                                    onDelete = vm::delete,
                                                    onRetry = vm::load,
                                                ),
                                            )
                                            editing?.let { id ->
                                                val initial = (ui.filters as? Loadable.Ready)?.value?.firstOrNull { it.id == id }
                                                if (id == 0L || initial != null) {
                                                    FilterEditorHost(
                                                        key = "filter-editor-$session",
                                                        make = { FilterEditorViewModel(container.filters, container.library, initial) },
                                                        onSaved = { f ->
                                                            editing = null
                                                            vm.saved(f, added = id == 0L)
                                                        },
                                                        onClose = { editing = null },
                                                    )
                                                }
                                            }
                                        }
                                        LibraryChip.DOWNLOADED -> OfflineLibraryScreen(
                                            offlineState.albums,
                                            onAlbum = { nav.navigate(OfflineAlbumRoute(it)) },
                                            onPlayAlbum = { ref ->
                                                val tracks = OfflineAlbumViewModel.albumOf(offlineState, ref).tracks
                                                if (tracks.isNotEmpty()) container.offlinePlayer.play(tracks, 0, false)
                                            },
                                        )
                                    }
                                },
                            )
                        }
                    }
                }
                composable<SearchRoute> {
                    val vm = viewModel { SearchViewModel(container.library, container.settings, container.searchDirectory, player::target) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val typed by vm.typed.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.nav_search)) {
                        SearchScreen(
                            ui,
                            typed,
                            SearchActions(
                                onQuery = vm::onQueryChange,
                                onSubmit = { vm.remember() },
                                onType = vm::setType,
                                onRetry = vm::retry,
                                onLoadMore = vm::loadMoreTyped,
                                onRecent = vm::useRecent,
                                onRemoveRecent = vm::removeRecent,
                                onClearRecents = vm::clearRecents,
                                onArtist = {
                                    vm.remember()
                                    nav.navigate(ArtistRoute(it.id, it.name))
                                },
                                onAlbum = {
                                    vm.remember()
                                    openAlbum(it.id)
                                },
                                onPlaylist = {
                                    vm.remember()
                                    nav.navigate(PlaylistRoute(it.id, it.name))
                                },
                                onPlay = { sel, placement ->
                                    vm.remember()
                                    player.enqueue(sel, placement)
                                },
                                onMore = openMore,
                                onSearchOn = { app ->
                                    vm.remember()
                                    nav.navigate(AppSearchRoute(app.cmd, app.name, ui.query.trim()))
                                },
                            ),
                        )
                    }
                }
                composable<ContributorRoute> { entry ->
                    val route = entry.toRoute<ContributorRoute>()
                    val role = ArtistRole.entries.firstOrNull { it.name == route.role } ?: ArtistRole.ARTIST
                    val vm = viewModel(key = "contributor-${route.id}-$role") { ContributorViewModel(container.library, route.id, role) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val title = stringResource(R.string.contributor_title, route.name, roleLabel(role))
                    Screen(title = title, onBack = { nav.popBackStack() }) {
                        when (val s = ui) {
                            Loadable.Loading -> LoadingState()
                            is Loadable.Failed -> ErrorState(s.error, vm::reload)
                            is Loadable.Ready -> AlbumGrid(
                                PagedState(s.value, total = s.value.size),
                                onAlbum = { openAlbum(it.id) },
                                onLoadMore = {},
                                onMore = openMore,
                                empty = stringResource(R.string.no_albums_role),
                                header = {
                                    TextButton(onClick = { nav.navigate(ArtistRoute(route.id, route.name)) }) {
                                        Text(stringResource(R.string.open_artist))
                                    }
                                },
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
                composable<AppSearchRoute> { entry ->
                    val route = entry.toRoute<AppSearchRoute>()
                    val vm = viewModel { MenuViewModel(container.menus, container.me, player::target) }
                    val title = stringResource(R.string.search_on_title, route.name, route.query)
                    LaunchedEffect(route, playerState.selectedId) {
                        vm.openSearch(container.searchDirectory, route.cmd, title, route.query, playerState.selectedId)
                    }
                    MenuRouteScreen(
                        title,
                        vm,
                        onBack = { nav.popBackStack() },
                        onEvent = { handleMenuEvent(it) { nav.popBackStack() } },
                        missingText = stringResource(R.string.search_on_missing),
                    )
                }
                composable<PinnedMenuRoute> { entry ->
                    val route = entry.toRoute<PinnedMenuRoute>()
                    val vm = viewModel { MenuViewModel(container.menus, container.me, player::target) }
                    LaunchedEffect(route, playerState.selectedId) { vm.openPinned(route.id, route.name, playerState.selectedId) }
                    MenuRouteScreen(route.name, vm, onBack = { nav.popBackStack() }, onEvent = { handleMenuEvent(it) { nav.popBackStack() } })
                }
                composable<AlarmsRoute> { entry ->
                    val route = entry.toRoute<AlarmsRoute>()
                    val vm = viewModel(key = "alarms-${route.player}") { AlarmsViewModel(container.alarms, route.player) }
                    Screen(title = stringResource(R.string.alarms), onBack = { nav.popBackStack() }) {
                        AlarmsScreen(vm, route.name)
                    }
                }
                composable<FilterRoute> { entry ->
                    val route = entry.toRoute<FilterRoute>()
                    val vm = viewModel(key = "filter-${route.id}") {
                        FilterAlbumsViewModel(container.filters, container.me, container.players, player::target, route.id)
                    }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    LaunchedEffect(vm) { vm.messages.collect { snackbar.showSnackbar(filterMessageText(res, it)) } }
                    LaunchedEffect(ui.deleted) { if (ui.deleted) nav.popBackStack() }
                    var editing by rememberSaveable { mutableStateOf(false) }
                    var session by rememberSaveable { mutableIntStateOf(0) }
                    val title = ((ui.filter as? Loadable.Ready)?.value?.name) ?: route.name
                    Screen(title = title, onBack = { nav.popBackStack() }) {
                        FilterAlbumsScreen(
                            ui,
                            FilterAlbumsActions(
                                onAlbum = { openAlbum(it.id) },
                                onLoadMore = vm::loadMore,
                                onMore = openMore,
                                onPlay = vm::play,
                                onEdit = {
                                    session++
                                    editing = true
                                },
                                onDelete = vm::delete,
                                onRetry = vm::reload,
                            ),
                        )
                    }
                    val initial = (ui.filter as? Loadable.Ready)?.value
                    if (editing && initial != null) {
                        FilterEditorHost(
                            key = "filter-editor-${route.id}-$session",
                            make = { FilterEditorViewModel(container.filters, container.library, initial) },
                            onSaved = {
                                editing = false
                                vm.saved(it)
                            },
                            onClose = { editing = false },
                        )
                    }
                }
                composable<HistoryRoute> {
                    val vm = viewModel { HistoryViewModel(container.me) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.section_history), onBack = { nav.popBackStack() }) {
                        HistoryScreen(ui, onMore = vm::loadMore, onRetry = vm::load)
                    }
                }
                composable<StatsRoute> {
                    val vm = viewModel { StatsViewModel(container.stats, container.me) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val cache = LocalContext.current.applicationContext
                    // Shared card images are kept an hour for the app that received them.
                    LaunchedEffect(Unit) {
                        withContext(Dispatchers.IO) { ShareImages.prune(ShareImages.dir(cache), System.currentTimeMillis()) }
                    }
                    Screen(title = stringResource(R.string.stats_title), onBack = { nav.popBackStack() }) {
                        StatsScreen(
                            ui,
                            StatsActions(
                                onPeriod = vm::select,
                                onRetry = vm::retry,
                                onArtist = { id, name -> nav.navigate(ArtistRoute(id, name)) },
                                onAlbum = openAlbum,
                                onStory = { nav.navigate(YearStoryRoute(it)) },
                                onHistorySettings = { nav.navigate(SettingsRoute) { launchSingleTop = true } },
                            ),
                        )
                    }
                }
                composable<YearStoryRoute> { entry ->
                    val route = entry.toRoute<YearStoryRoute>()
                    val vm = viewModel(key = "story-${route.year}") { YearStoryViewModel(container.stats, container.me, route.year) }
                    val ui by vm.state.collectAsStateWithLifecycle()
                    val shareContext = LocalContext.current
                    val sharer = remember(shareContext) { AndroidCardSharer(shareContext) }
                    YearStoryScreen(
                        ui,
                        onClose = { nav.popBackStack() },
                        onRetry = vm::retry,
                        sharer = sharer,
                        onHistorySettings = { nav.navigate(SettingsRoute) { launchSingleTop = true } },
                    )
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
                    // No top app bar: a small close chevron over the art instead (back and the
                    // swipe down close too).
                    NowPlayingScreen(
                        state = playerState,
                        clock = clock,
                        twoPane = LocalWindowLayout.current.nowPlayingTwoPane,
                        wide = LocalWindowLayout.current.nowPlayingWide,
                        visual = player,
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
                            onSleep = player::setSleep,
                            onCancelSleep = player::cancelSleep,
                            onRemoveUpNext = player::removeUpNext,
                            onRate = player::rate,
                            onAlarms = playerState.selected?.let { p -> { nav.navigate(AlarmsRoute(p.id, p.name)) } },
                            onLoadMoreQueue = player::loadMoreQueue,
                            onPlayNextInQueue = player::playNextInQueue,
                            onClearUpcoming = player::clearUpcoming,
                            onSaveQueue = { player.openSaveQueue(saveQueueToday()) },
                            onShuffleMode = player::setShuffle,
                            onLayout = player::setLayout,
                            onVisualiser = player::setVisualiser,
                            onVisualStyle = player::setVisualStyle,
                            onQueuePanel = player::setQueuePanel,
                            onCredits = player::loadCredits,
                            onCredit = { c -> c.artistId?.let { id -> nav.navigate(ContributorRoute(id, c.name, c.role.name)) } },
                            onRooms = { nav.navigate(RoomsRoute) },
                            onCarMode = car::openByHand,
                        ),
                    )
                }
                composable<QueueRoute> {
                    LaunchedEffect(playerState.selectedId, playerState.status?.queueIndex, playerState.status?.queueLength) {
                        player.loadQueue()
                    }
                    Screen(title = stringResource(R.string.nav_queue)) {
                        QueueScreen(
                            playerState,
                            QueueScreenActions(
                                queue = QueueActions(
                                    onJump = player::jumpTo,
                                    onRemove = player::removeFromQueue,
                                    onMove = player::moveInQueue,
                                    onClear = player::clearQueue,
                                    onLoadMore = player::loadMoreQueue,
                                    onPlayNext = player::playNextInQueue,
                                ),
                                onClearUpcoming = player::clearUpcoming,
                                onSaveQueue = { player.openSaveQueue(saveQueueToday()) },
                                onChoosePlayer = openOutputs,
                                onRetry = player::refresh,
                            ),
                        )
                    }
                }
                composable<RoomsRoute> {
                    val vm = viewModel { RoomsViewModel(container.players, container.events, container.alarms) }
                    LifecycleStartEffect(vm) {
                        vm.start()
                        onStopOrDispose { vm.stop() }
                    }
                    LaunchedEffect(vm) { vm.messages.collect { e -> snackbar.showSnackbar(errorText(res, e)) } }
                    val rooms by vm.state.collectAsStateWithLifecycle()
                    Screen(title = stringResource(R.string.rooms_title), onBack = { nav.popBackStack() }) {
                        RoomsScreen(
                            rooms,
                            RoomsActions(
                                onControl = { id ->
                                    if (id == playerState.local.playerId) player.backToPhone() else player.select(id)
                                    if (!nav.popBackStack(NowPlayingRoute, inclusive = false)) nav.navigate(NowPlayingRoute)
                                },
                                onTogglePlay = vm::togglePlay,
                                onVolume = vm::setVolume,
                                onPower = vm::setPower,
                                onSync = vm::sync,
                                onUnsync = vm::unsync,
                                onRetry = vm::retry,
                                onTransfer = player::moveTo,
                                onAlarms = { id, name -> nav.navigate(AlarmsRoute(id, name)) },
                            ),
                            selectedId = playerState.selectedId,
                            transfer = playerState.transfer,
                            onDismissTransfer = player::dismissTransfer,
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
                    OfflineNowPlayingScreen(
                        offlineUi,
                        offlineClock,
                        offlineNowPlayingActions(offlineVm),
                        visual = offlineVm,
                        onClose = { nav.popBackStack() },
                    )
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
                            onChangePassword = vm::changePassword,
                            onClosePasswordForm = vm::closePasswordForm,
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
                                    onStats = { nav.navigate(StatsRoute) },
                                )
                            },
                            sleepFade = { SleepFadeSetting(playerState.sleepFade, player::setSleepFade) },
                            lyrics = { LyricsPeekSetting(playerState.lyricsPeek, player::setLyricsPeek) },
                            visualiser = {
                                VisualiserSetting(
                                    playerState.nowPlaying.visualiser,
                                    playerState.nowPlaying.visualStyle,
                                    player::setVisualiser,
                                    player::setVisualStyle,
                                )
                            },
                            ratings = {
                                RatingsSetting(playerState.ratingStyle, player::setRatingStyle, player::clearRatings)
                            },
                            streaming = {
                                val app by container.settings.settings.collectAsStateWithLifecycle(AppSettings())
                                StreamingQualitySetting(app.streaming) { s -> scope.launch { container.settings.setStreaming(s) } }
                            },
                            device = { DeviceSettings(container, scope) },
                            gaps = {
                                val app by container.settings.settings.collectAsStateWithLifecycle(AppSettings())
                                ReadAheadSetting(app.readAhead) { s -> scope.launch { container.settings.setReadAhead(s) } }
                                val local by container.localPlayer.state.collectAsStateWithLifecycle()
                                LaunchedEffect(Unit) { container.localPlayer.refreshReplayCache() }
                                ReplayCacheSetting(
                                    bytes = app.replayCacheBytes,
                                    usage = local.replayCache,
                                    downloadsAllowed = ui.account?.downloadsEnabled,
                                    onSize = { b -> scope.launch { container.settings.setReplayCacheBytes(b) } },
                                    onClear = { scope.launch { container.localPlayer.clearReplayCache() } },
                                )
                            },
                        )
                    }
                }
                composable<CarModeRoute> {
                    CarModeContent(container, player, playerState, clock, snackbar, onExit = car::leaveByHand) { e -> discoveryText(e) }
                }
                composable<SoundRoute> {
                    val vm = viewModel { SoundViewModel(container.localPlayer, players = container.players) }
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
        // A track's credits (Wave 2 §3.1), under its play buttons.
        val credits by androidx.compose.runtime.produceState(emptyList<io.github.jamezuk.lyrionanywhere.core.Credit>(), request) {
            if (request.kind == io.github.jamezuk.lyrionanywhere.core.InfoKind.TRACK) {
                value = (container.library.trackCredits(request.id) as? ApiResult.Ok)?.value.orEmpty()
            }
        }
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
                credits = credits,
                onCredit = { c ->
                    c.artistId?.let { id ->
                        more = null
                        nav.navigate(ContributorRoute(id, c.name, c.role.name))
                    }
                },
            )
        }
    }

    // "Save as playlist" (from the Queue tab, the queue panel or the queue peek).
    playerState.saveQueue?.let { d ->
        SaveQueueDialogView(d, onSave = player::saveQueue, onChangeName = player::saveQueueChangeName, onDismiss = player::closeSaveQueue)
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
                        onAllRooms = {
                            outputs = false
                            nav.navigate(RoomsRoute)
                        },
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
private fun MenuRouteScreen(
    title: String,
    vm: MenuViewModel,
    onBack: () -> Unit,
    onEvent: (MenuEvent) -> Unit,
    missingText: String = stringResource(R.string.menu_missing),
) {
    val missing by vm.missing.collectAsStateWithLifecycle()
    LaunchedEffect(vm) { vm.navigator.events.collect(onEvent) }
    Screen(title = title, onBack = { if (!vm.navigator.back()) onBack() }) {
        if (missing) EmptyState(missingText) else MenuBrowser(vm.navigator)
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
