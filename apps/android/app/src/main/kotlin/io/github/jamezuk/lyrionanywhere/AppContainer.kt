package io.github.jamezuk.lyrionanywhere

import android.content.Context
import android.os.Build
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.preferencesDataStoreFile
import io.github.jamezuk.lyrionanywhere.data.AccountRepository
import io.github.jamezuk.lyrionanywhere.data.AnalysisRepository
import io.github.jamezuk.lyrionanywhere.data.RustAnalysisRepository
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.DataStoreSettings
import io.github.jamezuk.lyrionanywhere.data.DiscoveryRepository
import io.github.jamezuk.lyrionanywhere.data.RustDiscoveryRepository
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
    )
    val account: AccountRepository = accountImpl
    val library: LibraryRepository = RustLibraryRepository(accountImpl)
    val players: PlayersRepository = RustPlayersRepository(accountImpl)
    val art: ArtRepository = RustArtRepository(accountImpl)
    val events: EventsRepository = RustEventsRepository(accountImpl)
    val analysis: AnalysisRepository = RustAnalysisRepository(accountImpl)
    val menus: MenusRepository = RustMenusRepository(accountImpl)

    /** "Start radio" and "More like this" (A6). */
    val discovery: DiscoveryRepository = RustDiscoveryRepository(accountImpl) { android.os.SystemClock.elapsedRealtime() }

    /** The LMS virtual library browsed (offered only when LMS has any). */
    val libraries = VirtualLibraries(library, settings)

    /** How usable each app is here, remembered for a while (Apps & Radio). */
    val suitability = SuitabilityCache(clock = { android.os.SystemClock.elapsedRealtime() })

    /** This phone as an LMS player, while signed in and switched on. */
    val localPlayer: LocalPlayerRepository = RustLocalPlayerRepository(
        sessions = accountImpl.connection
            .map { it?.session }
            .stateIn(scope, SharingStarted.Eagerly, null),
        settings = settings,
        scope = scope,
        io = Dispatchers.IO,
    )

    /** Wi-Fi, mobile data or none (for download formats, "Wi-Fi only" and offline mode). */
    val network: NetworkMonitor = AndroidNetworkMonitor(app)

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
    val offlineMode = OfflineModeController(
        scope = scope,
        auth = accountImpl.state,
        network = network.kind,
        serverTrouble = localPlayer.state.map(OfflineModeController::troubleOf),
        retry = { accountImpl.retry() },
    ).also { it.start() }

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
