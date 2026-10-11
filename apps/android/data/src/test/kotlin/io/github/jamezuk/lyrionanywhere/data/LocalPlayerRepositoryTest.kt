package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerObserver
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.Session
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

fun status(id: String = "02:4c:41:00:00:01", connection: PlayerConnection = PlayerConnection.Connected) =
    LocalPlayerStatus(
        connection = connection,
        playback = LocalPlayback.PLAYING,
        playerId = id,
        name = "Phone",
        elapsedMs = 0u,
        bufferedMs = 0u,
        outputRate = 44_100u,
        underruns = 0u,
        underrunFrames = 0u,
        held = false,
        lastError = null,
    )

private class MemSettings : SettingsStore {
    val state = MutableStateFlow(AppSettings())
    override val settings: Flow<AppSettings> = state

    override suspend fun setServerUrl(url: String?) = state.update { it.copy(serverUrl = url) }

    override suspend fun setSelectedPlayer(playerId: String?) = state.update { it.copy(selectedPlayerId = playerId) }

    override suspend fun setPlayOnPhone(on: Boolean) = state.update { it.copy(playOnPhone = on) }
}

private class FakeHandle : LocalPlayerHandle {
    var observer: LocalPlayerObserver? = null
    var stopped = false
    var closed = false
    val calls = mutableListOf<String>()

    override fun start(observer: LocalPlayerObserver) {
        calls += "start"
        this.observer = observer
    }

    override fun stop() {
        stopped = true
    }

    override fun setHold(hold: Boolean) {
        calls += "hold $hold"
    }

    override fun setDuck(duck: Boolean) {
        calls += "duck $duck"
    }

    override fun setOutputRoute(route: OutputRoute) {
        calls += "route ${route.device}"
    }

    override fun close() {
        closed = true
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class LocalPlayerRepositoryTest {
    private val sessions = MutableStateFlow<Session?>(null)
    private val settings = MemSettings()
    private val handles = mutableListOf<FakeHandle>()
    private var failWith: Exception? = null

    private fun TestScope.repo(): RustLocalPlayerRepository {
        val d = UnconfinedTestDispatcher(testScheduler)
        val scope = CoroutineScope(backgroundScope.coroutineContext + d)
        return RustLocalPlayerRepository(sessions, settings, scope, d) {
            failWith?.let { throw it }
            FakeHandle().also { handles += it }
        }
    }

    @Test
    fun startsOnlyWhenSignedInAndEnabled() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        assertTrue("disabled: no player", handles.isEmpty())
        assertEquals(ApiResult.Ok(Unit), r.setEnabled(true))
        assertEquals(1, handles.size)
        assertEquals(listOf("route ", "start"), handles[0].calls)
        assertTrue(r.state.value.enabled)

        r.setEnabled(false)
        assertTrue(handles[0].stopped && handles[0].closed)
        assertFalse(r.state.value.enabled)
        assertNull(r.state.value.status)
    }

    @Test
    fun cannotEnableWhileSignedOut() = runTest {
        val r = repo()
        assertEquals(ApiResult.Err(AppError.SignedOut), r.setEnabled(true))
        assertFalse(settings.state.value.playOnPhone)
        assertTrue(handles.isEmpty())
    }

    @Test
    fun updatesFlowIntoStateAndStaleOnesAreIgnored() = runTest {
        val r = repo()
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        val first = handles.single()
        first.observer!!.onStatus(status())
        assertEquals("02:4c:41:00:00:01", r.state.value.playerId)
        assertTrue(r.state.value.running)

        // A new sign-in replaces the player; the old one's late updates are ignored.
        sessions.value = Session(NoHandle)
        assertTrue(first.stopped)
        val second = handles[1]
        first.observer!!.onStatus(status(id = "02:4c:41:00:00:99"))
        assertNull(r.state.value.playerId)
        second.observer!!.onStatus(status(connection = PlayerConnection.Stopped("refused")))
        assertFalse(r.state.value.running)
    }

    @Test
    fun signingOutStopsThePlayer() = runTest {
        val r = repo()
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        sessions.value = null
        assertTrue(handles.single().stopped && handles.single().closed)
        assertTrue("the choice is remembered", r.state.value.enabled)
    }

    @Test
    fun holdDuckAndRouteReachThePlayer() = runTest {
        val r = repo()
        r.setOutputRoute(OutputRoute("USB DAC", 96_000u, false))
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        r.setHold(true)
        r.setDuck(true)
        assertEquals(listOf("route USB DAC", "start", "hold true", "duck true"), handles.single().calls)
    }

    @Test
    fun aFailedStartIsReported() = runTest {
        failWith = ApiException.Unauthorised()
        val r = repo()
        settings.setPlayOnPhone(true)
        sessions.value = Session(NoHandle)
        assertEquals(AppError.SignedOut, r.state.value.error)
        assertFalse(r.state.value.running)
    }
}
