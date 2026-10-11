package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflinePlayerObserver
import io.github.jamezuk.lyrionanywhere.core.OfflinePlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OutputNegotiator
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.PlaysSyncOutcome
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.SoundSettings
import io.github.jamezuk.lyrionanywhere.data.status
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

fun offlineStatus(
    index: Int? = 0,
    keys: List<String> = listOf(offlineTrack(1).key, offlineTrack(2).key),
    playback: LocalPlayback = LocalPlayback.PLAYING,
) = OfflinePlayerStatus(
    playback = playback,
    index = index?.toUInt(),
    keys = keys,
    order = keys.indices.map { it.toUInt() },
    elapsedMs = 1500uL,
    durationMs = 61_000uL,
    shuffle = false,
    repeat = LocalRepeat.OFF,
    bufferedMs = 400uL,
    outputRate = 44_100u,
    underruns = 0uL,
    held = false,
    lastError = null,
)

private class FakeOfflineHandle(val dsp: DspSettings) : OfflinePlayerHandle {
    val calls = mutableListOf<String>()
    var observer: OfflinePlayerObserver? = null
    var shutDown = false

    override fun start(observer: OfflinePlayerObserver) {
        this.observer = observer
        calls += "start"
    }

    override fun playTracks(keys: List<String>, start: Int, shuffle: Boolean): Int {
        calls += "play ${keys.size} $start $shuffle"
        return keys.size
    }

    override fun play() {
        calls += "play"
    }

    override fun pause() {
        calls += "pause"
    }

    override fun next() {
        calls += "next"
    }

    override fun previous() {
        calls += "previous"
    }

    override fun jumpTo(index: Int) {
        calls += "jump $index"
    }

    override fun seekTo(ms: Long) {
        calls += "seek $ms"
    }

    override fun setShuffle(on: Boolean) {
        calls += "shuffle $on"
    }

    override fun setRepeat(repeat: LocalRepeat) {
        calls += "repeat $repeat"
    }

    override fun stop() {
        calls += "stop"
    }

    override fun setDsp(dsp: DspSettings) {
        calls += "dsp ${dsp.preampDb}"
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

    override fun setOutputNegotiator(negotiator: OutputNegotiator?) {
        calls += "negotiator ${negotiator != null}"
    }

    override fun renegotiateOutput() {
        calls += "renegotiate"
    }

    override fun shutdown() {
        shutDown = true
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class OfflinePlayerRepositoryTest {
    private val store = MutableStateFlow<OfflineStore?>(null)
    private val settings = MemSettings()
    private val handles = mutableListOf<FakeOfflineHandle>()

    private fun TestScope.repo(): RustOfflinePlayerRepository {
        val d = UnconfinedTestDispatcher(testScheduler)
        return RustOfflinePlayerRepository(
            store,
            settings,
            CoroutineScope(backgroundScope.coroutineContext + d),
            d,
            factory = { _, dsp -> FakeOfflineHandle(dsp).also { handles += it } },
        )
    }

    @Test
    fun playingStartsTheEngineForTheOpenLibrary() = runTest {
        val r = repo()
        val tracks = listOf(offlineTrack(1), offlineTrack(2))
        r.play(tracks, 1, shuffle = false)
        assertTrue("no library: nothing", handles.isEmpty())
        store.value = FakeStore(Owner(SERVER.url, "alice"))
        r.setOutputRoute(OutputRoute("Headphones", 48_000u))
        r.play(tracks, 1, shuffle = true)
        val h = handles.single()
        assertEquals(listOf("route Headphones", "start", "play 2 1 true"), h.calls)
        // The engine's reports become the state.
        h.observer!!.onStatus(offlineStatus(index = 1))
        val s = r.state.value
        assertTrue(s.active && s.playing)
        assertEquals(offlineTrack(2), s.current)
        assertEquals(tracks, s.queue)
        // Controls go to the engine.
        r.pause()
        r.next()
        r.previous()
        r.seekTo(30_000)
        r.jumpTo(0)
        r.setShuffle(false)
        r.setRepeat(LocalRepeat.ALL)
        r.setHold(true)
        r.setDuck(true)
        r.stop()
        assertEquals(
            listOf("pause", "next", "previous", "seek 30000", "jump 0", "shuffle false", "repeat ALL", "hold true", "duck true", "stop"),
            h.calls.drop(3),
        )
        // toggle follows the state.
        r.toggle()
        assertEquals("pause", h.calls.last())
        h.observer!!.onStatus(offlineStatus(index = 1, playback = LocalPlayback.PAUSED))
        r.toggle()
        assertEquals("play", h.calls.last())
    }

    @Test
    fun soundSettingsApplyAndAnotherLibraryReplacesTheEngine() = runTest {
        val r = repo()
        settings.setSound(SoundSettings(preampDb = -3.0))
        store.value = FakeStore(Owner(SERVER.url, "alice"))
        r.play(listOf(offlineTrack(1)))
        val first = handles.single()
        assertEquals(-3.0, first.dsp.preampDb, 0.0)
        settings.setSound(SoundSettings(preampDb = -6.0))
        assertTrue(first.calls.contains("dsp -6.0"))
        // A stale engine's reports are ignored once it is replaced.
        store.value = FakeStore(Owner(SERVER.url, "bob"))
        assertTrue(first.shutDown)
        first.observer!!.onStatus(offlineStatus())
        assertNull(r.state.value.status)
        assertFalse(r.state.value.active)
        store.value = null
        r.play(listOf(offlineTrack(1)))
        assertEquals(1, handles.size)
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class PlaysSenderTest {
    @Test
    fun sendsWhenReadyAndRetriesWithGrowingWaits() = runTest {
        val ready = MutableStateFlow(false)
        val outcomes = ArrayDeque(
            listOf(
                null,
                PlaysSyncOutcome(1u, 0u, 2u, false),
                PlaysSyncOutcome(2u, 0u, 0u, false),
            ),
        )
        var calls = 0
        val sender = PlaysSender(CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)), ready) {
            calls++
            outcomes.removeFirst()
        }
        sender.start()
        runCurrent()
        assertEquals(0, calls)
        ready.value = true
        runCurrent()
        assertEquals(1, calls)
        advanceTimeBy(PlaysSender.waitAfter(0, false) + 1)
        assertEquals(2, calls)
        advanceTimeBy(PlaysSender.waitAfter(1, false) + 1)
        assertEquals(3, calls)
        // Done: no more attempts.
        advanceTimeBy(24 * 60 * 60 * 1000L)
        assertEquals(3, calls)
        assertEquals(PlaysSyncOutcome(2u, 0u, 0u, false), sender.last.value)
    }

    @Test
    fun anOlderServerIsAskedHourlyAndGoingOfflineStops() = runTest {
        val ready = MutableStateFlow(true)
        var calls = 0
        val sender = PlaysSender(CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)), ready) {
            calls++
            PlaysSyncOutcome(0u, 0u, 4u, true)
        }
        sender.start()
        runCurrent()
        assertEquals(1, calls)
        advanceTimeBy(59 * 60_000L)
        assertEquals(1, calls)
        advanceTimeBy(2 * 60_000L)
        assertEquals(2, calls)
        ready.value = false
        advanceTimeBy(5 * 60 * 60_000L)
        assertEquals(2, calls)
        assertEquals(60 * 60_000L, PlaysSender.waitAfter(0, true))
        assertEquals(listOf(60_000L, 300_000L, 900_000L, 3_600_000L, 3_600_000L), (0..4).map { PlaysSender.waitAfter(it, false) })
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class OfflineModeTest {
    @Test
    fun decisions() {
        val signedIn = AuthState.SignedInState(SERVER, who())
        assertTrue(OfflineModeController.decide(AuthState.Unreachable(SERVER), false, false, false))
        assertFalse(OfflineModeController.decide(signedIn, false, false, true))
        assertTrue(OfflineModeController.decide(signedIn, true, false, false))
        assertTrue(OfflineModeController.decide(signedIn, false, true, false))
        assertTrue("a retry keeps offline mode", OfflineModeController.decide(AuthState.Restoring(SERVER), false, false, true))
        assertFalse(OfflineModeController.decide(AuthState.Restoring(SERVER), false, false, false))
        assertFalse(OfflineModeController.decide(AuthState.SignedOut(SERVER), true, true, true))
        assertFalse(OfflineModeController.decide(AuthState.NoServer, true, true, true))
    }

    @Test
    fun troubleIsAReconnectingPhonePlayer() {
        assertFalse(OfflineModeController.troubleOf(LocalPlayerState()))
        assertTrue(
            OfflineModeController.troubleOf(
                LocalPlayerState(enabled = true, status = status(connection = PlayerConnection.Reconnecting(1000uL))),
            ),
        )
        assertFalse(OfflineModeController.troubleOf(LocalPlayerState(enabled = true, status = status())))
    }

    private class Rig(scope: TestScope) {
        val auth = MutableStateFlow<AuthState>(AuthState.Starting)
        val network = MutableStateFlow(NetworkKind.WIFI)
        val trouble = MutableStateFlow(false)
        var retries = 0
        val c = OfflineModeController(
            CoroutineScope(scope.backgroundScope.coroutineContext + UnconfinedTestDispatcher(scope.testScheduler)),
            auth,
            network,
            trouble,
        ) {
            retries++
        }

        init {
            c.start()
        }
    }

    @Test
    fun goesOfflineWithoutNetworkOrServerAndComesBack() = runTest {
        val r = Rig(this)
        r.auth.value = AuthState.SignedInState(SERVER, who())
        runCurrent()
        assertFalse(r.c.offline.value)
        // A brief drop (a handover) isn't offline; a longer one is.
        r.network.value = NetworkKind.OFFLINE
        advanceTimeBy(1000)
        r.network.value = NetworkKind.WIFI
        advanceTimeBy(OfflineModeController.NETWORK_GRACE_MS * 2)
        assertFalse(r.c.offline.value)
        r.network.value = NetworkKind.OFFLINE
        advanceTimeBy(OfflineModeController.NETWORK_GRACE_MS + 1)
        assertTrue(r.c.offline.value)
        r.network.value = NetworkKind.MOBILE
        runCurrent()
        assertFalse("back at once", r.c.offline.value)
        // The phone's player can't reach the server for a while.
        r.trouble.value = true
        advanceTimeBy(OfflineModeController.TROUBLE_GRACE_MS + 1)
        assertTrue(r.c.offline.value)
        r.trouble.value = false
        runCurrent()
        assertFalse(r.c.offline.value)
    }

    @Test
    fun beforeSignInItRetriesNowAndThenAndWhenTheNetworkReturns() = runTest {
        val r = Rig(this)
        r.auth.value = AuthState.Unreachable(SERVER)
        runCurrent()
        assertTrue(r.c.offline.value)
        assertEquals(0, r.retries)
        advanceTimeBy(OfflineModeController.RETRY_MS + 1)
        assertEquals(1, r.retries)
        // A retry in progress keeps offline mode (no flicker), and doesn't restart the timer.
        r.auth.value = AuthState.Restoring(SERVER)
        r.auth.value = AuthState.Unreachable(SERVER)
        runCurrent()
        assertTrue(r.c.offline.value)
        assertEquals(1, r.retries)
        // No network: no retries; its return retries at once.
        r.network.value = NetworkKind.OFFLINE
        advanceTimeBy(OfflineModeController.RETRY_MS * 3)
        assertEquals(1, r.retries)
        r.network.value = NetworkKind.WIFI
        runCurrent()
        assertEquals(2, r.retries)
        r.auth.value = AuthState.SignedInState(SERVER, who())
        runCurrent()
        assertFalse(r.c.offline.value)
        advanceTimeBy(OfflineModeController.RETRY_MS * 3)
        assertEquals(2, r.retries)
    }

    @Test
    fun sustainedNeedsTheValueToHold() = runTest {
        val src = MutableStateFlow(false)
        val seen = mutableListOf<Boolean>()
        backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { src.sustained(100).toList(seen) }
        src.value = true
        advanceTimeBy(50)
        src.value = false
        advanceTimeBy(200)
        src.value = true
        advanceTimeBy(101)
        assertEquals(listOf(false, false, true), seen)
    }
}

private class FakeLocal(initial: LocalPlayerState) : LocalPlayerRepository {
    val flow = MutableStateFlow(initial)
    override val state: StateFlow<LocalPlayerState> = flow
    val holds = mutableListOf<Boolean>()

    override suspend fun setEnabled(on: Boolean) = ApiResult.Ok(Unit)

    override fun setHold(hold: Boolean) {
        holds += hold
    }

    override fun setDuck(duck: Boolean) {}

    override fun setOutputRoute(route: OutputRoute) {}
}

@OptIn(ExperimentalCoroutinesApi::class)
class PlaybackArbiterTest {
    @Test
    fun onlyOnePlayerMakesSound() = runTest {
        val local = FakeLocal(LocalPlayerState(enabled = true, status = status()))
        val offlineState = MutableStateFlow(OfflinePlayerState())
        val paused = mutableListOf<String>()
        var offlinePauses = 0
        val offline = object : OfflinePlayerRepository {
            override val state: StateFlow<OfflinePlayerState> = offlineState
            override fun play(tracks: List<io.github.jamezuk.lyrionanywhere.core.OfflineTrack>, start: Int, shuffle: Boolean) {}
            override fun play() {}
            override fun pause() {
                offlinePauses++
            }
            override fun next() {}
            override fun previous() {}
            override fun seekTo(ms: Long) {}
            override fun jumpTo(index: Int) {}
            override fun setShuffle(on: Boolean) {}
            override fun setRepeat(repeat: LocalRepeat) {}
            override fun stop() {}
        }
        val players = FakePlayersPause { id -> paused += id }
        PlaybackArbiter(CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)), local, players, offline).start()
        runCurrent()
        // Downloads start while LMS plays on this phone: LMS's play pauses.
        offlineState.value = OfflinePlayerState(status = offlineStatus())
        runCurrent()
        assertEquals(listOf("02:4c:41:00:00:01"), paused)
        assertEquals(listOf(true), local.holds)
        // LMS starts this phone's player again: the downloads pause.
        local.flow.value = LocalPlayerState(enabled = true, status = status().copy(playback = LocalPlayback.STOPPED))
        runCurrent()
        local.flow.value = LocalPlayerState(enabled = true, status = status())
        runCurrent()
        assertEquals(1, offlinePauses)
    }
}
