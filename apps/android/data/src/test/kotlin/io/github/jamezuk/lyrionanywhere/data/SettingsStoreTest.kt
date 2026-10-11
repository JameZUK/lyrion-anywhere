package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.doublePreferencesKey
import androidx.datastore.preferences.core.edit
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
}
