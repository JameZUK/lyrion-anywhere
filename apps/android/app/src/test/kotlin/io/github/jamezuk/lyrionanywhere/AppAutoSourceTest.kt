package io.github.jamezuk.lyrionanywhere

import io.github.jamezuk.lyrionanywhere.auto.AppAutoSource
import io.github.jamezuk.lyrionanywhere.core.Device
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.SignedIn
import io.github.jamezuk.lyrionanywhere.core.User
import io.github.jamezuk.lyrionanywhere.core.UserRole
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.LibraryRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.ServerInfo
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import java.lang.reflect.Proxy
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.advanceUntilIdle
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

private const val PHONE = "02:4c:41:00:00:01"
private val SERVER = ServerInfo("https://music.example.com", "Music", true)
private val SIGNED_IN = AuthState.SignedInState(SERVER, SignedIn(User("alice", UserRole.LISTENER), Device(1, "Phone", PHONE)))

/** An interface whose every call fails the test (it must not be used). */
private inline fun <reified T : Any> unused(): T =
    Proxy.newProxyInstance(T::class.java.classLoader, arrayOf(T::class.java)) { _, m, _ ->
        throw AssertionError("unexpected call: ${m.name}")
    } as T

private class Local : LocalPlayerRepository {
    override val state = MutableStateFlow(LocalPlayerState())
    val enabled = mutableListOf<Boolean>()

    override suspend fun setEnabled(on: Boolean): ApiResult<Unit> {
        enabled += on
        state.update { it.copy(enabled = on) }
        return ApiResult.Ok(Unit)
    }

    fun connect() = state.update {
        it.copy(
            status = LocalPlayerStatus(
                PlayerConnection.Connected, LocalPlayback.STOPPED, PHONE, "Phone", 0uL, 0uL, 0u, 0uL, 0uL, false, null,
            ),
        )
    }

    override fun setHold(hold: Boolean) {}

    override fun setDuck(duck: Boolean) {}

    override fun setOutputRoute(route: OutputRoute) {}
}

private class Offline : OfflinePlayerRepository {
    override val state = MutableStateFlow(OfflinePlayerState())
    val calls = mutableListOf<String>()

    override fun play(tracks: List<OfflineTrack>, start: Int, shuffle: Boolean) {
        calls += "queue ${tracks.size} $start"
    }

    override fun play() {
        calls += "play"
    }

    override fun pause() {}

    override fun next() {}

    override fun previous() {}

    override fun seekTo(ms: Long) {}

    override fun jumpTo(index: Int) {}

    override fun setShuffle(on: Boolean) {}

    override fun setRepeat(repeat: LocalRepeat) {}

    override fun stop() {}
}

/** The car's view of the app: online or not, the cold-start wait, and this phone's player. */
@OptIn(ExperimentalCoroutinesApi::class)
class AppAutoSourceTest {
    private val auth = MutableStateFlow<AuthState>(AuthState.Starting)
    private val offlineMode = MutableStateFlow(false)
    private val local = Local()
    private val offline = Offline()
    private val sent = mutableListOf<String>()
    private val players = Proxy.newProxyInstance(
        PlayersRepository::class.java.classLoader,
        arrayOf(PlayersRepository::class.java),
    ) { _, m, args ->
        sent += "${m.name} ${args?.firstOrNull()}"
        ApiResult.Ok(Unit)
    } as PlayersRepository

    private val downloads = MutableStateFlow(OfflineState())

    private fun source() = AppAutoSource(
        auth,
        offlineMode,
        unused<LibraryRepository>(),
        players,
        local,
        downloads,
        offline,
        settleMs = 1_000,
        phoneMs = 2_000,
    )

    private fun dl(n: Int, albumId: Long?, trackNumber: Int?) = OfflineTrack(
        "%064x".format(n), 100L + n, "Song $n", "Band", "Record", albumId, trackNumber?.toUInt(), 120.0,
        DownloadFormat.ORIGINAL, "flc", null, "flac", 1000uL, 0uL, DownloadOrigin.MANUAL,
    )

    @Test
    fun theDownloadsAreGroupedByAlbumOncePerState() {
        // The car's search reads every album's tracks: grouped once per state, not once per
        // album (review, 12 November).
        downloads.value = OfflineState(tracks = listOf(dl(1, 7, 2), dl(2, 8, 1), dl(3, 7, 1), dl(4, null, null)))
        val s = source()
        val seven = s.downloadedTracks("a7")
        assertEquals(listOf(103L, 101L), seven.map { it.trackId })
        assertEquals(listOf(102L), s.downloadedTracks("a8").map { it.trackId })
        assertEquals(listOf(104L), s.downloadedTracks("t" + "%064x".format(4).take(16)).map { it.trackId })
        assertTrue(s.downloadedTracks("a9").isEmpty())
        assertSame("the same grouping while the state is the same", seven, s.downloadedTracks("a7"))
        // A new state: grouped again.
        downloads.value = downloads.value.copy(tracks = listOf(dl(5, 7, 3)))
        assertEquals(listOf(105L), s.downloadedTracks("a7").map { it.trackId })
    }

    @Test
    fun onlineOnlySignedInAndReachable() {
        val s = source()
        assertFalse(s.online())
        auth.value = SIGNED_IN
        assertTrue(s.online())
        offlineMode.value = true
        assertFalse("offline mode: the server can't be reached", s.online())
        offlineMode.value = false
        auth.value = AuthState.Unreachable(SERVER)
        assertFalse(s.online())
    }

    @Test
    fun aColdStartWaitsBrieflyForTheSignIn() = runTest {
        val s = source()
        val waiting = async { s.settle() }
        runCurrent()
        assertFalse(waiting.isCompleted)
        auth.value = AuthState.Restoring(SERVER)
        runCurrent()
        assertFalse(waiting.isCompleted)
        auth.value = SIGNED_IN
        runCurrent()
        assertTrue(waiting.isCompleted)
        // Never for long, whatever happens.
        auth.value = AuthState.Starting
        val stuck = async { s.settle() }
        advanceUntilIdle()
        assertTrue(stuck.isCompleted)
    }

    @Test
    fun playingSwitchesThePhonesPlayerOnAndWaitsForIt() = runTest {
        auth.value = SIGNED_IN
        val s = source()
        val r = async { s.onPhone { id -> players.play(id) } }
        runCurrent()
        assertEquals(listOf(true), local.enabled)
        assertFalse(r.isCompleted)
        local.connect()
        runCurrent()
        assertEquals(ApiResult.Ok(Unit), r.await())
        assertEquals(listOf("play $PHONE"), sent)
    }

    @Test
    fun aPhoneThatNeverConnectsFails() = runTest {
        auth.value = SIGNED_IN
        val s = source()
        val r = async { s.onPhone { id -> players.play(id) } }
        advanceUntilIdle()
        assertTrue(r.await() is ApiResult.Err)
        assertTrue("nothing was sent", sent.isEmpty())
    }

    @Test
    fun resumeCarriesOnWithWhatHasTheSession() = runTest {
        auth.value = SIGNED_IN
        local.state.update { it.copy(enabled = true) }
        local.connect()
        val s = source()
        assertTrue(s.resume())
        assertEquals(listOf("play $PHONE"), sent)
        // Offline with nothing loaded: nothing to carry on with.
        auth.value = AuthState.Unreachable(SERVER)
        assertFalse(s.resume())
    }
}
