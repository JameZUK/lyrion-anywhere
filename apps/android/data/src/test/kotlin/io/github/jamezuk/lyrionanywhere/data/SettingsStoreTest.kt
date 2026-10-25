package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.doublePreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.job
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** The Sound settings survive a restart (a new DataStore on the same file), old settings
 *  migrate, and stored values that are out of range or unknown are refused. */
class SettingsStoreTest {
    @get:Rule val tmp = TemporaryFolder()

    private val scopes = mutableListOf<CoroutineScope>()

    private fun store(): DataStore<Preferences> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO).also { scopes += it }
        return PreferenceDataStoreFactory.create(scope = scope) { tmp.root.resolve("settings.preferences_pb") }
    }

    /** "Restart": drop the old store (one per file per process) and open the file again. */
    private fun restart(): DataStore<Preferences> {
        runBlocking { scopes.forEach { it.coroutineContext.job.cancelAndJoin() } }
        scopes.clear()
        return store()
    }

    @After
    fun tearDown() = scopes.forEach { it.cancel() }

    /** "Fix output level at 100%": on by default (and for an older file without it), and kept off. */
    @Test
    fun fixedOutputLevelDefaultsOnAndSurvivesARestart() = runBlocking {
        assertTrue(DataStoreSettings(store()).settings.first().sound.fixedOutputLevel)
        val first = DataStoreSettings(restart())
        first.setSound(first.settings.first().sound.copy(fixedOutputLevel = false))
        assertFalse(DataStoreSettings(restart()).settings.first().sound.fixedOutputLevel)
        val s = restart()
        s.edit { it[booleanPreferencesKey("sound_fixed_output_level")] = true }
        assertTrue(DataStoreSettings(s).settings.first().sound.fixedOutputLevel)
        // An older file with other Sound settings but not this one: the default (on).
        val old = restart()
        old.edit {
            it.remove(booleanPreferencesKey("sound_fixed_output_level"))
            it[booleanPreferencesKey("sound_loudness")] = true
        }
        assertTrue(DataStoreSettings(old).settings.first().sound.fixedOutputLevel)
    }

    /** The sleep timer's fade-out (Wave 1): 30 s by default, kept, and checked when read. */
    @Test
    fun sleepFadeDefaultsSurvivesARestartAndRefusesNonsense() = runBlocking {
        assertEquals(30, DataStoreSettings(store()).settings.first().sleepFadeSecs)
        DataStoreSettings(restart()).setSleepFade(90)
        assertEquals(90, DataStoreSettings(restart()).settings.first().sleepFadeSecs)
        DataStoreSettings(restart()).setSleepFade(0)
        assertEquals(0, DataStoreSettings(restart()).settings.first().sleepFadeSecs)
        // Out of range when written: the default is kept instead.
        DataStoreSettings(restart()).setSleepFade(5_000)
        assertEquals(30, DataStoreSettings(restart()).settings.first().sleepFadeSecs)
        // Stored nonsense (an older or tampered file) is never used.
        for (bad in listOf(-1, 121, Int.MAX_VALUE, Int.MIN_VALUE)) {
            val s = restart()
            s.edit { it[intPreferencesKey("sleep_fade_secs")] = bad }
            assertEquals("$bad", 30, DataStoreSettings(s).settings.first().sleepFadeSecs)
        }
    }

    /** Streaming quality: Original on Wi-Fi and High on mobile data by default, kept, and checked when read. */
    @Test
    fun streamingDefaultsSurviveARestartAndRefuseNonsense() = runBlocking {
        assertEquals(StreamingSettings(StreamQuality.ORIGINAL, StreamQuality.HIGH), DataStoreSettings(store()).settings.first().streaming)
        DataStoreSettings(restart()).setStreaming(StreamingSettings(StreamQuality.HIGH, StreamQuality.LOW))
        assertEquals(StreamingSettings(StreamQuality.HIGH, StreamQuality.LOW), DataStoreSettings(restart()).settings.first().streaming)
        val s = restart()
        s.edit {
            it[stringPreferencesKey("stream_quality_wifi")] = "ULTRA"
            it[stringPreferencesKey("stream_quality_mobile")] = ""
        }
        assertEquals(StreamingSettings(), DataStoreSettings(s).settings.first().streaming)
    }

    /** The replay cache's size (rolling cache, Phase B): 1 GB by default, kept, and checked. */
    @Test
    fun theReplayCacheIs1GbByDefaultSurvivesARestartAndRefusesNonsense() = runBlocking {
        assertEquals(ReplayCache.GB, DataStoreSettings(store()).settings.first().replayCacheBytes)
        DataStoreSettings(restart()).setReplayCacheBytes(0)
        assertEquals(0L, DataStoreSettings(restart()).settings.first().replayCacheBytes)
        DataStoreSettings(restart()).setReplayCacheBytes(5 * ReplayCache.GB)
        assertEquals(5 * ReplayCache.GB, DataStoreSettings(restart()).settings.first().replayCacheBytes)
        // Not one of the choices when written: the default is kept instead.
        DataStoreSettings(restart()).setReplayCacheBytes(123)
        assertEquals(ReplayCache.GB, DataStoreSettings(restart()).settings.first().replayCacheBytes)
        // Stored nonsense (another app version, damage) reads as the default.
        val s = restart()
        s.edit { it[androidx.datastore.preferences.core.longPreferencesKey("replay_cache_bytes")] = -5 }
        assertEquals(ReplayCache.GB, DataStoreSettings(s).settings.first().replayCacheBytes)
        assertEquals(listOf(0L, 500 * ReplayCache.MB, ReplayCache.GB, 2 * ReplayCache.GB, 5 * ReplayCache.GB), ReplayCache.CHOICES)
        for (v in listOf(null, -1L, 1L, Long.MAX_VALUE, 10 * ReplayCache.GB)) assertEquals(ReplayCache.GB, ReplayCache.sanitise(v))
    }

    /** Gaps in the signal (rolling cache): reading ahead on, also on mobile data, by default; both kept. */
    @Test
    fun readAheadIsOnByDefaultAndSurvivesARestart() = runBlocking {
        assertEquals(ReadAheadSettings(enabled = true, onMobileData = true), DataStoreSettings(store()).settings.first().readAhead)
        DataStoreSettings(restart()).setReadAhead(ReadAheadSettings(enabled = true, onMobileData = false))
        assertEquals(ReadAheadSettings(true, false), DataStoreSettings(restart()).settings.first().readAhead)
        DataStoreSettings(restart()).setReadAhead(ReadAheadSettings(enabled = false, onMobileData = false))
        assertEquals(ReadAheadSettings(false, false), DataStoreSettings(restart()).settings.first().readAhead)
        DataStoreSettings(restart()).setReadAhead(ReadAheadSettings())
        assertEquals(ReadAheadSettings(), DataStoreSettings(restart()).settings.first().readAhead)
        // Other settings are left alone.
        assertEquals(StreamingSettings(), DataStoreSettings(restart()).settings.first().streaming)
    }

    /** The lyrics peek (Wave 1): on by default, and kept. */
    @Test
    fun lyricsPeekIsOnByDefaultAndSurvivesARestart() = runBlocking {
        assertTrue(DataStoreSettings(store()).settings.first().lyricsPeek)
        DataStoreSettings(restart()).setLyricsPeek(false)
        assertFalse(DataStoreSettings(restart()).settings.first().lyricsPeek)
        DataStoreSettings(restart()).setLyricsPeek(true)
        assertTrue(DataStoreSettings(restart()).settings.first().lyricsPeek)
    }

    /** Now playing's choices (Wave 2 §4): Art, visualiser off, queue panel shown by default; kept; nonsense refused. */
    @Test
    fun nowPlayingPrefsDefaultSurviveARestartAndRefuseNonsense() = runBlocking {
        assertEquals(NowPlayingPrefs(), DataStoreSettings(store()).settings.first().nowPlaying)
        assertEquals(NowPlayingLayout.ART, NowPlayingPrefs().layout)
        assertFalse(NowPlayingPrefs().visualiser)
        val chosen = NowPlayingPrefs(NowPlayingLayout.INFO, visualiser = true, visualStyle = VisualStyle.VU, queuePanel = false)
        DataStoreSettings(restart()).setNowPlayingPrefs(chosen)
        assertEquals(chosen, DataStoreSettings(restart()).settings.first().nowPlaying)
        val s = restart()
        s.edit {
            it[stringPreferencesKey("now_playing_layout")] = "KARAOKE"
            it[stringPreferencesKey("now_playing_visual_style")] = ""
        }
        val read = DataStoreSettings(s).settings.first().nowPlaying
        assertEquals(NowPlayingLayout.ART, read.layout)
        assertEquals(VisualStyle.BARS, read.visualStyle)
        assertTrue(read.visualiser)
    }

    /**
     * This device's choices (Wave 2 §5): haptics and wallpaper colours on, car mode Off by
     * default; kept; a chosen car needs a real address; nonsense refused.
     */
    @Test
    fun devicePrefsDefaultSurviveARestartAndRefuseNonsense() = runBlocking {
        assertEquals(DevicePrefs(), DataStoreSettings(store()).settings.first().device)
        assertTrue(DevicePrefs().haptics)
        assertTrue(DevicePrefs().dynamicColour)
        assertEquals(CarAutoOpen.OFF, DevicePrefs().car.auto)
        val car = CarModeSetting(CarAutoOpen.THIS_DEVICE, "Example Car Kit", "0a:1b:2c:3d:4e:5f")
        DataStoreSettings(restart()).setDevicePrefs(DevicePrefs(haptics = false, dynamicColour = false, car = car))
        val read = DataStoreSettings(restart()).settings.first().device
        assertFalse(read.haptics)
        assertFalse(read.dynamicColour)
        assertEquals(CarModeSetting(CarAutoOpen.THIS_DEVICE, "Example Car Kit", "0A:1B:2C:3D:4E:5F"), read.car)
        // Choosing Any car forgets the device.
        DataStoreSettings(restart()).setDevicePrefs(read.copy(car = CarModeSetting(CarAutoOpen.ANY_CAR, "Example Car Kit", "0A:1B:2C:3D:4E:5F")))
        assertEquals(CarModeSetting(CarAutoOpen.ANY_CAR), DataStoreSettings(restart()).settings.first().device.car)
        // A device with a bad address isn't kept: Off.
        for (bad in listOf("", "0A:1B:2C:3D:4E", "0A:1B:2C:3D:4E:5G", "../../etc", "0A-1B-2C-3D-4E-5F")) {
            DataStoreSettings(restart()).setDevicePrefs(DevicePrefs(car = CarModeSetting(CarAutoOpen.THIS_DEVICE, "Car", bad)))
            assertEquals(bad, CarModeSetting(), DataStoreSettings(restart()).settings.first().device.car)
        }
        // Stored nonsense reads as the defaults.
        val s = restart()
        s.edit {
            it[stringPreferencesKey("car_mode_auto")] = "ALWAYS"
            it[stringPreferencesKey("car_mode_device_address")] = "not an address"
        }
        assertEquals(CarModeSetting(), DataStoreSettings(s).settings.first().device.car)
        s.edit {
            it[stringPreferencesKey("car_mode_auto")] = "THIS_DEVICE"
            it[stringPreferencesKey("car_mode_device_name")] = "Car\u0000\n" + "x".repeat(200)
            it[stringPreferencesKey("car_mode_device_address")] = "0a:1b:2c:3d:4e:5f"
        }
        val messy = DataStoreSettings(s).settings.first().device.car
        assertEquals("0A:1B:2C:3D:4E:5F", messy.deviceAddress)
        assertTrue(messy.deviceName.none { it.isISOControl() } && messy.deviceName.length <= 64)
    }

    /** Per-output EQ bindings (Wave 2 §5.3) are kept with the Sound settings. */
    @Test
    fun outputEqSurvivesARestart() = runBlocking {
        val p = EqProfile("Example profile", "Preamp: -3 dB\nFilter 1: ON PK Fc 1000 Hz Gain -3 dB Q 1\n", 1, -3.0)
        val sound = SoundSettings(
            eqEnabled = true,
            outputEq = listOf(OutputEq(OutputClass.BLUETOOTH, "Example Headphones", p), OutputEq(OutputClass.SPEAKER, "", null)),
        )
        DataStoreSettings(store()).setSound(sound)
        val read = DataStoreSettings(restart()).settings.first().sound
        assertEquals(sound.outputEq, read.outputEq)
        assertTrue(read.eqEnabled)
        // Removing every binding removes the stored value too.
        DataStoreSettings(restart()).setSound(read.copy(outputEq = emptyList()))
        assertTrue(DataStoreSettings(restart()).settings.first().sound.outputEq.isEmpty())
        // Junk stored there is skipped.
        val s = restart()
        s.edit { it[stringPreferencesKey("sound_output_eq")] = "\u001E\u001FWIRED\u001F" }
        assertTrue(DataStoreSettings(s).settings.first().sound.outputEq.isEmpty())
    }

    @Test
    fun soundDefaultsToBitPerfect() = runBlocking {
        val s = DataStoreSettings(store()).settings.first().sound
        assertEquals(SoundSettings(), s)
        assertTrue(s.bitPerfect)
        assertTrue(s.bitPerfectUsb)
    }

    @Test
    fun soundSurvivesARestart() = runBlocking {
        val eq = EqProfile("Headphones.txt", "Preamp: -3 dB\nFilter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1", 1, -3.0)
        val sound = SoundSettings(
            preampDb = -4.5,
            levelling = LevellingMode.REPLAYGAIN_ALBUM,
            limiter = false,
            limiterCeilingDb = -0.3,
            eqEnabled = true,
            eq = eq,
            crossfeed = CrossfeedMode.MODERATE,
            dither = DitherMode.TPDF_SHAPED,
            bitPerfectUsb = false,
        )
        DataStoreSettings(store()).setSound(sound)
        val again = DataStoreSettings(restart()).settings.first().sound
        assertEquals(sound, again)
        assertEquals(listOf(SoundStage.PREAMP, SoundStage.LEVELLING, SoundStage.EQ, SoundStage.CROSSFEED), again.activeStages)

        // Removing the profile removes it from storage too.
        DataStoreSettings(restart()).setSound(again.copy(eq = null, eqEnabled = false))
        val gone = DataStoreSettings(restart()).settings.first().sound
        assertNull(gone.eq)
        assertFalse(gone.eqEnabled)
    }

    /** Loudness-compensated volume is remembered on this device, and off when never set. */
    @Test
    fun loudnessSurvivesARestart() = runBlocking {
        assertFalse(DataStoreSettings(store()).settings.first().sound.loudness)
        DataStoreSettings(restart()).setSound(SoundSettings(loudness = true))
        val again = DataStoreSettings(restart()).settings.first().sound
        assertTrue(again.loudness)
        assertEquals(listOf(SoundStage.LOUDNESS), again.activeStages)
        // Other sound settings saved later keep it, and turning it off is kept too.
        DataStoreSettings(restart()).setSound(again.copy(crossfeed = CrossfeedMode.LIGHT))
        assertTrue(DataStoreSettings(restart()).settings.first().sound.loudness)
        DataStoreSettings(restart()).setSound(again.copy(loudness = false))
        assertFalse(DataStoreSettings(restart()).settings.first().sound.loudness)
    }

    @Test
    fun theOldLevellingSwitchMigratesToR128() = runBlocking {
        val s = store()
        s.edit { it[booleanPreferencesKey("loudness_levelling")] = true }
        val settings = DataStoreSettings(s)
        assertEquals(LevellingMode.R128, settings.settings.first().sound.levelling)
        // Saving writes the new form and drops the old key.
        settings.setSound(settings.settings.first().sound)
        assertFalse(s.data.first().contains(booleanPreferencesKey("loudness_levelling")))
        assertEquals(LevellingMode.R128, settings.settings.first().sound.levelling)
    }

    @Test
    fun storedNonsenseIsRefused() = runBlocking {
        val s = store()
        s.edit {
            it[doublePreferencesKey("sound_preamp_db")] = 1e9
            it[doublePreferencesKey("sound_limiter_ceiling_db")] = Double.NaN
            it[stringPreferencesKey("sound_levelling")] = "LOUDER"
            it[stringPreferencesKey("sound_crossfeed")] = ""
            it[stringPreferencesKey("sound_dither")] = "tpdf; DROP"
            it[booleanPreferencesKey("sound_eq_on")] = true
        }
        val sound = DataStoreSettings(s).settings.first().sound
        assertEquals(12.0, sound.preampDb, 0.0)
        assertEquals(-1.0, sound.limiterCeilingDb, 0.0)
        assertEquals(LevellingMode.OFF, sound.levelling)
        assertEquals(CrossfeedMode.OFF, sound.crossfeed)
        assertEquals(DitherMode.TPDF, sound.dither)
        assertFalse("no profile: the EQ can't be on", sound.eqEnabled)
        // And what is written is clamped first.
        DataStoreSettings(s).setSound(SoundSettings(preampDb = -50.0))
        assertEquals(-12.0, DataStoreSettings(s).settings.first().sound.preampDb, 0.0)
    }

    @Test
    fun transitionsSurviveARestartAndNonsenseIsRefused() = runBlocking {
        val sound = SoundSettings(transitions = TransitionsMode.SMART, crossfadeSecs = 4.0)
        DataStoreSettings(store()).setSound(sound)
        val again = DataStoreSettings(restart()).settings.first().sound
        assertEquals(sound, again)
        assertEquals(listOf(SoundStage.CROSSFADE), again.activeStages)
        assertFalse("a crossfade is never bit-perfect", again.bitPerfect)

        val s = restart()
        s.edit {
            it[stringPreferencesKey("sound_transitions")] = "FADE_EVERYTHING"
            it[doublePreferencesKey("sound_crossfade_secs")] = Double.POSITIVE_INFINITY
        }
        val bad = DataStoreSettings(s).settings.first().sound
        assertEquals("unknown modes fall back to gapless", TransitionsMode.OFF, bad.transitions)
        assertEquals(6.0, bad.crossfadeSecs, 0.0)
        s.edit { it[doublePreferencesKey("sound_crossfade_secs")] = 600.0 }
        assertEquals(10.0, DataStoreSettings(s).settings.first().sound.crossfadeSecs, 0.0)
    }

    @Test
    fun otherSettingsAreUntouchedBySound() = runBlocking {
        val settings = DataStoreSettings(store())
        settings.setServerUrl("https://music.example.com")
        settings.setPlayOnPhone(true)
        settings.setSound(SoundSettings(preampDb = -1.0))
        val all = settings.settings.first()
        assertEquals("https://music.example.com", all.serverUrl)
        assertTrue(all.playOnPhone)
        assertEquals(-1.0, all.sound.preampDb, 0.0)
    }

    @Test
    fun thisPhoneIsTheDefaultPlayerAndIsOn() = runBlocking {
        val all = DataStoreSettings(store()).settings.first()
        assertEquals(DefaultPlayer.THIS_PHONE, all.defaultPlayer)
        assertNull(all.defaultPlayerId)
        assertTrue(all.playOnPhone)
    }

    @Test
    fun theDefaultPlayerSurvivesARestartAndIsChecked() = runBlocking {
        val first = DataStoreSettings(store())
        first.setDefaultPlayer(DefaultPlayer.SPECIFIC, "AA:BB:CC:DD:EE:01")
        val again = DataStoreSettings(restart()).settings.first()
        assertEquals(DefaultPlayer.SPECIFIC, again.defaultPlayer)
        assertEquals("aa:bb:cc:dd:ee:01", again.defaultPlayerId)
        // A malformed player is refused; the setting is unchanged.
        val s = DataStoreSettings(restart())
        s.setDefaultPlayer(DefaultPlayer.SPECIFIC, "../x")
        assertEquals("aa:bb:cc:dd:ee:01", s.settings.first().defaultPlayerId)
        s.setDefaultPlayer(DefaultPlayer.LAST_USED, null)
        assertEquals(DefaultPlayer.LAST_USED, s.settings.first().defaultPlayer)
        assertNull(s.settings.first().defaultPlayerId)
    }

    @Test
    fun storedNonsenseForThePlayerAndLibraryIsIgnored() = runBlocking {
        val raw = store()
        raw.edit {
            it[stringPreferencesKey("default_player")] = "SPECIFIC"
            it[stringPreferencesKey("default_player_id")] = "not a player"
            it[stringPreferencesKey("virtual_library")] = "../../x"
        }
        val all = DataStoreSettings(raw).settings.first()
        assertEquals(DefaultPlayer.THIS_PHONE, all.defaultPlayer)
        assertNull(all.defaultPlayerId)
        assertNull(all.libraryId)
    }

    @Test
    fun aNewServerForgetsItsPlayersAndLibrary() = runBlocking {
        val s = DataStoreSettings(store())
        s.setLibrary("ab12")
        s.setLibrary("x y")
        assertEquals("ab12", s.settings.first().libraryId)
        s.setDefaultPlayer(DefaultPlayer.SPECIFIC, "aa:bb:cc:dd:ee:01")
        s.setServerUrl("https://music.example.com")
        val all = s.settings.first()
        assertNull(all.libraryId)
        assertEquals(DefaultPlayer.THIS_PHONE, all.defaultPlayer)
    }
}
