package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.MutablePreferences
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.doublePreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/** Non-secret settings: which server, which player is being controlled, whether this
 *  phone is a player itself, and its sound (DSP and bit-perfect output). */
data class AppSettings(
    val serverUrl: String? = null,
    val selectedPlayerId: String? = null,
    val playOnPhone: Boolean = false,
    /** This phone's player's sound (default: bit-perfect). */
    val sound: SoundSettings = SoundSettings(),
)

interface SettingsStore {
    val settings: Flow<AppSettings>

    suspend fun setServerUrl(url: String?)

    suspend fun setSelectedPlayer(playerId: String?)

    suspend fun setPlayOnPhone(on: Boolean)

    suspend fun setSound(sound: SoundSettings)
}

class DataStoreSettings(private val store: DataStore<Preferences>) : SettingsStore {
    override val settings: Flow<AppSettings> = store.data.map {
        AppSettings(
            serverUrl = it[SERVER],
            selectedPlayerId = it[PLAYER],
            playOnPhone = it[PLAY_ON_PHONE] ?: false,
            sound = readSound(it),
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

    override suspend fun setSound(sound: SoundSettings) {
        val s = sound.sanitised()
        store.edit { p -> writeSound(p, s) }
    }

    private companion object {
        val SERVER = stringPreferencesKey("server_url")
        val PLAYER = stringPreferencesKey("selected_player")
        val PLAY_ON_PHONE = booleanPreferencesKey("play_on_phone")

        /** A1–A3's single levelling switch; read once to migrate, then removed. */
        val OLD_LEVELLING = booleanPreferencesKey("loudness_levelling")

        val PREAMP = doublePreferencesKey("sound_preamp_db")
        val LEVELLING = stringPreferencesKey("sound_levelling")
        val LIMITER = booleanPreferencesKey("sound_limiter")
        val CEILING = doublePreferencesKey("sound_limiter_ceiling_db")
        val EQ_ON = booleanPreferencesKey("sound_eq_on")
        val EQ_NAME = stringPreferencesKey("sound_eq_name")
        val EQ_TEXT = stringPreferencesKey("sound_eq_text")
        val EQ_FILTERS = intPreferencesKey("sound_eq_filters")
        val EQ_PREAMP = doublePreferencesKey("sound_eq_preamp_db")
        val CROSSFEED = stringPreferencesKey("sound_crossfeed")
        val DITHER = stringPreferencesKey("sound_dither")
        val BIT_PERFECT_USB = booleanPreferencesKey("sound_bit_perfect_usb")

        /** An enum stored by name; anything unknown (an older or newer app) is the default. */
        inline fun <reified T : Enum<T>> enumOr(name: String?, default: T): T =
            enumValues<T>().firstOrNull { it.name == name } ?: default

        fun readSound(p: Preferences): SoundSettings {
            val d = SoundSettings()
            val levelling = p[LEVELLING]?.let { enumOr(it, LevellingMode.OFF) }
                ?: if (p[OLD_LEVELLING] == true) LevellingMode.R128 else LevellingMode.OFF
            val text = p[EQ_TEXT]
            val eq = if (text != null) {
                EqProfile(
                    name = p[EQ_NAME] ?: "",
                    text = text,
                    filters = p[EQ_FILTERS] ?: 0,
                    preampDb = p[EQ_PREAMP] ?: 0.0,
                )
            } else {
                null
            }
            return SoundSettings(
                preampDb = p[PREAMP] ?: d.preampDb,
                levelling = levelling,
                limiter = p[LIMITER] ?: d.limiter,
                limiterCeilingDb = p[CEILING] ?: d.limiterCeilingDb,
                eqEnabled = p[EQ_ON] ?: false,
                eq = eq,
                crossfeed = enumOr(p[CROSSFEED], d.crossfeed),
                dither = enumOr(p[DITHER], d.dither),
                bitPerfectUsb = p[BIT_PERFECT_USB] ?: d.bitPerfectUsb,
            ).sanitised()
        }

        fun writeSound(p: MutablePreferences, s: SoundSettings) {
            p.remove(OLD_LEVELLING)
            p[PREAMP] = s.preampDb
            p[LEVELLING] = s.levelling.name
            p[LIMITER] = s.limiter
            p[CEILING] = s.limiterCeilingDb
            p[EQ_ON] = s.eqEnabled
            val eq = s.eq
            if (eq == null) {
                p.remove(EQ_NAME)
                p.remove(EQ_TEXT)
                p.remove(EQ_FILTERS)
                p.remove(EQ_PREAMP)
            } else {
                p[EQ_NAME] = eq.name
                p[EQ_TEXT] = eq.text
                p[EQ_FILTERS] = eq.filters
                p[EQ_PREAMP] = eq.preampDb
            }
            p[CROSSFEED] = s.crossfeed.name
            p[DITHER] = s.dither.name
            p[BIT_PERFECT_USB] = s.bitPerfectUsb
        }
    }
}
