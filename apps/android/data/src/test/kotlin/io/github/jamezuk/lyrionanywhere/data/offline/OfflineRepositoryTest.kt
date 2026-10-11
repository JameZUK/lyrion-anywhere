package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.DownloadRequest
import io.github.jamezuk.lyrionanywhere.core.DownloadsSnapshot
import io.github.jamezuk.lyrionanywhere.core.FavouriteLibraryItems
import io.github.jamezuk.lyrionanywhere.core.FormatChoice
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.PlaysSyncOutcome
import io.github.jamezuk.lyrionanywhere.core.SmartPlan
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AppSettings
import io.github.jamezuk.lyrionanywhere.data.AuthState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class OfflineRepositoryTest {
    private val auth = MutableStateFlow<AuthState>(AuthState.Starting)
    private val link = MutableStateFlow<OnlineLink?>(null)
    private val network = MutableStateFlow(NetworkKind.WIFI)
    private val settings = MemSettings()
    private val library = FakeLibrary()
    private val stores = mutableListOf<FakeStore>()
    private val wiped = mutableListOf<Owner>()
    private var now = 1_000_000L
    private val fakeLink = FakeLink()
    private var seed: (Owner) -> MutableList<io.github.jamezuk.lyrionanywhere.core.OfflineTrack> = { mutableListOf() }

    private fun TestScope.repo(): RustOfflineRepository {
        val d = UnconfinedTestDispatcher(testScheduler)
        val scope = CoroutineScope(backgroundScope.coroutineContext + d)
        return RustOfflineRepository(
            root = "/data/user/0/app/no_backup/offline",
            auth = auth,
            link = link,
            settings = settings,
            network = network,
            library = library,
            scope = scope,
            io = d,
            openStore = { _, o -> FakeStore(o, seed(o)).also { stores += it } },
            wipe = { _, o -> wiped += o },
            clock = { now },
        )
    }

    private fun signIn(name: String = "alice") {
        auth.value = AuthState.SignedInState(SERVER, who(name))
        link.value = fakeLink
    }

    @Test
    fun signingInOpensTheUsersDownloadsAndStartsTheManager() = runTest {
        seed = { mutableListOf(offlineTrack(1), offlineTrack(2)) }
        val r = repo()
        assertNull(r.state.value.owner)
        signIn()
        val s = r.state.value
        assertEquals(Owner(SERVER.url, "alice"), s.owner)
        assertEquals(2, s.tracks.size)
        assertEquals(setOf(101L, 102L), s.downloadedIds)
        assertEquals(2000L, s.usedBytes)
        assertTrue(s.canDownload)
        assertEquals("alice", settings.state.value.offlineOwner)
        assertEquals(NetworkKind.WIFI, fakeLink.startedWith?.second)
        // Settings and the network reach the manager.
        settings.setDownloads(DownloadSettings(wifiOnly = true, capBytes = DownloadSettings.GB))
        network.value = NetworkKind.MOBILE
        val calls = fakeLink.downloaders.single().calls
        assertTrue(calls.toString(), "policy true ${DownloadSettings.GB}" in calls)
        assertTrue(calls.toString(), "network MOBILE" in calls)
        assertEquals(NetworkKind.MOBILE, r.state.value.network)
    }

    @Test
    fun offlineBeforeSignInShowsTheLastUsersDownloadsWithoutDownloading() = runTest {
        settings.state.update { it.copy(offlineOwner = "alice") }
        seed = { mutableListOf(offlineTrack(1)) }
        val r = repo()
        auth.value = AuthState.Unreachable(SERVER, "no route")
        assertEquals(Owner(SERVER.url, "alice"), r.state.value.owner)
        assertEquals(1, r.state.value.tracks.size)
        assertFalse(r.state.value.canDownload)
        assertTrue(fakeLink.downloaders.isEmpty())
        // Nobody remembered: nothing to show.
        settings.state.update { it.copy(offlineOwner = null) }
        assertNull(r.state.value.owner)
        assertTrue(stores.single().closed)
    }

    @Test
    fun aRevokedDeviceRemovesItsDownloads() = runTest {
        val r = repo()
        signIn()
        val store = stores.single()
        auth.value = AuthState.SignedOut(SERVER, revoked = true)
        link.value = null
        assertEquals(listOf(Owner(SERVER.url, "alice")), wiped)
        assertTrue(store.closed)
        assertTrue(fakeLink.downloaders.single().shutDown)
        assertNull(settings.state.value.offlineOwner)
        assertNull(r.state.value.owner)
    }

    @Test
    fun aRevocationFoundAtStartRemovesTheRememberedUsersDownloads() = runTest {
        settings.state.update { it.copy(offlineOwner = "alice") }
        repo()
        auth.value = AuthState.Restoring(SERVER)
        auth.value = AuthState.SignedOut(SERVER, revoked = true)
        assertEquals(listOf(Owner(SERVER.url, "alice")), wiped)
        assertNull(settings.state.value.offlineOwner)
    }

    @Test
    fun signingOutSendsPlaysThenKeepsOrRemovesAndForgetsTheUser() = runTest {
        seed = { mutableListOf(offlineTrack(1)) }
        val r = repo()
        signIn()
        r.signOut(removeDownloads = false)
        assertEquals(1, fakeLink.syncs)
        assertTrue(wiped.isEmpty())
        assertNull(settings.state.value.offlineOwner)
        assertTrue(stores.single().closed)
        // Still signed in for a moment: the account isn't remembered again.
        assertNull(settings.state.value.offlineOwner)
        auth.value = AuthState.SignedOut(SERVER)
        link.value = null
        assertNull(r.state.value.owner)

        signIn()
        r.signOut(removeDownloads = true)
        assertEquals(listOf(Owner(SERVER.url, "alice")), wiped)
    }

    @Test
    fun downloadsQueueLibraryTracksAndNeedASignIn() = runTest {
        val r = repo()
        assertEquals(ApiResult.Err(AppError.SignedOut), r.downloadAlbum(3))
        signIn()
        assertEquals(ApiResult.Ok(2), r.downloadAlbum(3))
        // A playlist's radio stream (no id) is skipped.
        assertEquals(ApiResult.Ok(1), r.downloadPlaylist(9))
        r.downloadTracks(listOf(libTrack(77, null)))
        val d = fakeLink.downloaders.single()
        assertEquals(
            listOf(
                DownloadRequest(31, 3, DownloadOrigin.MANUAL),
                DownloadRequest(32, 3, DownloadOrigin.MANUAL),
                DownloadRequest(501, 50, DownloadOrigin.MANUAL),
                DownloadRequest(77, null, DownloadOrigin.MANUAL),
            ),
            d.enqueued,
        )
        assertTrue(d.calls.all { !it.startsWith("enqueue") || it.endsWith(FormatChoice.AUTO.toString()) })
        r.pause(31)
        r.resume(null)
        r.cancel(32)
        r.clearFinished()
        assertEquals(listOf("pause 31", "resume null", "cancel 32", "clear"), d.calls.filter { !it.startsWith("enqueue") && !it.startsWith("policy") && !it.startsWith("network") })
        // A failing album lookup is reported, nothing queued.
        library.failAlbum = 4
        assertTrue(r.downloadAlbum(4) is ApiResult.Err)
    }

    @Test
    fun theManagersReportsAndLibraryChangesReachTheState() = runTest {
        val r = repo()
        signIn()
        val snap = DownloadsSnapshot(emptyList(), true, false, NetworkKind.WIFI, 4096uL, 0uL, 0u)
        fakeLink.observer!!.onDownloads(snap)
        assertEquals(snap, r.state.value.downloads)
        assertEquals(4096L, r.state.value.usedBytes)
        stores.single().tracks += offlineTrack(5)
        fakeLink.observer!!.onLibraryChanged()
        advanceTimeBy(RustOfflineRepository.REFRESH_COALESCE_MS + 1)
        runCurrent()
        assertEquals(1, r.state.value.tracks.size)
        r.remove(listOf(offlineTrack(5).key))
        assertTrue(r.state.value.tracks.isEmpty())
        stores.single().tracks += offlineTrack(6)
        r.removeAll()
        assertTrue(stores.single().removedAll)
        assertTrue("cancel null" in fakeLink.downloaders.single().calls)
        assertNotNull(r.artwork("a7"))
        assertNull(r.artwork("a8"))
    }

    @Test
    fun smartDownloadsRefreshOnWifiOnlyAndPlanFromTheLists() = runTest {
        val r = repo()
        network.value = NetworkKind.MOBILE
        signIn()
        settings.setDownloads(DownloadSettings(smartRecentAlbums = 2, smartFavourites = true))
        library.favourites = FavouriteLibraryItems(listOf(900L), listOf(5L))
        assertTrue("not on mobile data", stores.single().planned.isEmpty())
        stores.single().plan = SmartPlan(listOf(DownloadRequest(11, 1, DownloadOrigin.RECENT)), listOf("old"))
        network.value = NetworkKind.WIFI
        val wanted = stores.single().planned.single()
        assertEquals(listOf(11L, 12L, 21L, 22L, 900L, 51L, 52L), wanted.map { it.trackId })
        assertEquals(DownloadOrigin.FAVOURITE, wanted.first { it.trackId == 900L }.origin)
        assertEquals(DownloadOrigin.RECENT, wanted.first().origin)
        assertEquals(listOf("old"), stores.single().removed)
        assertEquals(listOf(DownloadRequest(11, 1, DownloadOrigin.RECENT)), fakeLink.downloaders.single().enqueued)
        // Not again until it's due, or the settings change.
        network.value = NetworkKind.MOBILE
        network.value = NetworkKind.WIFI
        assertEquals(1, stores.single().planned.size)
        now += RustOfflineRepository.SMART_EVERY_MS
        network.value = NetworkKind.MOBILE
        network.value = NetworkKind.WIFI
        assertEquals(2, stores.single().planned.size)
        // A failure part-way plans nothing (so nothing still wanted is removed).
        library.failAlbum = 2
        assertTrue(r.refreshSmart() is ApiResult.Err)
        assertEquals(2, stores.single().planned.size)
    }

    @Test
    fun playsAreSentWhenOnlineWithPlaysWaiting() = runTest {
        seed = { mutableListOf() }
        val r = repo()
        signIn()
        assertEquals(0, fakeLink.syncs)
        stores.single().pending = 3
        fakeLink.sync = {
            stores.single().pending = 0
            PlaysSyncOutcome(3u, 0u, 0u, false)
        }
        fakeLink.observer!!.onLibraryChanged()
        advanceTimeBy(RustOfflineRepository.REFRESH_COALESCE_MS + 1)
        runCurrent()
        assertEquals(1, fakeLink.syncs)
        assertEquals(0, r.state.value.pendingPlays)
        assertEquals(PlaysSyncOutcome(3u, 0u, 0u, false), r.playsSender.last.value)
    }

    @Test
    fun anotherUserGetsTheirOwnDownloads() = runTest {
        seed = { o -> if (o.username == "alice") mutableListOf(offlineTrack(1)) else mutableListOf() }
        val r = repo()
        signIn("alice")
        assertEquals(1, r.state.value.tracks.size)
        auth.value = AuthState.SignedOut(SERVER)
        link.value = null
        signIn("bob")
        assertEquals(Owner(SERVER.url, "bob"), r.state.value.owner)
        assertTrue(r.state.value.tracks.isEmpty())
        assertEquals(2, stores.size)
        assertTrue(stores[0].closed)
    }

    @Test
    fun settingsAreSanitisedAndKept() = runTest {
        val r = repo()
        r.setSettings(DownloadSettings(capBytes = -5, smartRecentAlbums = 9999))
        val d = settings.state.value.downloads
        assertEquals(0L, d.capBytes)
        assertEquals(DownloadSettings.MAX_RECENT, d.smartRecentAlbums)
        assertEquals(AppSettings().downloads.wifiFormat, r.state.value.settings.wifiFormat)
    }
}
