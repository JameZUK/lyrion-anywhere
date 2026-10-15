package io.github.jamezuk.lyrionanywhere.data.offline

import androidx.datastore.core.DataStore
import androidx.datastore.preferences.core.PreferenceDataStoreFactory
import androidx.datastore.preferences.core.Preferences
import androidx.datastore.preferences.core.edit
import androidx.datastore.preferences.core.intPreferencesKey
import androidx.datastore.preferences.core.longPreferencesKey
import androidx.datastore.preferences.core.stringPreferencesKey
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.data.DataStoreSettings
import io.github.jamezuk.lyrionanywhere.data.isOwnerName
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
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

/** Download settings and the offline owner survive a restart; stored nonsense is refused. */
class DownloadSettingsTest {
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
    fun defaultsMatchTheWebApp() = runBlocking {
        val d = DataStoreSettings(store()).settings.first().downloads
        assertEquals(DownloadFormat.ORIGINAL, d.wifiFormat)
        assertEquals(DownloadFormat.OPUS, d.mobileFormat)
        assertTrue(d.wifiOnly)
        assertEquals(0L, d.capBytes)
        assertFalse(d.smartOn)
        val p = d.policy()
        assertEquals(0uL, p.capBytes)
        assertEquals(DownloadFormat.OPUS, p.mobileFormat)
    }

    @Test
    fun downloadsAndTheOwnerSurviveARestart() = runBlocking {
        val s = DataStoreSettings(store())
        val d = DownloadSettings(
            wifiFormat = DownloadFormat.MP3,
            mobileFormat = DownloadFormat.MP3,
            wifiOnly = true,
            capBytes = 5 * DownloadSettings.GB,
            smartRecentAlbums = 10,
            smartFavourites = true,
        )
        s.setDownloads(d)
        s.setOfflineOwner("alice")
        val again = DataStoreSettings(restart()).settings.first()
        assertEquals(d, again.downloads)
        assertEquals("alice", again.offlineOwner)
        assertTrue(again.downloads.smartOn)
    }

    @Test
    fun aNewServerForgetsTheOwner() = runBlocking {
        val s = DataStoreSettings(store())
        s.setOfflineOwner("alice")
        s.setServerUrl("https://lms.example.lan/")
        assertNull(s.settings.first().offlineOwner)
        s.setOfflineOwner("bob")
        s.setOfflineOwner(null)
        assertNull(s.settings.first().offlineOwner)
    }

    @Test
    fun storedNonsenseIsRefused() = runBlocking {
        val ds = store()
        ds.edit {
            it[stringPreferencesKey("downloads_wifi_format")] = "flac-ultra"
            it[longPreferencesKey("downloads_cap_bytes")] = -7
            it[intPreferencesKey("downloads_smart_recent")] = 100_000
            it[stringPreferencesKey("offline_owner")] = "eve\u0000\n"
        }
        val a = DataStoreSettings(ds).settings.first()
        assertEquals(DownloadFormat.ORIGINAL, a.downloads.wifiFormat)
        assertEquals(0L, a.downloads.capBytes)
        assertEquals(DownloadSettings.MAX_RECENT, a.downloads.smartRecentAlbums)
        assertNull(a.offlineOwner)
        assertFalse(isOwnerName(""))
        assertFalse(isOwnerName("x".repeat(129)))
        assertTrue(isOwnerName("alice"))
        // Refused when written, too.
        DataStoreSettings(ds).setOfflineOwner("a\nb")
        assertNull(DataStoreSettings(ds).settings.first().offlineOwner)
    }

    @Test
    fun networksMapToDownloadRules() {
        assertEquals(NetworkKind.OFFLINE, networkKind(hasInternet = false, unmetered = true))
        assertEquals(NetworkKind.WIFI, networkKind(hasInternet = true, unmetered = true))
        assertEquals(NetworkKind.MOBILE, networkKind(hasInternet = true, unmetered = false))
    }

    @Test
    fun choicesAreSane() {
        assertEquals(0L, DownloadSettings.CAP_CHOICES.first())
        assertTrue(DownloadSettings.CAP_CHOICES.zipWithNext().all { (a, b) -> a < b })
        assertTrue(DownloadSettings.RECENT_CHOICES.all { it in 0..DownloadSettings.MAX_RECENT })
    }
}
