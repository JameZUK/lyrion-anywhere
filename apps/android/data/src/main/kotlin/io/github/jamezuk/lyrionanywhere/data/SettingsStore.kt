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
    /** Gaps in the signal: reading the next track ahead (rolling cache, Phase A). */
    val readAhead: ReadAheadSettings = ReadAheadSettings(),
    /** The Library page's chip, album order and view (Wave 2 §3.2). */
    val library: LibraryPrefs = LibraryPrefs(),
    /** Recent searches, newest first (on this device only; Wave 2 §3.3). */
    val recentSearches: List<String> = emptyList(),
    /** Now playing's layout, visualiser and queue panel (on this device; Wave 2 §4). */
    val nowPlaying: NowPlayingPrefs = NowPlayingPrefs(),
    /** Haptics, the theme's colours and car mode (on this device; Wave 2 §5). */
    val device: DevicePrefs = DevicePrefs(),
)

/** How car mode opens by itself (Wave 2 §5.1). */
enum class CarAutoOpen {
    /** Only from its button. */
    OFF,

    /** When the phone connects to any car audio over Bluetooth. */
    ANY_CAR,

    /** When the phone connects to one chosen paired device. */
    THIS_DEVICE,
}

/**
 * Car mode's automatic opening. The chosen device's name and address are kept on this phone
 * only and never sent anywhere.
 */
data class CarModeSetting(
    val auto: CarAutoOpen = CarAutoOpen.OFF,
    val deviceName: String = "",
    val deviceAddress: String = "",
) {
    /** Checked: names cleaned, an address only for [CarAutoOpen.THIS_DEVICE] (else Off). */
    fun sanitised(): CarModeSetting = when (auto) {
        CarAutoOpen.THIS_DEVICE -> if (isBluetoothAddress(deviceAddress)) {
            CarModeSetting(auto, AudioOutput.cleanName(deviceName), deviceAddress.uppercase())
        } else {
            CarModeSetting()
        }
        else -> CarModeSetting(auto)
    }
}

/** A Bluetooth device address, `AA:BB:CC:DD:EE:FF`. */
fun isBluetoothAddress(v: String?): Boolean = isPlayerId(v?.lowercase())

/** This device's own choices that aren't about playback. */
data class DevicePrefs(
    /** Light ticks on skips, scrubbing, volume steps, drags, ratings and the sleep timer
     *  (on by default; the system's touch-feedback setting still applies). */
    val haptics: Boolean = true,
    /** The wallpaper's colours (Android 12+), else the app's own. */
    val dynamicColour: Boolean = true,
    val car: CarModeSetting = CarModeSetting(),
)

/** This phone's stream level by network: as sent on Wi-Fi, Opus 160 on mobile data. */
data class StreamingSettings(
    val wifi: StreamQuality = StreamQuality.ORIGINAL,
    val mobile: StreamQuality = StreamQuality.HIGH,
)

/**
 * Settings › Playback › Gaps in the signal (docs/plans/wave3-design.md, "Rolling cache"):
 * this phone's player fetches the next track long before the current one ends, so a gap in
 * the signal doesn't stop the music. Both on by default (the owner's decision 2).
 */
data class ReadAheadSettings(
    /** "Read ahead the next track". */
    val enabled: Boolean = true,
    /** "On mobile data too": also on a metered network. */
    val onMobileData: Boolean = true,
) {
    /** Whether the player reads ahead now: on, and on mobile data only if allowed there. */
    fun effective(metered: Boolean): Boolean = enabled && (!metered || onMobileData)
}

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

    /** Reading the next track ahead, and whether on mobile data too. */
    suspend fun setReadAhead(readAhead: ReadAheadSettings) {}

    /** The Library page's chip, album order and view (checked before they are kept). */
    suspend fun setLibraryPrefs(prefs: LibraryPrefs) {}

    /** Put a search first among the recent ones (at most 20 are kept). */
    suspend fun addRecentSearch(term: String) {}

    suspend fun removeRecentSearch(term: String) {}

    suspend fun clearRecentSearches() {}

    /** Now playing's layout, visualiser and queue panel. */
    suspend fun setNowPlayingPrefs(prefs: NowPlayingPrefs) {}

    /** Haptics, the theme's colours and car mode (checked before they are kept). */
    suspend fun setDevicePrefs(prefs: DevicePrefs) {}
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
            readAhead = ReadAheadSettings(
                enabled = it[READ_AHEAD] ?: true,
                onMobileData = it[READ_AHEAD_MOBILE] ?: true,
            ),
            library = LibraryPrefs(
                chip = it[LIB_CHIP]?.let { c -> LibraryChip.entries.firstOrNull { e -> e.name == c } },
                albumOrder = LibraryAlbumOrder.fromStored(it[LIB_SORT]),
                albumGrid = it[LIB_GRID] ?: true,
            ).sanitised(),
            recentSearches = RecentSearches.decode(it[RECENT_SEARCHES]),
            nowPlaying = NowPlayingPrefs(
                layout = enumOr(it[NP_LAYOUT], NowPlayingLayout.ART),
                visualiser = it[NP_VISUALISER] ?: false,
                visualStyle = enumOr(it[NP_STYLE], VisualStyle.BARS),
                queuePanel = it[NP_QUEUE_PANEL] ?: true,
            ),
            device = DevicePrefs(
                haptics = it[HAPTICS] ?: true,
                dynamicColour = it[DYNAMIC_COLOUR] ?: true,
                car = CarModeSetting(
                    auto = enumOr(it[CAR_AUTO], CarAutoOpen.OFF),
                    deviceName = it[CAR_NAME].orEmpty(),
                    deviceAddress = it[CAR_ADDRESS].orEmpty(),
                ).sanitised(),
            ),
        )
    }

    override suspend fun setDevicePrefs(prefs: DevicePrefs) {
        val car = prefs.car.sanitised()
        store.edit { e ->
            e[HAPTICS] = prefs.haptics
            e[DYNAMIC_COLOUR] = prefs.dynamicColour
            e[CAR_AUTO] = car.auto.name
            if (car.auto == CarAutoOpen.THIS_DEVICE) {
                e[CAR_NAME] = car.deviceName
                e[CAR_ADDRESS] = car.deviceAddress
            } else {
                // A device no longer chosen isn't kept.
                e.remove(CAR_NAME)
                e.remove(CAR_ADDRESS)
            }
        }
    }

    override suspend fun setNowPlayingPrefs(prefs: NowPlayingPrefs) {
        store.edit { e ->
            e[NP_LAYOUT] = prefs.layout.name
            e[NP_VISUALISER] = prefs.visualiser
            e[NP_STYLE] = prefs.visualStyle.name
            e[NP_QUEUE_PANEL] = prefs.queuePanel
        }
    }

    override suspend fun setLibraryPrefs(prefs: LibraryPrefs) {
        val p = prefs.sanitised()
        store.edit { e ->
            if (p.chip == null) e.remove(LIB_CHIP) else e[LIB_CHIP] = p.chip.name
            e[LIB_SORT] = p.albumOrder.name
            e[LIB_GRID] = p.albumGrid
        }
    }

    override suspend fun addRecentSearch(term: String) {
        store.edit { e -> e[RECENT_SEARCHES] = RecentSearches.encode(RecentSearches.add(RecentSearches.decode(e[RECENT_SEARCHES]), term)) }
    }

    override suspend fun removeRecentSearch(term: String) {
        store.edit { e -> e[RECENT_SEARCHES] = RecentSearches.encode(RecentSearches.remove(RecentSearches.decode(e[RECENT_SEARCHES]), term)) }
    }

    override suspend fun clearRecentSearches() {
        store.edit { e -> e.remove(RECENT_SEARCHES) }
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

    override suspend fun setReadAhead(readAhead: ReadAheadSettings) {
        store.edit { p ->
            p[READ_AHEAD] = readAhead.enabled
            p[READ_AHEAD_MOBILE] = readAhead.onMobileData
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
            // So do searches of its library.
            p.remove(RECENT_SEARCHES)
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
        val READ_AHEAD = booleanPreferencesKey("read_ahead")
        val READ_AHEAD_MOBILE = booleanPreferencesKey("read_ahead_mobile_data")
        val LIB_CHIP = stringPreferencesKey("library_chip")
        val LIB_SORT = stringPreferencesKey("library_album_sort")
        val LIB_GRID = booleanPreferencesKey("library_album_grid")
        val RECENT_SEARCHES = stringPreferencesKey("recent_searches")
        val NP_LAYOUT = stringPreferencesKey("now_playing_layout")
        val NP_VISUALISER = booleanPreferencesKey("now_playing_visualiser")
        val NP_STYLE = stringPreferencesKey("now_playing_visual_style")
        val NP_QUEUE_PANEL = booleanPreferencesKey("now_playing_queue_panel")
        val HAPTICS = booleanPreferencesKey("haptics")
        val DYNAMIC_COLOUR = booleanPreferencesKey("dynamic_colour")
        val CAR_AUTO = stringPreferencesKey("car_mode_auto")
        val CAR_NAME = stringPreferencesKey("car_mode_device_name")
        val CAR_ADDRESS = stringPreferencesKey("car_mode_device_address")
        val OUTPUT_EQ = stringPreferencesKey("sound_output_eq")
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
        val LOUDNESS = booleanPreferencesKey("sound_loudness")

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
                outputEq = OutputEq.decode(p[OUTPUT_EQ]),
                loudness = p[LOUDNESS] ?: d.loudness,
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
            if (s.outputEq.isEmpty()) p.remove(OUTPUT_EQ) else p[OUTPUT_EQ] = OutputEq.encode(s.outputEq)
            p[LOUDNESS] = s.loudness
        }
    }
}
