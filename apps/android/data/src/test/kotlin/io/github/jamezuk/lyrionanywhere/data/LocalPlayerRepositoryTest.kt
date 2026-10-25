package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import io.github.jamezuk.lyrionanywhere.core.TransitionMode
import io.github.jamezuk.lyrionanywhere.core.ApiException
import io.github.jamezuk.lyrionanywhere.core.CrossfeedSetting
import io.github.jamezuk.lyrionanywhere.core.DitherSetting
import io.github.jamezuk.lyrionanywhere.core.DspSettings
import io.github.jamezuk.lyrionanywhere.core.Levelling
import io.github.jamezuk.lyrionanywhere.core.MixerOffer
import io.github.jamezuk.lyrionanywhere.core.OfferBlocker
import io.github.jamezuk.lyrionanywhere.core.OutputClock
import io.github.jamezuk.lyrionanywhere.core.OutputNegotiator
import io.github.jamezuk.lyrionanywhere.core.OutputOffers
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerObserver
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.NoHandle
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.Session
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.launch
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
    // Off to start with, so each test turns the phone's player on itself.
    val state = MutableStateFlow(AppSettings(playOnPhone = false))
    override val settings: Flow<AppSettings> = state

    override suspend fun setServerUrl(url: String?) = state.update { it.copy(serverUrl = url) }

    override suspend fun setSelectedPlayer(playerId: String?) = state.update { it.copy(selectedPlayerId = playerId) }

    override suspend fun setPlayOnPhone(on: Boolean) = state.update { it.copy(playOnPhone = on) }

    override suspend fun setSound(sound: SoundSettings) = state.update { it.copy(sound = sound) }

    override suspend fun setStreaming(streaming: StreamingSettings) = state.update { it.copy(streaming = streaming) }

    override suspend fun setReadAhead(readAhead: ReadAheadSettings) = state.update { it.copy(readAhead = readAhead) }

    override suspend fun setReplayCacheBytes(bytes: Long) = state.update { it.copy(replayCacheBytes = ReplayCache.sanitise(bytes)) }

    override suspend fun setLibrary(id: String?) = state.update { it.copy(libraryId = id) }

    override suspend fun setDefaultPlayer(choice: io.github.jamezuk.lyrionanywhere.data.DefaultPlayer, playerId: String?) =
        state.update { it.copy(defaultPlayer = choice, defaultPlayerId = playerId) }
}

private class FakeHandle : LocalPlayerHandle {
    var observer: LocalPlayerObserver? = null
    var stopped = false
    var closed = false
    val calls = mutableListOf<String>()

    override fun start(observer: LocalPlayerObserver) {
        calls += "start"
        cacheSizesAtStart = cacheSizes.size
        qualitiesAtStart = qualities.size
        readAheadsAtStart = readAheads.size
        this.observer = observer
    }

    /** Stream levels set, in order (kept apart from [calls]), and how many before start. */
    val qualities: MutableList<StreamQuality> = java.util.concurrent.CopyOnWriteArrayList()
    var qualitiesAtStart: Int? = null

    /** Runs inside [setStreamQuality] before the level is taken (the engine being slow). */
    @Volatile var onQuality: ((StreamQuality) -> Unit)? = null

    override fun stop() {
        stopped = true
    }

    override fun setHold(hold: Boolean) {
        calls += "hold $hold"
    }

    override fun setDuck(duck: Boolean) {
        calls += "duck $duck"
    }

    override fun setSleepFade(endsInSecs: Double?, fadeSecs: UInt) {
        calls += "fade $endsInSecs $fadeSecs"
    }

    override fun setOutputRoute(route: OutputRoute) {
        calls += "route ${route.device}"
    }

    val dsp = mutableListOf<DspSettings>()

    override fun setDsp(dsp: DspSettings) {
        this.dsp += dsp
        calls += "dsp ${dsp.analysisLevelling}"
    }

    override fun setOutputNegotiator(negotiator: OutputNegotiator?) {
        calls += "negotiator ${negotiator != null}"
    }

    override fun renegotiateOutput() {
        calls += "renegotiate"
    }

    override fun setStreamQuality(quality: StreamQuality) {
        onQuality?.invoke(quality)
        qualities += quality
    }

    /** Visualiser switches, in order; and the next frame the engine hands over. */
    val visual = mutableListOf<Boolean>()
    var frame: ByteArray? = null

    override fun setVisualiser(on: Boolean) {
        visual += on
    }

    override fun visualFrame(): ByteArray? = frame

    /** Read-ahead and network switches, in order, and how many came before start. */
    val readAheads = mutableListOf<Boolean>()
    val networks = mutableListOf<Boolean>()
    var readAheadsAtStart: Int? = null

    override fun setReadAhead(on: Boolean) {
        readAheads += on
    }

    override fun setNetworkAvailable(available: Boolean) {
        networks += available
    }

    /** Replay-cache sizes set, in order, and how many came before start. */
    val cacheSizes = mutableListOf<Long>()
    var cacheSizesAtStart: Int? = null

    override fun setStreamCacheBytes(bytes: Long) {
        cacheSizes += bytes
    }

    /** Queue snapshots given, in order (ids, current, repeat). */
    val snapshots = mutableListOf<Triple<List<Long>, UInt?, io.github.jamezuk.lyrionanywhere.core.LocalRepeat>>()

    override fun setQueueSnapshot(trackIds: List<Long>, currentIndex: UInt?, repeat: io.github.jamezuk.lyrionanywhere.core.LocalRepeat) {
        snapshots += Triple(trackIds, currentIndex, repeat)
        calls += "snapshot"
    }

    val controls = mutableListOf<LocalControl>()
    var controlAnswer = true

    override fun localControl(control: LocalControl): Boolean {
        controls += control
        return controlAnswer
    }

    override fun localAvailability(trackIds: List<Long>): List<io.github.jamezuk.lyrionanywhere.core.LocalAvailability> =
        trackIds.map {
            if (it == 7L) io.github.jamezuk.lyrionanywhere.core.LocalAvailability.DOWNLOADED else io.github.jamezuk.lyrionanywhere.core.LocalAvailability.NONE
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

    private var clockMs = 1_000_000L

    private val network = MutableStateFlow(NetworkKind.WIFI)

    /** Folders whose spools were cleared, in order. */
    private val cleared = mutableListOf<String>()

    /** The replay cache as the seams see it, and the folders it was emptied in. */
    private var cacheUsage = ReplayCacheUsage(0, 0)
    private val cacheCleared = mutableListOf<String>()

    /** What `GET /v1/me` says the server's player protocol is (an exception: it can't be asked). */
    private var protocolAnswer: () -> UInt? = { 2u }
    private var protocolReads = 0

    private fun TestScope.repo(): RustLocalPlayerRepository {
        val d = UnconfinedTestDispatcher(testScheduler)
        val scope = CoroutineScope(backgroundScope.coroutineContext + d)
        return RustLocalPlayerRepository(sessions, settings, scope, d, factory = {
            failWith?.let { throw it }
            FakeHandle().also { handles += it }
        }, nowMs = { clockMs }, network = network, cacheDir = "/data/app/stream-cache", clearSpools = { cleared += it },
            cacheTotals = { _, _ -> cacheUsage },
            cacheClear = { _, dir ->
                cacheCleared += dir
                cacheUsage = ReplayCacheUsage(0, 0)
            },
            serverProtocol = {
                protocolReads++
                protocolAnswer()
            })
    }

    /**
     * The override from the signal-path sheet: it reaches the engine at once, is in memory
     * only, survives the engine's own updates (a track change) and a restart of the engine,
     * and ends when the network type changes; "back to automatic" restores the setting.
     */
    @Test
    fun theQualityOverrideLastsUntilTheNetworkTypeChanges() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        network.value = NetworkKind.MOBILE
        r.setEnabled(true)
        val h = handles.single()
        assertEquals(StreamQuality.HIGH, h.qualities.last())
        assertEquals(io.github.jamezuk.lyrionanywhere.core.QualityOrigin.Setting(io.github.jamezuk.lyrionanywhere.core.QualityNetwork.MOBILE, StreamQuality.HIGH), r.state.value.qualityOrigin)
        r.setQualityOverride(StreamQuality.ORIGINAL)
        assertEquals(StreamQuality.ORIGINAL, h.qualities.last())
        assertEquals(StreamQuality.ORIGINAL, r.state.value.streamQuality)
        assertEquals(StreamQuality.HIGH, r.state.value.streamSetting)
        assertEquals(StreamQuality.ORIGINAL, r.state.value.qualityOverride)
        assertEquals(io.github.jamezuk.lyrionanywhere.core.QualityOrigin.Override(StreamQuality.ORIGINAL), r.state.value.qualityOrigin)
        // The engine's updates (the next track starting) leave it alone.
        h.observer!!.onStatus(status())
        h.observer!!.onSignalPath(null)
        assertEquals(StreamQuality.ORIGINAL, r.state.value.qualityOverride)
        // A change of the setting doesn't end it either.
        settings.setStreaming(StreamingSettings(mobile = StreamQuality.LOW))
        assertEquals(StreamQuality.ORIGINAL, h.qualities.last())
        // A new engine starts with it.
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf(StreamQuality.ORIGINAL), handles.last().qualities)
        // Back to automatic: the setting for this network.
        r.setQualityOverride(null)
        assertEquals(StreamQuality.LOW, handles.last().qualities.last())
        assertNull(r.state.value.qualityOverride)
        // Overridden again, then the network type changes: the override is gone.
        r.setQualityOverride(StreamQuality.HIGH)
        assertEquals(StreamQuality.HIGH, handles.last().qualities.last())
        network.value = NetworkKind.WIFI
        assertNull(r.state.value.qualityOverride)
        assertEquals(StreamQuality.ORIGINAL, handles.last().qualities.last())
        assertEquals(io.github.jamezuk.lyrionanywhere.core.QualityOrigin.Setting(io.github.jamezuk.lyrionanywhere.core.QualityNetwork.WIFI, StreamQuality.ORIGINAL), r.state.value.qualityOrigin)
    }

    /**
     * `GET /v1/me`'s protocol: below 2, or not given (a plugin before 0.7.3), the player
     * socket is never opened and the state says why; 2 or more plays; a server that can't
     * be asked doesn't stop the player (it can't be playing, and the engine stops on 4426).
     */
    @Test
    fun anOldPluginNeverGetsThePlayerSocket() = runTest {
        protocolAnswer = { null }
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertTrue("no player socket", handles.isEmpty())
        assertTrue(r.state.value.serverTooOld)
        assertEquals(PhoneBlock.UPDATE_PLUGIN, r.state.value.phoneBlocked)
        assertNull(r.state.value.serverProtocol)
    }

    @Test
    fun protocolOneIsTooOld() = runTest {
        protocolAnswer = { 1u }
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertTrue(handles.isEmpty())
        assertEquals(1u, r.state.value.serverProtocol)
        assertEquals(PhoneBlock.UPDATE_PLUGIN, r.state.value.phoneBlocked)
    }

    @Test
    fun protocolTwoOrMorePlays() = runTest {
        protocolAnswer = { 3u }
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(1, handles.size)
        assertFalse(r.state.value.serverTooOld)
        assertNull(r.state.value.phoneBlocked)
    }

    @Test
    fun aServerThatCantBeAskedStillPlays() = runTest {
        protocolAnswer = { throw ApiException.Network("test: no route") }
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(1, handles.size)
        assertFalse(r.state.value.protocolChecked)
        assertFalse(r.state.value.serverTooOld)
    }

    /** Read once per sign-in, and known for About even with the player off. */
    @Test
    fun theProtocolIsReadOncePerSignIn() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        assertEquals(1, protocolReads)
        assertEquals(2u, r.state.value.serverProtocol)
        assertTrue(r.state.value.protocolChecked)
        r.setEnabled(true)
        assertEquals(1, protocolReads)
        assertEquals(2u, r.state.value.serverProtocol)
        // Another sign-in asks again.
        sessions.value = Session(NoHandle)
        assertEquals(2, protocolReads)
    }

    /**
     * A phone blocked by an old plugin is unblocked once the plugin is updated, without a new
     * session: Play on this phone switched again asks the server again, at most once a minute.
     */
    @Test
    fun protocolOneThenTwoUnblocksWithoutANewSession() = runTest {
        protocolAnswer = { 1u }
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(PhoneBlock.UPDATE_PLUGIN, r.state.value.phoneBlocked)
        assertEquals(1, protocolReads)
        // The plugin is updated.
        protocolAnswer = { 2u }
        // Switched off and on at once: not asked again within the minute.
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(1, protocolReads)
        assertEquals(PhoneBlock.UPDATE_PLUGIN, r.state.value.phoneBlocked)
        assertTrue(handles.isEmpty())
        // A minute on: switching it again asks, and the player starts.
        clockMs += PROTOCOL_RECHECK_MS
        r.setEnabled(false)
        assertEquals(2, protocolReads)
        r.setEnabled(true)
        assertEquals(2, protocolReads)
        assertNull(r.state.value.phoneBlocked)
        assertEquals(2u, r.state.value.serverProtocol)
        assertEquals(1, handles.size)
        assertEquals(listOf("start"), handles[0].calls.filter { it == "start" })
    }

    /** The network coming back while blocked asks again (at most once a minute). */
    @Test
    fun theNetworkBackWhileBlockedAsksAgain() = runTest {
        protocolAnswer = { 1u }
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(1, protocolReads)
        protocolAnswer = { 2u }
        network.value = NetworkKind.OFFLINE
        network.value = NetworkKind.WIFI
        assertEquals("not within the minute", 1, protocolReads)
        assertTrue(handles.isEmpty())
        clockMs += PROTOCOL_RECHECK_MS
        network.value = NetworkKind.OFFLINE
        network.value = NetworkKind.WIFI
        assertEquals(2, protocolReads)
        assertNull(r.state.value.phoneBlocked)
        assertEquals(1, handles.size)
        // Not blocked any more: the network going and coming asks nothing.
        clockMs += PROTOCOL_RECHECK_MS
        network.value = NetworkKind.OFFLINE
        network.value = NetworkKind.WIFI
        assertEquals(2, protocolReads)
        assertEquals("the same engine", 1, handles.size)
    }

    /** A server that can't be asked again leaves a plugin known to be too old blocked. */
    @Test
    fun aFailedRecheckKeepsTheBlock() = runTest {
        protocolAnswer = { 1u }
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        protocolAnswer = { throw ApiException.Network("test: no route") }
        clockMs += PROTOCOL_RECHECK_MS
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(2, protocolReads)
        assertEquals(PhoneBlock.UPDATE_PLUGIN, r.state.value.phoneBlocked)
        assertEquals(1u, r.state.value.serverProtocol)
        assertTrue(handles.isEmpty())
    }

    /** A sign-out or another sign-in ends the override; switching the player doesn't. */
    @Test
    fun theOverrideEndsWithTheSignIn() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        r.setQualityOverride(StreamQuality.LOW)
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(StreamQuality.LOW, r.state.value.qualityOverride)
        assertEquals(listOf(StreamQuality.LOW), handles.last().qualities)
        // Another server (or user): the setting applies again, to the new engine too.
        sessions.value = Session(NoHandle)
        assertNull(r.state.value.qualityOverride)
        assertEquals(StreamQuality.ORIGINAL, r.state.value.streamQuality)
        assertEquals(listOf(StreamQuality.ORIGINAL), handles.last().qualities)
        // Signed out: gone too.
        r.setQualityOverride(StreamQuality.HIGH)
        sessions.value = null
        assertNull(r.state.value.qualityOverride)
        assertEquals(StreamQuality.ORIGINAL, r.state.value.streamQuality)
    }

    /**
     * The sheet's override and a network change at the same moment, on real threads: the
     * engine is slow to take the override, and the network collector must wait for it, so
     * the state and the engine end on the same level (the network's: it came last).
     */
    @Test
    fun theOverrideAndANetworkChangeCannotLeaveTheStateAndTheEngineApart() {
        val pool = java.util.concurrent.Executors.newFixedThreadPool(2)
        val io = pool.asCoroutineDispatcher()
        val job = kotlinx.coroutines.SupervisorJob()
        val scope = CoroutineScope(job + kotlinx.coroutines.Dispatchers.Unconfined)
        val made = java.util.Collections.synchronizedList(mutableListOf<FakeHandle>())
        try {
            kotlinx.coroutines.runBlocking {
                val r = RustLocalPlayerRepository(
                    sessions, settings, scope, io, factory = { FakeHandle().also { made += it } },
                    nowMs = { clockMs }, network = network, serverProtocol = { 2u },
                )
                sessions.value = Session(NoHandle)
                r.setEnabled(true)
                kotlinx.coroutines.withTimeout(5_000) { while (made.firstOrNull()?.observer == null) kotlinx.coroutines.delay(5) }
                val h = made.single()
                val entered = java.util.concurrent.CountDownLatch(1)
                val release = java.util.concurrent.CountDownLatch(1)
                h.onQuality = { q ->
                    if (q == StreamQuality.LOW) {
                        entered.countDown()
                        release.await(5, java.util.concurrent.TimeUnit.SECONDS)
                    }
                }
                val sheet = launch(kotlinx.coroutines.Dispatchers.Default) { r.setQualityOverride(StreamQuality.LOW) }
                assertTrue(entered.await(5, java.util.concurrent.TimeUnit.SECONDS))
                // On mobile data now, while the engine is still taking the override.
                network.value = NetworkKind.MOBILE
                // Unguarded, the collector would reach the engine now; give it the chance.
                kotlinx.coroutines.withTimeoutOrNull(300) { while (StreamQuality.HIGH !in h.qualities) kotlinx.coroutines.delay(5) }
                release.countDown()
                sheet.join()
                kotlinx.coroutines.withTimeoutOrNull(5_000) {
                    while (h.qualities.last() != StreamQuality.HIGH || r.state.value.streamQuality != StreamQuality.HIGH) kotlinx.coroutines.delay(5)
                }
                assertEquals("the engine", StreamQuality.HIGH, h.qualities.last())
                assertEquals("the state", StreamQuality.HIGH, r.state.value.streamQuality)
                assertNull("the network type changed after the override", r.state.value.qualityOverride)
                assertEquals(listOf(StreamQuality.ORIGINAL, StreamQuality.LOW, StreamQuality.HIGH), h.qualities)
            }
        } finally {
            job.cancel()
            pool.shutdownNow()
        }
    }

    /** Close 4426 (the server needs a newer app): the player has ended and says why. */
    @Test
    fun updateAppEndsThePlayerWithItsOwnReason() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        handles.single().observer!!.onStatus(status(connection = PlayerConnection.UpdateApp))
        val s = r.state.value
        assertTrue(s.appTooOld)
        assertFalse(s.running)
        assertEquals(PhoneBlock.UPDATE_APP, s.phoneBlocked)
        assertTrue(PlayerConnection.UpdateApp.ended)
        assertFalse(PlayerConnection.Connected.ended)
    }

    /** The replay cache's size reaches a new engine before it starts, and a running one at once. */
    @Test
    fun theQueueSnapshotAndTheOfflineControlsReachTheEngine() = runTest {
        val r = repo()
        // No engine yet: kept for it, and nothing to control.
        r.setQueueSnapshot(listOf(5, null, -3, 7), 1, io.github.jamezuk.lyrionanywhere.core.RepeatMode.ALL)
        assertFalse(r.localControl(LocalControl.Next))
        assertEquals(
            List(2) { io.github.jamezuk.lyrionanywhere.core.LocalAvailability.NONE },
            r.localAvailability(listOf(7, 8)),
        )
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        val h = handles[0]
        // Given before the engine starts; what isn't a library track is 0.
        assertTrue(h.calls.indexOf("snapshot") < h.calls.indexOf("start"))
        assertEquals(
            Triple(listOf(5L, 0L, 0L, 7L), 1u, io.github.jamezuk.lyrionanywhere.core.LocalRepeat.ALL),
            h.snapshots.single(),
        )
        // The same again: not sent twice. A negative position is none.
        r.setQueueSnapshot(listOf(5, null, -3, 7), 1, io.github.jamezuk.lyrionanywhere.core.RepeatMode.ALL)
        assertEquals(1, h.snapshots.size)
        r.setQueueSnapshot(listOf(5), -1, io.github.jamezuk.lyrionanywhere.core.RepeatMode.ONE)
        assertEquals(Triple(listOf(5L), null, io.github.jamezuk.lyrionanywhere.core.LocalRepeat.ONE), h.snapshots.last())
        // Bounded.
        r.setQueueSnapshot(List(MAX_QUEUE_SNAPSHOT + 5) { 1L }, 0, io.github.jamezuk.lyrionanywhere.core.RepeatMode.OFF)
        assertEquals(MAX_QUEUE_SNAPSHOT, h.snapshots.last().first.size)
        // Controls and availability go to the engine, which says what it could do (a jump,
        // which reads what is on the phone first, goes off the calling thread).
        assertTrue(r.localControl(LocalControl.Seek(1_000)))
        h.controlAnswer = false
        assertFalse(r.localControl(LocalControl.Next))
        r.localControl(LocalControl.Jump(3))
        assertEquals(listOf(LocalControl.Seek(1_000), LocalControl.Next, LocalControl.Jump(3)), h.controls)
        assertEquals(
            listOf(io.github.jamezuk.lyrionanywhere.core.LocalAvailability.DOWNLOADED, io.github.jamezuk.lyrionanywhere.core.LocalAvailability.NONE),
            r.localAvailability(listOf(7, 8)),
        )
        // A new engine gets the last snapshot.
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(MAX_QUEUE_SNAPSHOT, handles[1].snapshots.single().first.size)
    }

    @Test
    fun theReplayCacheSizeReachesTheEngine() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(listOf(ReplayCache.GB), handles[0].cacheSizes)
        assertEquals(1, handles[0].cacheSizesAtStart)
        settings.setReplayCacheBytes(5 * ReplayCache.GB)
        assertEquals(5 * ReplayCache.GB, handles[0].cacheSizes.last())
        settings.setReplayCacheBytes(0)
        assertEquals(0L, handles[0].cacheSizes.last())
        assertEquals("the engine empties it itself", emptyList<String>(), cacheCleared)
        // A new engine starts with the size wanted now.
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf(0L), handles.last().cacheSizes)
    }

    /** "Off" empties the cache even with this phone's player off; usage and "Clear" go through the seams. */
    @Test
    fun theReplayCacheCanBeReadClearedAndTurnedOffWithoutAPlayer() = runTest {
        val r = repo()
        // Signed out: nothing to show, nothing to clear.
        r.refreshReplayCache()
        assertNull(r.state.value.replayCache)
        r.clearReplayCache()
        assertEquals(emptyList<String>(), cacheCleared)
        sessions.value = Session(NoHandle)
        cacheUsage = ReplayCacheUsage(700L * 1024 * 1024, 9)
        r.refreshReplayCache()
        assertEquals(ReplayCacheUsage(700L * 1024 * 1024, 9), r.state.value.replayCache)
        r.clearReplayCache()
        assertEquals(listOf("/data/app/stream-cache"), cacheCleared)
        assertEquals(ReplayCacheUsage(0, 0), r.state.value.replayCache)
        // Off, with no player running: emptied at once.
        cacheUsage = ReplayCacheUsage(1, 1)
        settings.setReplayCacheBytes(0)
        assertEquals(2, cacheCleared.size)
        assertEquals(ReplayCacheUsage(0, 0), r.state.value.replayCache)
        assertTrue(handles.isEmpty())
    }

    /** Gaps in the signal: read ahead when the setting is on, and on mobile data only if allowed there. */
    @Test
    fun theReadAheadRule() {
        for (metered in listOf(false, true)) {
            assertFalse(ReadAheadSettings(enabled = false, onMobileData = true).effective(metered))
            assertFalse(ReadAheadSettings(enabled = false, onMobileData = false).effective(metered))
            assertTrue(ReadAheadSettings(enabled = true, onMobileData = true).effective(metered))
        }
        assertTrue(ReadAheadSettings(enabled = true, onMobileData = false).effective(metered = false))
        assertFalse(ReadAheadSettings(enabled = true, onMobileData = false).effective(metered = true))
        // On by default, everywhere (the owner's decision).
        assertTrue(ReadAheadSettings().effective(metered = true))
        assertTrue(networkAvailable(NetworkKind.WIFI) && networkAvailable(NetworkKind.MOBILE))
        assertFalse(networkAvailable(NetworkKind.OFFLINE))
    }

    /** Read-ahead follows the settings and the network live, and a new engine is told before it starts. */
    @Test
    fun readAheadFollowsTheSettingsAndTheNetwork() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(listOf(true), handles[0].readAheads)
        assertEquals(1, handles[0].readAheadsAtStart)
        assertTrue(r.state.value.readAhead)
        // Mobile data, allowed there by default: nothing changes.
        network.value = NetworkKind.MOBILE
        assertEquals(listOf(true), handles[0].readAheads)
        // Not on mobile data: off at once, and on again back on Wi-Fi.
        settings.setReadAhead(ReadAheadSettings(enabled = true, onMobileData = false))
        assertEquals(listOf(true, false), handles[0].readAheads)
        assertFalse(r.state.value.readAhead)
        network.value = NetworkKind.WIFI
        assertEquals(listOf(true, false, true), handles[0].readAheads)
        // Off altogether.
        settings.setReadAhead(ReadAheadSettings(enabled = false, onMobileData = true))
        assertEquals(false, handles[0].readAheads.last())
        // A new engine starts with what is wanted now.
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf(false), handles.last().readAheads)
        assertEquals(1, handles.last().readAheadsAtStart)
    }

    /** No network: the engine is told (it stops reconnecting), and told again when it is back. */
    @Test
    fun theNetworkReachesTheEngine() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(listOf(true), handles[0].networks)
        network.value = NetworkKind.MOBILE
        assertEquals(listOf(true), handles[0].networks)
        network.value = NetworkKind.OFFLINE
        assertFalse(r.state.value.networkAvailable)
        network.value = NetworkKind.WIFI
        assertEquals(listOf(true, false, true), handles[0].networks)
        // A new engine without a network is told so before it starts.
        network.value = NetworkKind.OFFLINE
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf(false), handles.last().networks)
    }

    /**
     * Back in the foreground: a player waiting to reconnect tries again at once (its back-off
     * reset by "network gone, back"); a connected one, or one without a network, is left alone.
     */
    @Test
    fun reconnectNowOnlyKicksAReconnectingPlayer() = runTest {
        val r = repo()
        r.reconnectNow() // no player: nothing to do, no crash
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        val h = handles[0]
        h.observer!!.onStatus(status())
        r.reconnectNow()
        assertEquals("connected: untouched", listOf(true), h.networks)
        h.observer!!.onStatus(status(connection = PlayerConnection.Reconnecting(8_000uL)))
        r.reconnectNow()
        assertEquals(listOf(true, false, true), h.networks)
        // No network: nothing to reconnect over (the engine waits for the network itself).
        network.value = NetworkKind.OFFLINE
        val before = h.networks.toList()
        r.reconnectNow()
        assertEquals(before, h.networks)
    }

    /** Spools go at sign-out, revocation and a change of server or user, not when the player is merely switched off. */
    @Test
    fun spoolsAreClearedWhenTheSignInEnds() = runTest {
        val r = repo()
        settings.setPlayOnPhone(true)
        val first = Session(NoHandle)
        sessions.value = first
        assertEquals(emptyList<String>(), cleared)
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals("switching the player off and on keeps nothing to clear", emptyList<String>(), cleared)
        // Signed out (or revoked): cleared once the player has stopped.
        sessions.value = null
        assertTrue(handles.last().stopped)
        assertEquals(listOf("/data/app/stream-cache"), cleared)
        // Signing in again clears nothing; another server or user does.
        sessions.value = Session(NoHandle)
        assertEquals(1, cleared.size)
        sessions.value = Session(NoHandle)
        assertEquals(2, cleared.size)
        // Even with this phone's player off, a sign-out clears what a crash may have left.
        r.setEnabled(false)
        sessions.value = null
        assertEquals(3, cleared.size)
    }

    /** The cushion shows only while reading ahead, from a second up, and not once stopped. */
    @Test
    fun theCushionShownComesFromTheStatus() {
        val s = status().copy(cushionMs = 240_000u, readAhead = true)
        assertEquals(240_000L, LocalPlayerState(enabled = true, status = s).cushionMs)
        assertNull(LocalPlayerState(enabled = true, status = s.copy(readAhead = false)).cushionMs)
        assertNull(LocalPlayerState(enabled = true, status = s.copy(cushionMs = 0u)).cushionMs)
        assertNull(LocalPlayerState(enabled = true, status = s.copy(cushionMs = 999u)).cushionMs)
        assertNull(LocalPlayerState(enabled = true, status = s.copy(connection = PlayerConnection.Stopped("x"))).cushionMs)
        assertNull(LocalPlayerState().cushionMs)
    }

    /** The output clock shown comes from the status; nothing while no output is open. */
    @Test
    fun theOutputTimingComesFromTheStatus() {
        val s = status().copy(outputClock = OutputClock.DEVICE, outputLatencyMs = 38u, outputClockErrors = 0u)
        assertEquals(OutputTiming(OutputClock.DEVICE, 38, 0), LocalPlayerState(enabled = true, status = s).outputTiming)
        assertEquals(
            OutputTiming(OutputClock.ESTIMATED, 90, 12),
            LocalPlayerState(enabled = true, status = s.copy(outputClock = OutputClock.ESTIMATED, outputLatencyMs = 90u, outputClockErrors = 12u)).outputTiming,
        )
        assertEquals(
            OutputTiming(OutputClock.DEVICE, 38, 0, networkFlapsPerMinute = 3),
            LocalPlayerState(enabled = true, status = s.copy(networkFlaps = 3u)).outputTiming,
        )
        assertNull(LocalPlayerState(enabled = true, status = s.copy(outputClock = null)).outputTiming)
        assertNull(LocalPlayerState(enabled = true, status = status()).outputTiming)
        assertNull(LocalPlayerState().outputTiming)
    }

    /** Streaming quality: the setting for the network the phone is on, before the engine starts and on every change. */
    @Test
    fun theStreamLevelFollowsTheNetworkAndTheSetting() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        // Wi-Fi by default: Original, set before the engine starts.
        assertEquals(listOf(StreamQuality.ORIGINAL), handles[0].qualities)
        assertEquals(1, handles[0].qualitiesAtStart)
        assertEquals(StreamQuality.ORIGINAL, r.state.value.streamQuality)
        // Mobile data: High by default.
        network.value = NetworkKind.MOBILE
        assertEquals(listOf(StreamQuality.ORIGINAL, StreamQuality.HIGH), handles[0].qualities)
        assertEquals(StreamQuality.HIGH, r.state.value.streamQuality)
        assertTrue(r.state.value.metered)
        // The user picks Data saver for mobile data.
        settings.setStreaming(StreamingSettings(mobile = StreamQuality.LOW))
        assertEquals(StreamQuality.LOW, handles[0].qualities.last())
        // Back on Wi-Fi; offline counts as unmetered (nothing streams then anyway).
        network.value = NetworkKind.WIFI
        assertEquals(StreamQuality.ORIGINAL, handles[0].qualities.last())
        network.value = NetworkKind.OFFLINE
        assertFalse(r.state.value.metered)
        // A new engine is told the current level before it starts.
        network.value = NetworkKind.MOBILE
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf(StreamQuality.LOW), handles.last().qualities)
        assertEquals(1, handles.last().qualitiesAtStart)
    }

    /** The visualiser (Wave 2 §4.1): off by default, remembered for a new engine, frames only while on and well-formed. */
    @Test
    fun theVisualiserIsOffByDefaultAndRemembered() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(listOf(false), handles[0].visual)
        handles[0].frame = ByteArray(VisualFrame.BYTES)
        assertNull("off: no frames", r.visualFrame())
        r.setVisualiser(true)
        assertEquals(listOf(false, true), handles[0].visual)
        assertEquals(VisualFrame.BANDS, r.visualFrame()!!.bands.size)
        // A frame of the wrong size is refused.
        handles[0].frame = ByteArray(VisualFrame.BYTES - 1)
        assertNull(r.visualFrame())
        // A new engine starts with it on.
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf(true), handles.last().visual)
        r.setVisualiser(false)
        assertEquals(listOf(true, false), handles.last().visual)
    }

    @Test
    fun theLevelForANetwork() {
        val s = StreamingSettings(wifi = StreamQuality.HIGH, mobile = StreamQuality.LOW)
        assertEquals(StreamQuality.LOW, streamQualityFor(true, s))
        assertEquals(StreamQuality.HIGH, streamQualityFor(false, s))
        assertEquals(StreamQuality.HIGH, streamQualityFor(true, StreamingSettings()))
    }

    /** The sleep fade (Wave 1): passed to the engine once per change, checked, and again for a new engine. */
    @Test
    fun theSleepFadeReachesTheEngineOncePerChange() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        r.setSleepFade(600.0, 30)
        // Every status repeats the timer: the same end (within polling drift) isn't passed again.
        clockMs += 5_000
        r.setSleepFade(595.5, 30)
        assertEquals(listOf("fade 600.0 30"), handles[0].calls.filter { it.startsWith("fade") })
        // Re-armed, a new length, cancelled: each passed on.
        r.setSleepFade(20.0, 30)
        r.setSleepFade(20.0, 60)
        r.setSleepFade(null, 60)
        r.setSleepFade(null, 60)
        assertEquals(
            listOf("fade 600.0 30", "fade 20.0 30", "fade 20.0 60", "fade null 60"),
            handles[0].calls.filter { it.startsWith("fade") },
        )
        // Junk never reaches the engine as a timer; lengths are clamped.
        for (bad in listOf(Double.NaN, Double.POSITIVE_INFINITY, -5.0, 0.0, 1e9)) r.setSleepFade(bad, 30)
        r.setSleepFade(100.0, 5_000)
        r.setSleepFade(200.0, -3)
        val last = handles[0].calls.filter { it.startsWith("fade") }.takeLast(3)
        assertEquals(listOf("fade null 30", "fade 100.0 120", "fade 200.0 0"), last)
        // A new engine (restarted) is told again.
        r.setEnabled(false)
        r.setEnabled(true)
        r.setSleepFade(200.0, 0)
        assertEquals(listOf("fade 200.0 0"), handles.last().calls.filter { it.startsWith("fade") })
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
    fun soundIsBitPerfectByDefaultAndAppliesLive() = runTest {
        val r = repo()
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        // Bit-perfect: the engine keeps its default (no DSP call at all).
        assertEquals(listOf("route ", "start"), handles[0].calls)
        assertTrue(r.state.value.sound.bitPerfect)

        r.setSound(SoundSettings(levelling = LevellingMode.R128))
        assertEquals(LevellingMode.R128, settings.state.value.sound.levelling)
        assertEquals(LevellingMode.R128, r.state.value.sound.levelling)
        assertEquals("dsp true", handles[0].calls.last())

        r.setSound(SoundSettings())
        assertEquals(DEFAULT_DSP, handles[0].dsp.last())

        // A player started while a stage is on gets it before any audio.
        r.setSound(SoundSettings(preampDb = -3.0))
        r.setEnabled(false)
        r.setEnabled(true)
        assertEquals(listOf("route ", "dsp false", "start"), handles.last().calls)
        assertEquals(-3.0, handles.last().dsp.single().preampDb, 0.0)
    }

    @Test
    fun theBitPerfectSwitchRenegotiatesAndTheNegotiatorReachesThePlayer() = runTest {
        val r = repo()
        val neg = object : OutputNegotiator {
            override fun offers() = OutputOffers("", OfferBlocker.NO_USB_DAC, emptyList())

            override fun apply(offer: MixerOffer?) = false
        }
        r.setOutputNegotiator(neg)
        sessions.value = Session(NoHandle)
        r.setEnabled(true)
        assertEquals(listOf("route ", "negotiator true", "start"), handles[0].calls)
        r.setSound(SoundSettings(bitPerfectUsb = false))
        assertEquals("renegotiate", handles[0].calls.last())
        assertTrue("no DSP change", handles[0].dsp.isEmpty())
        r.renegotiateOutput()
        r.setOutputNegotiator(null)
        assertEquals(listOf("renegotiate", "negotiator false"), handles[0].calls.takeLast(2))
    }

    @Test
    fun soundSettingsMapToTheEngine() {
        assertEquals(DEFAULT_DSP, dspFor(SoundSettings()))
        val off = DEFAULT_DSP
        assertFalse(off.enabled || off.analysisLevelling)
        assertEquals(Levelling.OFF, off.levelling)
        assertNull(off.eqProfile)
        assertEquals(CrossfeedSetting.OFF, off.crossfeed)

        val r128 = dspFor(SoundSettings(levelling = LevellingMode.R128))
        assertTrue(r128.enabled && r128.analysisLevelling && r128.limiter)
        assertEquals(Levelling.TRACK, r128.levelling)
        assertEquals(-18.0, r128.targetLufs, 0.0)
        val album = dspFor(SoundSettings(levelling = LevellingMode.REPLAYGAIN_ALBUM))
        assertEquals(Levelling.ALBUM, album.levelling)
        assertFalse(album.analysisLevelling)
        val track = dspFor(SoundSettings(levelling = LevellingMode.REPLAYGAIN_TRACK))
        assertEquals(Levelling.TRACK, track.levelling)
        assertFalse(track.analysisLevelling)

        val eq = EqProfile("Headphones", "Filter 1: ON PK Fc 1000 Hz Gain 3 dB Q 1", 1, 0.0)
        val all = dspFor(
            SoundSettings(
                preampDb = -2.5,
                limiter = false,
                limiterCeilingDb = -0.5,
                eqEnabled = true,
                eq = eq,
                crossfeed = CrossfeedMode.NATURAL,
                dither = DitherMode.TPDF_SHAPED,
            ),
        )
        assertTrue(all.enabled)
        assertEquals(-2.5, all.preampDb, 0.0)
        assertFalse(all.limiter)
        assertEquals(-0.5, all.limiterCeilingDb, 0.0)
        assertEquals(eq.text, all.eqProfile)
        assertEquals(CrossfeedSetting.NATURAL, all.crossfeed)
        assertEquals(DitherSetting.TPDF_SHAPED, all.dither)
        // A loaded but switched-off EQ isn't applied (and alone keeps bit-perfect).
        val eqOff = SoundSettings(eq = eq, eqEnabled = false)
        assertTrue(eqOff.bitPerfect)
        assertNull(dspFor(eqOff).eqProfile)
        assertFalse(dspFor(eqOff).enabled)
        // Dither alone changes nothing; it's still passed on for when depth is reduced.
        val shaped = dspFor(SoundSettings(dither = DitherMode.TPDF_SHAPED))
        assertFalse(shaped.enabled)
        assertEquals(DitherSetting.TPDF_SHAPED, shaped.dither)
    }

    @Test
    fun transitionsReachTheEngineWithoutSwitchingTheChainOn() {
        // Off by default: gapless, bit-perfect.
        assertEquals(TransitionMode.OFF, dspFor(SoundSettings()).transitions)
        val x = dspFor(SoundSettings(transitions = TransitionsMode.CROSSFADE, crossfadeSecs = 3.0))
        assertEquals(TransitionMode.CROSSFADE, x.transitions)
        assertEquals(3.0, x.crossfadeS, 0.0)
        assertFalse("a crossfade alone leaves the DSP chain off", x.enabled)
        val smart = dspFor(
            SoundSettings(transitions = TransitionsMode.SMART, preampDb = -3.0, crossfadeSecs = 99.0),
        )
        assertEquals(TransitionMode.SMART, smart.transitions)
        assertEquals(10.0, smart.crossfadeS, 0.0)
        assertTrue(smart.enabled)
        assertEquals(-3.0, smart.preampDb, 0.0)
    }

    @Test
    fun outOfRangeSoundSettingsAreClamped() {
        val s = SoundSettings(preampDb = 99.0, limiterCeilingDb = -99.0).sanitised()
        assertEquals(12.0, s.preampDb, 0.0)
        assertEquals(-6.0, s.limiterCeilingDb, 0.0)
        val n = SoundSettings(preampDb = Double.NaN, limiterCeilingDb = Double.POSITIVE_INFINITY).sanitised()
        assertEquals(0.0, n.preampDb, 0.0)
        assertEquals(-1.0, n.limiterCeilingDb, 0.0)
        assertEquals(-12.0, dspFor(SoundSettings(preampDb = -40.0)).preampDb, 0.0)
        // An oversized profile is dropped, and the EQ is then off.
        val huge = EqProfile("x", "#".repeat(SoundSettings.MAX_EQ_CHARS + 1), 0, 0.0)
        val h = SoundSettings(eq = huge, eqEnabled = true).sanitised()
        assertNull(h.eq)
        assertFalse(h.eqEnabled)
        // An EQ switched on with no profile does nothing.
        assertTrue(SoundSettings(eqEnabled = true).bitPerfect)
    }

    @Test
    fun eqProfilesAreCheckedByTheEngineAndOversizedOnesRefusedFirst() = runTest {
        val seen = mutableListOf<String>()
        val d = UnconfinedTestDispatcher(testScheduler)
        val r = RustLocalPlayerRepository(
            sessions,
            settings,
            CoroutineScope(backgroundScope.coroutineContext + d),
            d,
            eqCheck = { seen += it; EqCheck.Ok(2, -3.0, false) },
        )
        assertEquals(EqCheck.Ok(2, -3.0, false), r.checkEq("Preamp: -3 dB"))
        assertTrue(r.checkEq("x".repeat(SoundSettings.MAX_EQ_CHARS + 1)) is EqCheck.Invalid)
        assertEquals(listOf("Preamp: -3 dB"), seen)
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
        r.setOutputRoute(OutputRoute("USB DAC", 96_000u))
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
