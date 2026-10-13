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
        DataStoreSettings(restart()).setLibraryPrefs(LibraryPrefs(LibraryChip.ARTISTS, LibraryAlbumOrder.YEAR, albumGrid = false))
        assertEquals(
            LibraryPrefs(LibraryChip.ARTISTS, LibraryAlbumOrder.YEAR, albumGrid = false),
            DataStoreSettings(restart()).settings.first().library,
        )
        // Back to the overview (no chip).
        DataStoreSettings(restart()).setLibraryPrefs(LibraryPrefs(null, LibraryAlbumOrder.NEWEST))
        assertEquals(LibraryPrefs(null, LibraryAlbumOrder.NEWEST), DataStoreSettings(restart()).settings.first().library)
    }

    @Test
    fun storedNonsenseIsNeverUsed() = runBlocking {
        val s = store()
        s.edit {
            it[stringPreferencesKey("library_chip")] = "EVERYTHING"
            it[stringPreferencesKey("library_album_sort")] = "RANDOM"
            it[booleanPreferencesKey("library_album_grid")] = false
        }
        assertEquals(LibraryPrefs(null, LibraryAlbumOrder.TITLE, albumGrid = false), DataStoreSettings(s).settings.first().library)
    }

    @Test
    fun recentlyPlayedIsKeptAcrossARestart() = runBlocking {
        DataStoreSettings(store()).setLibraryPrefs(LibraryPrefs(LibraryChip.ALBUMS, LibraryAlbumOrder.RECENTLY_PLAYED))
        assertEquals(
            LibraryPrefs(LibraryChip.ALBUMS, LibraryAlbumOrder.RECENTLY_PLAYED),
            DataStoreSettings(restart()).settings.first().library,
        )
    }

    @Test
    fun choicesSavedBeforeRecentlyPlayedExistedStillLoad() = runBlocking {
        // Older versions stored the LMS order's name.
        for ((stored, order) in listOf(
            "NEWEST" to LibraryAlbumOrder.NEWEST,
            "TITLE" to LibraryAlbumOrder.TITLE,
            "ARTIST" to LibraryAlbumOrder.ARTIST,
            "YEAR" to LibraryAlbumOrder.YEAR,
        )) {
            val s = restart()
            s.edit { it[stringPreferencesKey("library_album_sort")] = stored }
            assertEquals(order, DataStoreSettings(s).settings.first().library.albumOrder)
        }
    }

    @Test
    fun unknownStoredOrdersFallBackToTheDefault() = runBlocking {
        // LMS's server-wide "recently played", random, other spellings, junk: never used.
        for (stored in listOf("RANDOM", "RECENTLYPLAYED", "recentlyplayed", "recently_played", "", " TITLE", "\u0000", "x".repeat(5000))) {
            val s = restart()
            s.edit { it[stringPreferencesKey("library_album_sort")] = stored }
            assertEquals(stored, LibraryAlbumOrder.TITLE, DataStoreSettings(s).settings.first().library.albumOrder)
        }
    }

    @Test
    fun theOrdersAreOfferedInTheAgreedSequenceAndOnlyLmsOnesMapToAnLmsSort() {
        assertEquals(
            listOf(
                LibraryAlbumOrder.NEWEST,
                LibraryAlbumOrder.RECENTLY_PLAYED,
                LibraryAlbumOrder.TITLE,
                LibraryAlbumOrder.ARTIST,
                LibraryAlbumOrder.YEAR,
            ),
            LibraryAlbumOrder.OFFERED,
        )
        assertEquals(null, LibraryAlbumOrder.RECENTLY_PLAYED.lmsSort)
        assertEquals(AlbumSort.NEWEST, LibraryAlbumOrder.NEWEST.lmsSort)
        assertEquals(AlbumSort.TITLE, LibraryAlbumOrder.TITLE.lmsSort)
        assertEquals(AlbumSort.ARTIST, LibraryAlbumOrder.ARTIST.lmsSort)
        assertEquals(AlbumSort.YEAR, LibraryAlbumOrder.YEAR.lmsSort)
        // Never LMS's random (a page of its own) for the Library.
        assertTrue(LibraryAlbumOrder.entries.none { it.lmsSort == AlbumSort.RANDOM })
        assertEquals(50, LibraryAlbumOrder.RECENTLY_PLAYED_MAX)
        assertEquals(LibraryAlbumOrder.TITLE, LibraryAlbumOrder.fromStored(null))
    }

    @Test
    fun fromStoredNeverThrowsOnArbitraryInput() {
        val r = Random(7)
        repeat(2000) {
            val s = String(CharArray(r.nextInt(0, 40)) { r.nextInt(0, 0x3000).toChar() })
            assertTrue(LibraryAlbumOrder.fromStored(s) in LibraryAlbumOrder.OFFERED)
        }
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
