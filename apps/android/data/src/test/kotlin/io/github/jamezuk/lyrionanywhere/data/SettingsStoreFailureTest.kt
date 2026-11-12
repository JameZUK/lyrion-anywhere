package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.Preferences
import java.io.IOException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import org.junit.Rule
import org.junit.rules.TemporaryFolder

/**
 * The settings file going bad never stops the app (review, 12 November): a corrupt file is
 * replaced by the defaults (and is written again), and a disk that fails to read or write
 * gives the defaults and loses only that change.
 */
class SettingsStoreFailureTest {
    @get:Rule val tmp = TemporaryFolder()

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    @After
    fun tearDown() = scope.cancel()

    @Test
    fun aCorruptFileIsReplacedByTheDefaults() = runBlocking {
        val file = tmp.root.resolve("settings.preferences_pb")
        // Not a preferences file at all (a truncated write, a bad block).
        file.writeBytes(byteArrayOf(0x0A, 0x7F, 0x12, 0x00, 0xFF.toByte(), 0x13, 0x37))
        val s = DataStoreSettings(settingsDataStore(scope) { file })
        assertEquals(AppSettings(), s.settings.first())
        // The file works again: a change is kept.
        s.setPlayOnPhone(false)
        assertFalse(s.settings.first().playOnPhone)
    }

    /** A disk that fails every read and write. */
    private class FailingStore : DataStore<Preferences> {
        override val data: Flow<Preferences> = flow { throw IOException("test: I/O error") }

        override suspend fun updateData(transform: suspend (t: Preferences) -> Preferences): Preferences =
            throw IOException("test: no space left on device")
    }

    @Test
    fun aFailingDiskGivesTheDefaultsAndLosesOnlyTheChange() = runBlocking {
        val s = DataStoreSettings(FailingStore())
        assertEquals(AppSettings(), s.settings.first())
        // Neither throws.
        s.setPlayOnPhone(false)
        s.setServerUrl("https://music.example.com/")
        s.addRecentSearch("Kind of Blue")
        assertEquals(AppSettings(), s.settings.first())
    }

    @Test
    fun otherFailuresAreNotHidden() = runBlocking {
        val broken = object : DataStore<Preferences> {
            override val data: Flow<Preferences> = flow { throw IllegalStateException("a bug") }
            override suspend fun updateData(transform: suspend (t: Preferences) -> Preferences): Preferences =
                throw IllegalStateException("a bug")
        }
        val s = DataStoreSettings(broken)
        assertEquals("a bug", runCatching { s.settings.first() }.exceptionOrNull()?.message)
        assertEquals("a bug", runCatching { s.setPlayOnPhone(true) }.exceptionOrNull()?.message)
    }
}
