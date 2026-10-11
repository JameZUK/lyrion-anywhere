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
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
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

    /** This phone as an LMS player, while signed in and switched on. */
    val localPlayer: LocalPlayerRepository = RustLocalPlayerRepository(
        sessions = accountImpl.connection
            .map { it?.session }
            .stateIn(scope, SharingStarted.Eagerly, null),
        settings = settings,
        scope = scope,
        io = Dispatchers.IO,
    )

    /** What this phone is called in LMS and the device list, e.g. "Google Pixel 8". */
    private fun deviceName(): String {
        val maker = Build.MANUFACTURER.replaceFirstChar { it.uppercase() }
        val model = Build.MODEL
        return if (model.startsWith(maker, ignoreCase = true)) model else "$maker $model"
    }
}
