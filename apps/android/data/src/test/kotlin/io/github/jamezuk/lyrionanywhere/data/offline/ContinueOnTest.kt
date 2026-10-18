package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.TransferOutcome
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val KITCHEN = "aa:bb:cc:dd:ee:01"
private val ALICE = Owner(SERVER.url, "alice")
private val BOB = Owner(SERVER.url, "bob")

/** Records what is sent to players; [failAt] fails the nth command (0-based). */
class RecordingPlayers(var failAt: Int = -1, var listed: List<Player> = emptyList()) : PlayersRepository {
    val sent = mutableListOf<String>()

    private fun rec(s: String): ApiResult<Unit> {
        if (sent.size == failAt) return ApiResult.Err(AppError.Offline("connection lost"))
        sent += s
        return ApiResult.Ok(Unit)
    }

    private fun <T> no(): ApiResult<T> = ApiResult.Err(AppError.Unexpected("not in this fake"))

    override suspend fun list(): ApiResult<List<Player>> = ApiResult.Ok(listed)

    override suspend fun syncGroups() = no<List<SyncGroup>>()

    override suspend fun status(player: String) = no<PlayerStatus>()

    override suspend fun queue(player: String, start: Int, count: Int) = no<List<Track>>()

    override suspend fun play(player: String) = rec("$player play")

    override suspend fun setPaused(player: String, paused: Boolean) = rec("$player paused $paused")

    override suspend fun next(player: String) = no<Unit>()

    override suspend fun previous(player: String) = no<Unit>()

    override suspend fun jumpTo(player: String, index: Int) = no<Unit>()

    override suspend fun seek(player: String, seconds: Double) = rec("$player seek ${seconds.toLong()}")

    override suspend fun setVolume(player: String, volume: Int) = no<Unit>()

    override suspend fun setShuffle(player: String, mode: ShuffleMode) = rec("$player shuffle $mode")

    override suspend fun setRepeat(player: String, mode: RepeatMode) = no<Unit>()

    override suspend fun setPower(player: String, on: Boolean) = rec("$player power $on")

    override suspend fun enqueue(player: String, selection: Selection, placement: Placement, playIndex: Int?) = no<Unit>()

    override suspend fun playFavourite(player: String, itemId: String, placement: Placement) = no<Unit>()

    override suspend fun loadTrackList(player: String, ids: List<Long>, start: Int) = rec("$player load $ids from $start")

    override suspend fun removeFromQueue(player: String, index: Int) = no<Unit>()

    override suspend fun moveInQueue(player: String, from: Int, to: Int) = no<Unit>()

    override suspend fun clearQueue(player: String) = no<Unit>()

    override suspend fun restoreToQueue(player: String, trackId: Long, index: Int) = no<Unit>()

    override suspend fun setSynced(current: String, other: String, on: Boolean) = no<Unit>()

    override suspend fun transfer(source: String, target: String, onStage: (TransferStage) -> Unit) = no<TransferOutcome>()
}

/** The library now: id → key; others unknown. Counts the ids asked about, and whose library each lookup was in. */
class FakeResolver(private val library: Map<Long, String>, var error: AppError? = null) : TrackResolver {
    val asked = mutableListOf<Long>()
    val owners = mutableListOf<Owner>()
    var onResolve: () -> Unit = {}

    override suspend fun resolve(owner: Owner, tracks: List<QueuedDownload>): ApiResult<List<Long?>> {
        owners += owner
        error?.let { return ApiResult.Err(it) }
        asked += tracks.map { it.trackId }
        val out = tracks.map { t -> t.trackId.takeIf { library[it] == t.key } }
        onResolve()
        return ApiResult.Ok(out)
    }
}

private fun key(n: Int) = offlineTrack(n).key

private fun snap(n: Int, index: Int = 0, positionMs: Long = 0, playing: Boolean = true, owner: Owner = ALICE) =
    OfflineQueueSnapshot(owner, (1..n).map { QueuedDownload(100L + it, key(it)) }, index, positionMs, playing)

private fun all(n: Int) = (1..n).associate { 100L + it to key(it) }

class ContinueOnUseCaseTest {
    private var stopped = 0

    private fun useCase(players: PlayersRepository, resolver: TrackResolver, current: () -> OfflineQueueSnapshot?, clock: () -> Long = { 0 }) =
        ContinueOnUseCase(players, resolver, { stopped++ }, current, clock)

    @Test
    fun loadsTheResolvedQueueInOrderAtThePositionThenStopsTheDownloads() = runTest {
        val p = RecordingPlayers()
        // 102 was renumbered (its id names another file now); 105 is gone.
        val resolver = FakeResolver(all(5) - 105 + (102L to key(9)))
        val r = useCase(p, resolver, { snap(5, index = 2, positionMs = 61_500) }).continueOn(KITCHEN)
        assertEquals(HandOffResult.Moved(moved = 3, skipped = 2), r)
        // Looked up in the queue owner's own library (review M14).
        assertEquals(listOf(ALICE), resolver.owners)
        assertEquals(
            listOf(
                "$KITCHEN power true",
                "$KITCHEN shuffle OFF",
                "$KITCHEN load [101, 103, 104] from 1",
                "$KITCHEN seek 61",
            ),
            p.sent,
        )
        assertEquals(1, stopped)
    }

    @Test
    fun pausedStaysPausedAndPlayingAllowsForTheTimeTaken() = runTest {
        val paused = RecordingPlayers()
        useCase(paused, FakeResolver(all(2)), { snap(2, index = 1, positionMs = 30_000, playing = false) }).continueOn(KITCHEN)
        assertEquals("$KITCHEN seek 30", paused.sent[3])
        assertEquals("$KITCHEN paused true", paused.sent.last())

        var t = 0L
        val playing = RecordingPlayers()
        useCase(playing, FakeResolver(all(2)), { snap(2, index = 1, positionMs = 30_000) }, clock = { t.also { t += 4_000 } })
            .continueOn(KITCHEN)
        assertEquals("$KITCHEN seek 34", playing.sent.last())
        assertFalse(playing.sent.any { "paused" in it })
        // A second in is as good as the start.
        val start = RecordingPlayers()
        useCase(start, FakeResolver(all(1)), { snap(1, positionMs = 1_500) }).continueOn(KITCHEN)
        assertFalse(start.sent.any { "seek" in it })
    }

    @Test
    fun aSkippedCurrentTrackStartsTheNextFromItsStart() {
        assertEquals(HandOffPlan(listOf(101, 103), 1, 0, false, 1), HandOff.plan(snap(3, index = 1, positionMs = 50_000), listOf(101, null, 103)))
        assertEquals(HandOffPlan(listOf(101, 102), 1, 0, false, 1), HandOff.plan(snap(3, index = 2, positionMs = 50_000), listOf(101, 102, null)))
        assertNull(HandOff.plan(snap(2), listOf(null, null)))
    }

    @Test
    fun aVeryLongQueueKeepsTheStretchAroundTheCurrentTrack() {
        val n = HandOff.MAX_TRACKS + 400
        val s = snap(n, index = n - 5)
        val plan = HandOff.plan(s, s.tracks.map { it.trackId })!!
        assertEquals(HandOff.MAX_TRACKS, plan.ids.size)
        assertEquals(100L + n - 4, plan.ids[plan.start])
        assertEquals(100L + n, plan.ids.last())
        assertEquals(400, plan.skipped)
    }

    @Test
    fun aFailurePartWayLeavesTheDownloadsPlaying() = runTest {
        for (at in 0..3) {
            val p = RecordingPlayers(failAt = at)
            val r = useCase(p, FakeResolver(all(3)), { snap(3, index = 1, positionMs = 40_000) }).continueOn(KITCHEN)
            assertTrue(r is HandOffResult.Failed)
            assertEquals(0, stopped)
        }
    }

    @Test
    fun aRefusedLookupSendsNothing() = runTest {
        for (e in listOf(AppError.SignedOut, AppError.Forbidden("downloads are not allowed"), AppError.Offline("down"))) {
            val p = RecordingPlayers()
            val r = useCase(p, FakeResolver(all(2), error = e), { snap(2) }).continueOn(KITCHEN)
            assertEquals(HandOffResult.Failed(e), r)
            assertTrue(p.sent.isEmpty())
        }
        assertEquals(0, stopped)
    }

    @Test
    fun nothingResolvableOrNothingQueuedSendsNothing() = runTest {
        val p = RecordingPlayers()
        assertEquals(HandOffResult.NoneResolved, useCase(p, FakeResolver(emptyMap()), { snap(2) }).continueOn(KITCHEN))
        assertEquals(HandOffResult.Empty, useCase(p, FakeResolver(all(2)), { null }).continueOn(KITCHEN))
        assertTrue(p.sent.isEmpty())
        assertEquals(0, stopped)
    }

    @Test
    fun aQueueChangedWhileCheckedIsCheckedAgainAndTheNewOneGoes() = runTest {
        var s = snap(2)
        val resolver = FakeResolver(all(5))
        resolver.onResolve = {
            // The user picks other tracks meanwhile.
            s = OfflineQueueSnapshot(ALICE, (3..5).map { QueuedDownload(100L + it, key(it)) }, 1, 12_000, false)
        }
        val p = RecordingPlayers()
        val r = useCase(p, resolver, { s }).continueOn(KITCHEN)
        assertEquals(HandOffResult.Moved(3, 0), r)
        assertEquals(listOf(ALICE, ALICE), resolver.owners)
        assertTrue("$KITCHEN load [103, 104, 105] from 1" in p.sent)
        assertEquals("$KITCHEN paused true", p.sent.last())
        assertEquals(listOf(101L, 102L, 103L, 104L, 105L), resolver.asked)
    }

    @Test
    fun anotherAccountsQueueIsNeverContinued() = runTest {
        var n = 0
        val p = RecordingPlayers()
        val resolver = FakeResolver(all(2))
        val r = useCase(p, resolver, { if (n++ == 0) snap(2) else snap(2, owner = BOB) }).continueOn(KITCHEN)
        assertEquals(HandOffResult.Changed, r)
        assertTrue(p.sent.isEmpty())
        // Only ever looked up in the first owner's library, never bob's.
        assertEquals(listOf(ALICE), resolver.owners)
    }

    @Test
    fun bobsQueueIsLookedUpInBobsLibrary() = runTest {
        val p = RecordingPlayers()
        val resolver = FakeResolver(all(2))
        val r = useCase(p, resolver, { snap(2, owner = BOB) }).continueOn(KITCHEN)
        assertEquals(HandOffResult.Moved(2, 0), r)
        assertEquals(listOf(BOB), resolver.owners)
    }

    @Test
    fun theRealResolverNeedsASignInAndTheOwnersOpenLibrary() = runTest {
        val q = listOf(QueuedDownload(101, key(1)))
        val signedOut = object : io.github.jamezuk.lyrionanywhere.data.ConnectionSource {
            override val connection = MutableStateFlow<io.github.jamezuk.lyrionanywhere.data.Connection?>(null)
        }
        var lookedUp = 0
        val none = RustTrackResolver(signedOut) { lookedUp++; null }
        assertEquals(ApiResult.Err(AppError.SignedOut), none.resolve(ALICE, q))
        assertEquals(0, lookedUp)
    }

    @Test
    fun theSnapshotFollowsThePlayOrder() {
        val tracks = (1..3).map(::offlineTrack)
        val status = offlineStatus(index = 0, keys = tracks.map { it.key }, playback = LocalPlayback.PAUSED)
            .copy(order = listOf(2u, 0u, 1u), elapsedMs = 9_000uL, shuffle = true)
        val s = snapshotOf(OfflinePlayerState(status = status, tracks = tracks.associateBy { it.key }), ALICE)!!
        assertEquals(listOf(103L, 101L, 102L), s.tracks.map { it.trackId })
        assertEquals(1, s.index)
        assertEquals(9_000L, s.positionMs)
        assertFalse(s.playing)
        assertNull(snapshotOf(OfflinePlayerState(), ALICE))
        assertNull(snapshotOf(OfflinePlayerState(status = status), null))
    }
}

class HandOffOfferTest {
    @Test
    fun offeredOnlyAfterBeingOfflineAndOnlyWithAQueue() {
        val o = HandOffOffer()
        o.backOnline(ALICE, snap(2))
        assertNull(o.offered)
        o.wentOffline()
        o.backOnline(ALICE, null)
        assertNull(o.offered)
        o.wentOffline()
        o.backOnline(ALICE, snap(2))
        assertEquals(ALICE, o.offered)
        assertEquals(3, o.queue(snap(3))?.tracks?.size)
        assertNull(o.queue(null))
        o.dismiss()
        o.backOnline(ALICE, snap(2))
        assertNull(o.offered)
    }

    @Test
    fun aQueueEndedWithTheSignInGoesOnlyToTheSameAccountPaused() {
        val o = HandOffOffer()
        o.wentOffline()
        o.keep(snap(3, index = 1, positionMs = 20_000))
        o.backOnline(BOB, null)
        assertNull(o.offered)

        val p = HandOffOffer()
        p.wentOffline()
        p.keep(snap(3, index = 1, positionMs = 20_000))
        p.backOnline(ALICE, null)
        assertEquals(ALICE, p.offered)
        val q = p.queue(snap(1, owner = BOB))!!
        assertEquals(ALICE, q.owner)
        assertFalse(q.playing)
        p.reset()
        assertNull(p.queue(null))
    }
}

/** The offline player as the controller sees it. */
private class FakeOfflinePlayer : OfflinePlayerRepository {
    override val state = MutableStateFlow(OfflinePlayerState())
    var stops = 0

    override fun play(tracks: List<OfflineTrack>, start: Int, shuffle: Boolean) {}

    override fun play() {}

    override fun pause() {}

    override fun next() {}

    override fun previous() {}

    override fun seekTo(ms: Long) {}

    override fun jumpTo(index: Int) {}

    override fun setShuffle(on: Boolean) {}

    override fun setRepeat(repeat: LocalRepeat) {}

    override fun stop() {
        stops++
        state.value = OfflinePlayerState()
    }

    fun queue(n: Int) {
        val tracks = (1..n).map(::offlineTrack)
        state.value = OfflinePlayerState(status = offlineStatus(index = 0, keys = tracks.map { it.key }), tracks = tracks.associateBy { it.key })
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class ContinueOnControllerTest {
    private class Rig(scope: TestScope) {
        val auth = MutableStateFlow<AuthState>(AuthState.SignedInState(SERVER, who()))
        val ui = MutableStateFlow(ConnectionUi())
        val net = MutableStateFlow(NetworkKind.WIFI)
        val player = FakeOfflinePlayer()
        val offline = MutableStateFlow(OfflineState(owner = ALICE))
        val players = RecordingPlayers()
        val resolver = FakeResolver(all(3))
        val c = ContinueOnController(
            scope = scope.backgroundScope,
            auth = auth,
            connection = ui,
            network = net,
            offlinePlayer = player,
            owner = offline,
            players = players,
            resolver = resolver,
            clock = { 0 },
        ).also { it.start() }
    }

    @Test
    fun offeredBackOnlineWithAQueueAndHandedOverOnRequest() = runTest(UnconfinedTestDispatcher()) {
        val r = Rig(this)
        r.player.queue(3)
        assertFalse(r.c.state.value.offered) // never offline: nothing offered
        r.ui.value = ConnectionUi(takeover = true)
        r.ui.value = ConnectionUi()
        runCurrent()
        assertEquals(3, r.c.state.value.tracks)
        var done: HandOffResult? = null
        r.c.continueOn(KITCHEN, "Kitchen") { res, _ -> done = res }
        runCurrent()
        assertEquals(HandOffResult.Moved(3, 0), done)
        assertEquals(1, r.player.stops)
        assertFalse(r.c.state.value.offered)
        assertTrue(r.players.sent.contains("$KITCHEN load [101, 102, 103] from 0"))
        assertEquals(listOf(ALICE), r.resolver.owners)
    }

    @Test
    fun noQueueNoOfferAndNotNowSticks() = runTest(UnconfinedTestDispatcher()) {
        val r = Rig(this)
        r.net.value = NetworkKind.OFFLINE
        r.net.value = NetworkKind.WIFI
        runCurrent()
        assertFalse(r.c.state.value.offered)
        r.player.queue(2)
        r.net.value = NetworkKind.OFFLINE
        r.net.value = NetworkKind.WIFI
        runCurrent()
        assertTrue(r.c.state.value.offered)
        // The queue cleared while offered: the offer goes.
        r.player.stop()
        runCurrent()
        assertFalse(r.c.state.value.offered)
        r.player.queue(2)
        r.net.value = NetworkKind.OFFLINE
        r.net.value = NetworkKind.WIFI
        runCurrent()
        r.c.dismiss()
        assertFalse(r.c.state.value.offered)
        assertTrue(r.players.sent.isEmpty())
        assertEquals(1, r.player.stops)
    }

    @Test
    fun anExpiredSignInOffersAfterSigningInAgainAsTheSameAccountOnly() = runTest(UnconfinedTestDispatcher()) {
        val r = Rig(this)
        r.auth.value = AuthState.Unreachable(SERVER)
        r.player.queue(3)
        runCurrent()
        // The sign-in can't be renewed: signed out, the library (and its player) closes.
        r.auth.value = AuthState.SignedOut(SERVER)
        r.player.state.value = OfflinePlayerState()
        r.offline.value = OfflineState()
        runCurrent()
        r.auth.value = AuthState.SignedInState(SERVER, who())
        runCurrent()
        assertEquals(3, r.c.state.value.tracks)
        assertFalse(r.c.state.value.playing)

        // Someone else signing in instead gets nothing.
        val s = Rig(this)
        s.auth.value = AuthState.Unreachable(SERVER)
        s.player.queue(3)
        s.auth.value = AuthState.SignedOut(SERVER)
        s.player.state.value = OfflinePlayerState()
        s.auth.value = AuthState.SignedInState(SERVER, who("bob"))
        runCurrent()
        assertFalse(s.c.state.value.offered)

        // Signing out on purpose forgets it.
        val t = Rig(this)
        t.auth.value = AuthState.Unreachable(SERVER)
        t.player.queue(3)
        t.c.forget()
        t.auth.value = AuthState.SignedOut(SERVER)
        t.player.state.value = OfflinePlayerState()
        t.auth.value = AuthState.SignedInState(SERVER, who())
        runCurrent()
        assertFalse(t.c.state.value.offered)
    }

    @Test
    fun aFailedHandOffKeepsTheOfferAndTheDownloads() = runTest(UnconfinedTestDispatcher()) {
        val r = Rig(this)
        r.player.queue(2)
        r.net.value = NetworkKind.OFFLINE
        r.net.value = NetworkKind.WIFI
        r.players.failAt = 1
        r.c.continueOn(KITCHEN, "Kitchen")
        runCurrent()
        assertTrue(r.c.state.value.result is HandOffResult.Failed)
        assertTrue(r.c.state.value.offered)
        assertEquals(0, r.player.stops)
    }
}
