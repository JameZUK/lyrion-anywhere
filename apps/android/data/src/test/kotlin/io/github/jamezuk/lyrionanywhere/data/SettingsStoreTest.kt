package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.doublePreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
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
