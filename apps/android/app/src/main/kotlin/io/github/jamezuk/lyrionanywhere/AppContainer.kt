package io.github.jamezuk.lyrionanywhere

import android.content.Context
import android.os.Build
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import io.github.jamezuk.lyrionanywhere.data.RustRatingsRepository
import io.github.jamezuk.lyrionanywhere.data.RatingsRepository
import io.github.jamezuk.lyrionanywhere.data.RustAlarmsRepository
import io.github.jamezuk.lyrionanywhere.data.AlarmsRepository
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.AnalysisRepository
import io.github.jamezuk.lyrionanywhere.data.LyricsRepository
import io.github.jamezuk.lyrionanywhere.data.RustLyricsRepository
import io.github.jamezuk.lyrionanywhere.data.RustAnalysisRepository
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.DataStoreSettings
import io.github.jamezuk.lyrionanywhere.data.DiscoveryRepository
import io.github.jamezuk.lyrionanywhere.data.RustDiscoveryRepository
import io.github.jamezuk.lyrionanywhere.data.MeRepository
import io.github.jamezuk.lyrionanywhere.data.RustMeRepository
import io.github.jamezuk.lyrionanywhere.data.FiltersRepository
import io.github.jamezuk.lyrionanywhere.data.RustFiltersRepository
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.MenusRepository
import io.github.jamezuk.lyrionanywhere.data.RustMenusRepository
import io.github.jamezuk.lyrionanywhere.data.SuitabilityCache
import io.github.jamezuk.lyrionanywhere.data.VirtualLibraries
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.RustAccountRepository
import io.github.jamezuk.lyrionanywhere.data.RustArtRepository
import io.github.jamezuk.lyrionanywhere.data.RustEventsRepository
import io.github.jamezuk.lyrionanywhere.data.RustLibraryRepository
import io.github.jamezuk.lyrionanywhere.data.RustPlayersRepository
import io.github.jamezuk.lyrionanywhere.data.ServerBusyTracker
import io.github.jamezuk.lyrionanywhere.data.SettingsStore
import io.github.jamezuk.lyrionanywhere.data.secure.AesGcmCipher
import io.github.jamezuk.lyrionanywhere.data.secure.EncryptedCredentialStore
import io.github.jamezuk.lyrionanywhere.data.secure.KeystoreKeyProvider
import io.github.jamezuk.lyrionanywhere.data.secure.NoBackupFileStorage
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.RustLocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.AndroidNetworkMonitor
import io.github.jamezuk.lyrionanywhere.data.offline.NetworkMonitor
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineModeController
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineRepository
import io.github.jamezuk.lyrionanywhere.data.offline.PlaybackArbiter
import io.github.jamezuk.lyrionanywhere.data.offline.RustOfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.RustOfflineRepository
import io.github.jamezuk.lyrionanywhere.data.offline.RustOnlineLink
import io.github.jamezuk.lyrionanywhere.ui.offline.DownloadActions
import io.github.jamezuk.lyrionanywhere.ui.onboarding.PairingLinks
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.SharingStarted
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.stateIn

/**
 * The app's object graph, wired by hand (no DI framework): it is small, has one scope
 * (the process), and constructor injection keeps every view model testable with fakes.
 * No annotation processing or reflection, which keeps builds fast and reproducible.
 */
class AppContainer(context: Context, scope: CoroutineScope) {
    private val app = context.applicationContext

    val settings: SettingsStore = DataStoreSettings(
        PreferenceDataStoreFactory.create { app.preferencesDataStoreFile("settings") },
    )

    private val secureStore = EncryptedCredentialStore(
        cipher = AesGcmCipher(KeystoreKeyProvider()),
        storage = NoBackupFileStorage(app),
    )

    private val accountImpl = RustAccountRepository(
        settings = settings,
        secureStore = secureStore,
        deviceName = deviceName(),
        io = Dispatchers.IO,
        // A gateway on this device over plain HTTP: development builds only.
        allowPlainLoopback = BuildConfig.DEBUG,
    )
    val account: AccountRepository = accountImpl

    /**
     * A `lyrion-anywhere://pair` link the app was opened with, held until the user confirms
     * (or declines) on the pairing screen. Holding it changes nothing by itself.
     */
    val pairingLinks = PairingLinks(account::readPairingCode)

    /** Where the hardware volume keys go while the app is in front (see [VolumeKeys]). */
    val volumeKeys = VolumeKeys()

    /** The launcher's app shortcuts, waiting for the signed-in app (Wave 2 §5.5). */
    val shortcuts = AppShortcuts { android.os.SystemClock.elapsedRealtime() }
    /** Whether the music server is busy (it stopped answering; the gateway is reachable). */
    val serverBusy = ServerBusyTracker(scope) { android.os.SystemClock.elapsedRealtime() }
    val library: LibraryRepository = RustLibraryRepository(accountImpl)
    val players: PlayersRepository = RustPlayersRepository(accountImpl, serverBusy)
    val art: ArtRepository = RustArtRepository(accountImpl)
    val events: EventsRepository = RustEventsRepository(accountImpl, serverBusy)
    val analysis: AnalysisRepository = RustAnalysisRepository(accountImpl)
    val lyrics: LyricsRepository = RustLyricsRepository(accountImpl)
    val alarms: AlarmsRepository = RustAlarmsRepository(accountImpl)
    val menus: MenusRepository = RustMenusRepository(accountImpl)

    /** Which apps the library search can hand its text to ("Search on"), found once per session. */
    val searchDirectory = io.github.jamezuk.lyrionanywhere.ui.library.SearchDirectory(menus)

    /** Listening history, the customisable Home and stations (Wave 1). */
    val me: MeRepository = RustMeRepository(accountImpl)

    /** "Your listening" and "Your year in sound" (Wave 3 §1). */
    val stats: io.github.jamezuk.lyrionanywhere.data.StatsRepository =
        io.github.jamezuk.lyrionanywhere.data.RustStatsRepository(accountImpl)

    /** Smart filters (Wave 2 §5.6). */
    val filters: FiltersRepository = RustFiltersRepository(accountImpl)

    /** "Start radio" and "More like this" (A6). */
    val discovery: DiscoveryRepository = RustDiscoveryRepository(accountImpl) { android.os.SystemClock.elapsedRealtime() }

    /** The LMS virtual library browsed (offered only when LMS has any). */
    val libraries = VirtualLibraries(library, settings)

    /** How usable each app is here, remembered for a while (Apps & Radio). */
    val suitability = SuitabilityCache(clock = { android.os.SystemClock.elapsedRealtime() })

    /** Wi-Fi, mobile data or none (for download formats, "Wi-Fi only", offline mode and the stream level). */
    val network: NetworkMonitor = AndroidNetworkMonitor(app)

    /** This phone as an LMS player, while signed in and switched on. */
    val localPlayer: LocalPlayerRepository = RustLocalPlayerRepository(
        sessions = accountImpl.connection
            .map { it?.session }
            .stateIn(scope, SharingStarted.Eagerly, null),
        settings = settings,
        scope = scope,
        io = Dispatchers.IO,
        network = network.kind,
        // Live streams spooled for reading ahead (rolling cache): app-private, never backed
        // up, beside the downloads; the engine creates it and scopes it by server and user.
        cacheDir = File(app.noBackupFilesDir, "stream-cache").absolutePath,
        // The downloads (read only): an Original FLAC stream of a downloaded track plays
        // from its download after its first bytes.
        downloadsDir = File(app.noBackupFilesDir, "offline").absolutePath,
    )

    private val offlineImpl = RustOfflineRepository(
        // App-private and never backed up or transferred (the app also disables backup).
        root = File(app.noBackupFilesDir, "offline").absolutePath,
        auth = accountImpl.state,
        link = accountImpl.connection
            .map { c -> c?.let { RustOnlineLink(it) } }
            .stateIn(scope, SharingStarted.Eagerly, null),
        settings = settings,
        network = network.kind,
        library = library,
        scope = scope,
        io = Dispatchers.IO,
    )

    /** Downloads, the offline library, smart downloads and plays sync-back (A5). */
    val offline: OfflineRepository = offlineImpl

    /** Ratings (Wave 2); a downloaded track rated offline is queued and sent with the plays. */
    val ratings: RatingsRepository = RustRatingsRepository(accountImpl) { id, v -> offlineImpl.rateOffline(id, v) }

    /** Downloads played on this phone from the app's own queue. */
    val offlinePlayer: OfflinePlayerRepository = RustOfflinePlayerRepository(
        store = offlineImpl.store,
        settings = settings,
        scope = scope,
        io = Dispatchers.IO,
    )

    /** "Download" buttons across the app. */
    val downloads = DownloadActions(offline, scope)

    /** Offline mode: the server can't be reached, so the app shows and plays downloads. */
    /** Whether the app is in the foreground (an activity started). */
    val appForeground = AppForeground().foreground

    /** How well this phone's player keeps going with the screen off. */
    val backgroundPlayback = io.github.jamezuk.lyrionanywhere.data.BackgroundPlayback()

    val offlineMode = OfflineModeController(
        scope = scope,
        auth = accountImpl.state,
        network = network.kind,
        serverTrouble = localPlayer.state.map(OfflineModeController::troubleOf),
        retry = { accountImpl.retry() },
        // Android cuts a backgrounded app's network when it isn't playing: the phone's
        // player losing the server then doesn't count, and it reconnects on return.
        foreground = appForeground,
        playing = localPlayer.state.map(OfflineModeController::playingOf),
        reconnectNow = { localPlayer.reconnectNow() },
        now = { android.os.SystemClock.elapsedRealtime() },
        // The music server busy: the gateway answers, so that is never offline.
        serverBusy = serverBusy.busy,
    ).also { it.start() }

    /** "Continue on <player>": back online, offer to hand the downloads queue to a player (SPEC §8.4). */
    val continueOn = io.github.jamezuk.lyrionanywhere.data.offline.ContinueOnController(
        scope = scope,
        auth = accountImpl.state,
        connection = offlineMode.ui,
        network = network.kind,
        offlinePlayer = offlinePlayer,
        owner = offlineImpl.state,
        players = players,
        resolver = io.github.jamezuk.lyrionanywhere.data.offline.RustTrackResolver(accountImpl, offlineImpl::storeOf),
        localPlayer = localPlayer,
        clock = { android.os.SystemClock.elapsedRealtime() },
    ).also {
        it.start()
        offlineImpl.beforeSignOut = it::forget
    }

    init {
        // Only one of the phone's two players makes sound at a time.
        PlaybackArbiter(scope, localPlayer, players, offlinePlayer).start()
    }

    /** What this phone is called in LMS and the device list, e.g. "Google Pixel 8". */
    private fun deviceName(): String {
        val maker = Build.MANUFACTURER.replaceFirstChar { it.uppercase() }
        val model = Build.MODEL
        return if (model.startsWith(maker, ignoreCase = true)) model else "$maker $model"
    }
}
