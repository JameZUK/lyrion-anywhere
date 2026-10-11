package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/** Non-secret settings: which server, which player is being controlled, whether this
 *  phone is a player itself, and its loudness levelling. */
data class AppSettings(
    val serverUrl: String? = null,
    val selectedPlayerId: String? = null,
    val playOnPhone: Boolean = false,
    /** Level loudness on this phone's player (off: bit-perfect). */
    val loudnessLevelling: Boolean = false,
)

interface SettingsStore {
    val settings: Flow<AppSettings>

    suspend fun setServerUrl(url: String?)

    suspend fun setSelectedPlayer(playerId: String?)

    suspend fun setPlayOnPhone(on: Boolean)

    suspend fun setLoudnessLevelling(on: Boolean)
}

class DataStoreSettings(private val store: DataStore<Preferences>) : SettingsStore {
    override val settings: Flow<AppSettings> = store.data.map {
        AppSettings(
            serverUrl = it[SERVER],
            selectedPlayerId = it[PLAYER],
            playOnPhone = it[PLAY_ON_PHONE] ?: false,
            loudnessLevelling = it[LEVELLING] ?: false,
        )
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

    override suspend fun setPlayOnPhone(on: Boolean) {
        store.edit { p -> p[PLAY_ON_PHONE] = on }
    }

    override suspend fun setLoudnessLevelling(on: Boolean) {
        store.edit { p -> p[LEVELLING] = on }
    }

    private companion object {
        val SERVER = stringPreferencesKey("server_url")
        val PLAYER = stringPreferencesKey("selected_player")
        val PLAY_ON_PHONE = booleanPreferencesKey("play_on_phone")
        val LEVELLING = booleanPreferencesKey("loudness_levelling")
    }
}
