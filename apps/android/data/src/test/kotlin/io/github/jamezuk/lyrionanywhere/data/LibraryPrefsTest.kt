package io.github.jamezuk.lyrionanywhere.data

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.booleanPreferencesKey
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.stringPreferencesKey
import io.github.jamezuk.lyrionanywhere.core.AlbumSort
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.job
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import kotlin.random.Random

/**
 * Wave 2 §3.2–3.3 on the device: the Library's remembered chip, order and view, and recent
 * searches (on this device only, at most 20, each once, removable, cleared with the server).
 */
class LibraryPrefsTest {
    @get:Rule val tmp = TemporaryFolder()

    private val scopes = mutableListOf<CoroutineScope>()

    private fun store(): DataStore<Preferences> {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO).also { scopes += it }
        return PreferenceDataStoreFactory.create(scope = scope) { tmp.root.resolve("settings.preferences_pb") }
    }

    private fun restart(): DataStore<Preferences> {
        runBlocking { scopes.forEach { it.coroutineContext.job.cancelAndJoin() } }
        scopes.clear()
        return store()
    }

    @After
    fun tearDown() = scopes.forEach { it.cancel() }

    @Test
    fun theLibraryChoiceSurvivesARestart() = runBlocking {
        assertEquals(LibraryPrefs(), DataStoreSettings(store()).settings.first().library)
        DataStoreSettings(restart()).setLibraryPrefs(LibraryPrefs(LibraryChip.ARTISTS, AlbumSort.YEAR, albumGrid = false))
        assertEquals(
            LibraryPrefs(LibraryChip.ARTISTS, AlbumSort.YEAR, albumGrid = false),
            DataStoreSettings(restart()).settings.first().library,
        )
        // Back to the overview (no chip).
        DataStoreSettings(restart()).setLibraryPrefs(LibraryPrefs(null, AlbumSort.NEWEST))
        assertEquals(LibraryPrefs(null, AlbumSort.NEWEST), DataStoreSettings(restart()).settings.first().library)
    }

    @Test
    fun storedNonsenseIsNeverUsed() = runBlocking {
        val s = store()
        s.edit {
            it[stringPreferencesKey("library_chip")] = "EVERYTHING"
            it[stringPreferencesKey("library_album_sort")] = "RANDOM"
            it[booleanPreferencesKey("library_album_grid")] = false
        }
        assertEquals(LibraryPrefs(null, AlbumSort.TITLE, albumGrid = false), DataStoreSettings(s).settings.first().library)
        // Random isn't a Library order, and isn't kept either.
        DataStoreSettings(s).setLibraryPrefs(LibraryPrefs(LibraryChip.ALBUMS, AlbumSort.RANDOM))
        assertEquals(AlbumSort.TITLE, DataStoreSettings(s).settings.first().library.albumSort)
    }

    @Test
    fun recentSearchesAreKeptNewestFirstEachOnceAtMostTwenty() = runBlocking {
        val s = DataStoreSettings(store())
        s.addRecentSearch("blue")
        s.addRecentSearch("  red\n  moon ")
        s.addRecentSearch("BLUE")
        assertEquals(listOf("BLUE", "red moon"), s.settings.first().recentSearches)
        repeat(30) { s.addRecentSearch("term $it") }
        val list = DataStoreSettings(restart()).settings.first().recentSearches
        assertEquals(RecentSearches.MAX, list.size)
        assertEquals("term 29", list.first())
        val again = DataStoreSettings(restart())
        again.removeRecentSearch("TERM 29")
        assertEquals("term 28", again.settings.first().recentSearches.first())
        again.clearRecentSearches()
        assertTrue(again.settings.first().recentSearches.isEmpty())
    }

    @Test
    fun anotherServerForgetsTheSearches() = runBlocking {
        val s = DataStoreSettings(store())
        s.addRecentSearch("blue")
        s.setServerUrl("https://music.example.com/")
        assertTrue(s.settings.first().recentSearches.isEmpty())
    }

    @Test
    fun aTamperedListIsCleanedWhenRead() = runBlocking {
        val st = store()
        val sep = RecentSearches.SEPARATOR
        st.edit { it[stringPreferencesKey("recent_searches")] = "a${sep}${sep}\u0000b\u0007${sep}A$sep" + "x".repeat(5000) }
        val list = DataStoreSettings(st).settings.first().recentSearches
        assertEquals(listOf("a", "b", "x".repeat(RecentSearches.MAX_LENGTH)), list)
    }

    /** Whatever goes in, what comes out is a short, clean list: never a crash. */
    @Test
    fun recentSearchesSurviveArbitraryInput() {
        val r = Random(7)
        var list = emptyList<String>()
        repeat(5_000) {
            val term = String(CharArray(r.nextInt(0, 300)) { r.nextInt(0, 0x3000).toChar() })
            list = when (r.nextInt(4)) {
                0 -> RecentSearches.remove(list, term)
                1 -> RecentSearches.decode(RecentSearches.encode(list) + term)
                else -> RecentSearches.add(list, term)
            }
            assertTrue(list.size <= RecentSearches.MAX)
            assertTrue(list.all { t -> t.isNotBlank() && t.length <= RecentSearches.MAX_LENGTH && t.none { it.isISOControl() } })
            assertEquals(list.size, list.map { it.lowercase() }.toSet().size)
        }
    }
}
