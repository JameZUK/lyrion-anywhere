package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/** Non-secret settings: which server, and which player is being controlled. */
data class AppSettings(
    val serverUrl: String? = null,
    val selectedPlayerId: String? = null,
)

interface SettingsStore {
    val settings: Flow<AppSettings>

    suspend fun setServerUrl(url: String?)

    suspend fun setSelectedPlayer(playerId: String?)
}

class DataStoreSettings(private val store: DataStore<Preferences>) : SettingsStore {
    override val settings: Flow<AppSettings> = store.data.map {
        AppSettings(serverUrl = it[SERVER], selectedPlayerId = it[PLAYER])
    }

    override suspend fun setServerUrl(url: String?) {
        store.edit { p ->
            if (url == null) p.remove(SERVER) else p[SERVER] = url
            // A different server has different players.
            p.remove(PLAYER)
        }
    }

    override suspend fun setSelectedPlayer(playerId: String?) {
        store.edit { p -> if (playerId == null) p.remove(PLAYER) else p[PLAYER] = playerId }
    }

    private companion object {
        val SERVER = stringPreferencesKey("server_url")
        val PLAYER = stringPreferencesKey("selected_player")
    }
}
