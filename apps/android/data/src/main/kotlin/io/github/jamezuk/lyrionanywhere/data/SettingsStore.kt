package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.MutablePreferences
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.doublePreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.data.offline.DownloadSettings
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.map

/** Non-secret settings: which server, which player is being controlled, whether this
 *  phone is a player itself, its sound (DSP and bit-perfect output), and which LMS virtual
 *  library to browse. */
data class AppSettings(
    val serverUrl: String? = null,
    val selectedPlayerId: String? = null,
    /** This phone is an LMS player (on by default: the phone is the default player, D20). */
    val playOnPhone: Boolean = true,
    /** This phone's player's sound (default: bit-perfect). */
    val sound: SoundSettings = SoundSettings(),
    /** The LMS virtual library browsed, or null for the whole library. */
    val libraryId: String? = null,
    /** Which player the app controls when it starts (D20). */
    val defaultPlayer: DefaultPlayer = DefaultPlayer.THIS_PHONE,
    /** The player for [DefaultPlayer.SPECIFIC]. */
    val defaultPlayerId: String? = null,
    /** Downloads: formats, Wi-Fi only, the storage cap and smart downloads. */
    val downloads: DownloadSettings = DownloadSettings(),
    /**
     * The user whose downloads offline mode shows when the server can't be reached before
     * signing in (the last one signed in on this server). Forgotten at sign-out.
     */
    val offlineOwner: String? = null,
    /** The sleep timer's fade-out on this phone's player, seconds (0–120; 0 is off). */
    val sleepFadeSecs: Int = SLEEP_FADE_DEFAULT,
    /** The current lyric line, small, on Now playing (when a track has synced lyrics). */
    val lyricsPeek: Boolean = true,
    /** The live stream's level on Wi-Fi and on mobile data (docs/plans/streaming-quality.md). */
    val streaming: StreamingSettings = StreamingSettings(),
)

/** This phone's stream level by network: as sent on Wi-Fi, Opus 160 on mobile data. */
data class StreamingSettings(
    val wifi: StreamQuality = StreamQuality.ORIGINAL,
    val mobile: StreamQuality = StreamQuality.HIGH,
)

/** The sleep timer's fade-out: 30 s by default, at most 2 minutes. */
const val SLEEP_FADE_DEFAULT = 30
const val SLEEP_FADE_MAX = 120

/** A stored fade-out length, checked: 0–120 seconds, else the default. */
fun sanitiseSleepFade(v: Int?): Int = if (v != null && v in 0..SLEEP_FADE_MAX) v else SLEEP_FADE_DEFAULT

/** Which player the app controls when it starts (per device). */
enum class DefaultPlayer {
    /** This phone (the default). */
    THIS_PHONE,

    /** Whichever was controlled last. */
    LAST_USED,

    /** One chosen player (this phone if it isn't available). */
    SPECIFIC,
}

/** An LMS player id (a MAC address). */
fun isPlayerId(v: String?): Boolean =
    v != null && v.length == 17 &&
        v.withIndex().all { (i, c) -> if (i % 3 == 2) c == ':' else c.isDigit() || c.lowercaseChar() in 'a'..'f' }

/** An LMS virtual library id: short hex, as the gateway requires. */
fun isLibraryId(v: String?): Boolean =
    v != null && v.length in 1..32 && v.all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' }

interface SettingsStore {
    val settings: Flow<AppSettings>

    suspend fun setServerUrl(url: String?)

    suspend fun setSelectedPlayer(playerId: String?)

    suspend fun setPlayOnPhone(on: Boolean)

    suspend fun setSound(sound: SoundSettings)

    /** Browse this virtual library (null: the whole library). A malformed id is ignored. */
    suspend fun setLibrary(id: String?)

    /** The default player; [playerId] is required for [DefaultPlayer.SPECIFIC]. */
    suspend fun setDefaultPlayer(choice: DefaultPlayer, playerId: String?)

    /** Download settings (sanitised before they are kept). */
    suspend fun setDownloads(downloads: DownloadSettings) {}

    /** Whose downloads offline mode may show (null: nobody's). */
    suspend fun setOfflineOwner(username: String?) {}

    /** The sleep timer's fade-out (seconds; checked before it is kept). */
    suspend fun setSleepFade(seconds: Int) {}

    /** Show the current lyric line on Now playing. */
    suspend fun setLyricsPeek(on: Boolean) {}

    /** The stream level on Wi-Fi and on mobile data. */
    suspend fun setStreaming(streaming: StreamingSettings) {}
}

/** A user name fit to remember as the offline owner. */
fun isOwnerName(v: String?): Boolean = v != null && v.length in 1..128 && v.none { it.isISOControl() }

class DataStoreSettings(private val store: DataStore<Preferences>) : SettingsStore {
    override val settings: Flow<AppSettings> = store.data.map {
        AppSettings(
            serverUrl = it[SERVER],
            selectedPlayerId = it[PLAYER],
            playOnPhone = it[PLAY_ON_PHONE] ?: true,
            sound = readSound(it),
            // Stored nonsense is never used.
            libraryId = it[LIBRARY]?.takeIf(::isLibraryId),
            defaultPlayer = readDefault(it),
            defaultPlayerId = if (readDefault(it) == DefaultPlayer.SPECIFIC) it[DEFAULT_PLAYER_ID] else null,
            downloads = readDownloads(it),
            offlineOwner = it[OFFLINE_OWNER]?.takeIf(::isOwnerName),
            sleepFadeSecs = sanitiseSleepFade(it[SLEEP_FADE]),
            lyricsPeek = it[LYRICS_PEEK] ?: true,
            streaming = StreamingSettings(
                wifi = enumOr(it[STREAM_WIFI], StreamingSettings().wifi),
                mobile = enumOr(it[STREAM_MOBILE], StreamingSettings().mobile),
            ),
        )
    }

    override suspend fun setSleepFade(seconds: Int) {
        store.edit { p -> p[SLEEP_FADE] = sanitiseSleepFade(seconds) }
    }

    override suspend fun setStreaming(streaming: StreamingSettings) {
        store.edit { p ->
            p[STREAM_WIFI] = streaming.wifi.name
            p[STREAM_MOBILE] = streaming.mobile.name
        }
    }

    override suspend fun setLyricsPeek(on: Boolean) {
        store.edit { p -> p[LYRICS_PEEK] = on }
    }

    override suspend fun setDownloads(downloads: DownloadSettings) {
        val d = downloads.sanitised()
        store.edit { p ->
            p[DL_WIFI] = d.wifiFormat.name
            p[DL_MOBILE] = d.mobileFormat.name
            p[DL_WIFI_ONLY] = d.wifiOnly
            p[DL_CAP] = d.capBytes
            p[DL_RECENT] = d.smartRecentAlbums
            p[DL_FAVOURITES] = d.smartFavourites
        }
    }

    override suspend fun setOfflineOwner(username: String?) {
        store.edit { p ->
            if (username != null && isOwnerName(username)) p[OFFLINE_OWNER] = username else p.remove(OFFLINE_OWNER)
        }
    }

    override suspend fun setServerUrl(url: String?) {
        store.edit { p ->
            if (url == null) p.remove(SERVER) else p[SERVER] = url
            // A different server has different players and libraries.
            p.remove(PLAYER)
            p.remove(LIBRARY)
            p.remove(DEFAULT_PLAYER_ID)
            if (p[DEFAULT_PLAYER] == DefaultPlayer.SPECIFIC.name) p.remove(DEFAULT_PLAYER)
            // Downloads belong to a user on a server.
            p.remove(OFFLINE_OWNER)
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

    override suspend fun setDefaultPlayer(choice: DefaultPlayer, playerId: String?) {
        if (choice == DefaultPlayer.SPECIFIC && !isPlayerId(playerId)) return
        store.edit { p ->
            p[DEFAULT_PLAYER] = choice.name
            if (choice == DefaultPlayer.SPECIFIC && playerId != null) {
                p[DEFAULT_PLAYER_ID] = playerId.lowercase()
            } else {
                p.remove(DEFAULT_PLAYER_ID)
            }
        }
    }

    override suspend fun setLibrary(id: String?) {
        if (id != null && !isLibraryId(id)) return
        store.edit { p -> if (id == null) p.remove(LIBRARY) else p[LIBRARY] = id }
    }

    private companion object {
        val SERVER = stringPreferencesKey("server_url")
        val PLAYER = stringPreferencesKey("selected_player")
        val PLAY_ON_PHONE = booleanPreferencesKey("play_on_phone")
        val LIBRARY = stringPreferencesKey("virtual_library")
        val DEFAULT_PLAYER = stringPreferencesKey("default_player")
        val DEFAULT_PLAYER_ID = stringPreferencesKey("default_player_id")
        val OFFLINE_OWNER = stringPreferencesKey("offline_owner")
        val SLEEP_FADE = intPreferencesKey("sleep_fade_secs")
        val LYRICS_PEEK = booleanPreferencesKey("lyrics_peek")
        val STREAM_WIFI = stringPreferencesKey("stream_quality_wifi")
        val STREAM_MOBILE = stringPreferencesKey("stream_quality_mobile")
        val DL_WIFI = stringPreferencesKey("downloads_wifi_format")
        val DL_MOBILE = stringPreferencesKey("downloads_mobile_format")
        val DL_WIFI_ONLY = booleanPreferencesKey("downloads_wifi_only")
        val DL_CAP = longPreferencesKey("downloads_cap_bytes")
        val DL_RECENT = intPreferencesKey("downloads_smart_recent")
        val DL_FAVOURITES = booleanPreferencesKey("downloads_smart_favourites")

        fun readDownloads(p: Preferences): DownloadSettings {
            val d = DownloadSettings()
            return DownloadSettings(
                wifiFormat = enumOr(p[DL_WIFI], d.wifiFormat),
                mobileFormat = enumOr<DownloadFormat>(p[DL_MOBILE], d.mobileFormat),
                wifiOnly = p[DL_WIFI_ONLY] ?: d.wifiOnly,
                capBytes = p[DL_CAP] ?: d.capBytes,
                smartRecentAlbums = p[DL_RECENT] ?: d.smartRecentAlbums,
                smartFavourites = p[DL_FAVOURITES] ?: d.smartFavourites,
            ).sanitised()
        }

        /** A specific default without a valid player is "this phone". */
        fun readDefault(p: Preferences): DefaultPlayer {
            val d = enumOr(p[DEFAULT_PLAYER], DefaultPlayer.THIS_PHONE)
            return if (d == DefaultPlayer.SPECIFIC && !isPlayerId(p[DEFAULT_PLAYER_ID])) DefaultPlayer.THIS_PHONE else d
        }

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
        val TRANSITIONS = stringPreferencesKey("sound_transitions")
        val CROSSFADE_SECS = doublePreferencesKey("sound_crossfade_secs")

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
                transitions = enumOr(p[TRANSITIONS], d.transitions),
                crossfadeSecs = p[CROSSFADE_SECS] ?: d.crossfadeSecs,
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
            p[TRANSITIONS] = s.transitions.name
            p[CROSSFADE_SECS] = s.crossfadeSecs
        }
    }
}
