package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.RatingStyle
import io.github.jamezuk.lyrionanywhere.core.TrackRating
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.DefaultPlayer
import io.github.jamezuk.lyrionanywhere.data.RatingsRepository
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerMessage
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerViewModel
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEdit
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

private const val KITCHEN = "aa:00:00:00:00:01"
private const val PHONE = "02:4c:41:00:00:01"

/** Queue paging, safe removal, volume keys and clearing ratings on the player view model. */
@OptIn(ExperimentalCoroutinesApi::class)
class QueueVolumeRatingsTests : MainDispatcherTest() {
    private val vms = mutableListOf<PlayerViewModel>()

    private fun vmTest(body: suspend TestScope.() -> Unit) = runTest(dispatcher) {
        try {
            body()
        } finally {
            vms.forEach { it.close() }
        }
    }

    /** A view model controlling the kitchen player (not this phone). */
    private fun kitchen(
        players: FakePlayers,
        local: FakeLocalPlayer = FakeLocalPlayer(),
        ratings: RatingsRepository? = null,
    ): PlayerViewModel {
        val settings = FakeSettings()
        settings.state.update { it.copy(defaultPlayer = DefaultPlayer.LAST_USED, selectedPlayerId = KITCHEN) }
        val vm = PlayerViewModel(players, FakeEvents(), settings, local, ratings = ratings) { 0L }
        vms += vm
        vm.start()
        vm.select(KITCHEN)
        return vm
    }

    private fun longQueue(players: FakePlayers, n: Int) {
        players.queueTracks = (1..n).map { sampleTrack(it.toLong(), it - 1) }
        players.statusResult = ApiResult.Ok(sampleStatus().copy(queueLength = n.toUInt()))
    }

    // --- queue paging -----------------------------------------------------------------------

    @Test
    fun theQueueIsReadAPageAtATimeAsTheListScrolls() = vmTest {
        val players = FakePlayers()
        longQueue(players, 450)
        val vm = kitchen(players)
        vm.loadQueue()
        assertEquals(PlayerViewModel.QUEUE_PAGE, vm.state.value.queue.size)
        assertTrue(vm.state.value.queueMore)
        vm.loadMoreQueue()
        assertEquals(400, vm.state.value.queue.size)
        vm.loadMoreQueue()
        assertEquals(450, vm.state.value.queue.size)
        assertFalse(vm.state.value.queueMore)
        assertEquals((1L..450L).toList(), vm.state.value.queue.map { it.track.id })
        // Keys stay unique, so the list keeps each row's state.
        assertEquals(450, vm.state.value.queue.map { it.key }.toSet().size)
        // Nothing more to read: no further calls.
        // (Page reads only: the move's own one-entry check is not a page.)
        fun pages() = players.queueCalls.count { it.second > 1 }
        val calls = pages()
        vm.loadMoreQueue()
        assertEquals(calls, pages())
    }

    @Test
    fun aRefreshKeepsThePagesAlreadyShown() = vmTest {
        val players = FakePlayers()
        longQueue(players, 450)
        val vm = kitchen(players)
        vm.loadQueue()
        vm.loadMoreQueue()
        assertEquals(400, vm.state.value.queue.size)
        // A track change reads the queue again: as far as the list had got, a page at a time.
        players.queueCalls.clear()
        vm.loadQueue()
        assertEquals(400, vm.state.value.queue.size)
        assertEquals(listOf(0 to 200, 200 to 200), players.queueCalls)
        // Another player: back to the first page.
        vm.select("aa:00:00:00:00:02")
        vm.loadQueue()
        assertEquals(PlayerViewModel.QUEUE_PAGE, vm.state.value.queue.size)
    }

    @Test
    fun editsWorkBeyondTheFirstPage() = vmTest {
        val players = FakePlayers()
        longQueue(players, 300)
        val vm = kitchen(players)
        vm.loadQueue()
        vm.loadMoreQueue()
        assertEquals(300, vm.state.value.queue.size)
        vm.removeFromQueue(250)
        assertTrue("remove 250 $KITCHEN" in players.commands)
        vm.moveInQueue(260, 10)
        assertTrue("move 260 10 $KITCHEN" in players.commands)
    }

    @Test
    fun aPageArrivingAfterAnEditIsDropped() = vmTest {
        val players = FakePlayers()
        longQueue(players, 300)
        val gate = kotlinx.coroutines.CompletableDeferred<Unit>()
        val vm = PlayerViewModel(HeldPlayers(players, gate), FakeEvents(), FakeSettings(), FakeLocalPlayer()) { 0L }
        vms += vm
        vm.start()
        vm.select(KITCHEN)
        vm.loadQueue()
        assertEquals(200, vm.state.value.queue.size)
        // An edit on its way: no page is read onto a list about to change.
        vm.moveInQueue(0, 1)
        val calls = players.queueCalls.size
        vm.loadMoreQueue()
        assertEquals(calls, players.queueCalls.size)
        gate.complete(Unit)
        runCurrent()
        // Once it is done, the list is read afresh and paging works again.
        vm.loadMoreQueue()
        assertEquals(300, vm.state.value.queue.size)
    }

    // --- removal checks the track first -----------------------------------------------------

    @Test
    fun removalChecksTheTrackIsStillThere() = vmTest {
        val players = FakePlayers()
        val vm = kitchen(players)
        vm.loadQueue()
        val got = mutableListOf<PlayerMessage>()
        backgroundScope.launch { vm.messages.collect { got += it } }
        // Meanwhile the first track ended and another controller added one at the top.
        players.queueTracks = listOf(sampleTrack(9, 0), sampleTrack(1, 1), sampleTrack(2, 2))
        vm.removeFromQueue(1)
        assertFalse(players.commands.any { it.startsWith("remove") })
        assertEquals(PlayerMessage.QueueChanged, got.single())
        // The queue was read again.
        assertEquals(listOf(9L, 1L, 2L), vm.state.value.queue.map { it.track.id })
    }

    @Test
    fun upNextIsOnlyRemovedIfItIsStillNext() = vmTest {
        val players = FakePlayers()
        val vm = kitchen(players)
        val next = sampleTrack(2, 1).copy(queueIndex = 1u)
        players.statusResult = ApiResult.Ok(sampleStatus().copy(queueIndex = 0u, next = next))
        vm.refresh()
        val got = mutableListOf<PlayerMessage>()
        backgroundScope.launch { vm.messages.collect { got += it } }
        // The queue moved on: another track is at that place now.
        players.queueTracks = listOf(sampleTrack(1, 0), sampleTrack(5, 1))
        vm.removeUpNext()
        assertFalse(players.commands.any { it.startsWith("remove") })
        assertEquals(PlayerMessage.QueueChanged, got.single())
        // The status was read again, so "Up next" is shown once more.
        assertEquals(2L, vm.state.value.status?.next?.id)
        // Still there: removed.
        players.queueTracks = listOf(sampleTrack(1, 0), sampleTrack(2, 1))
        vm.removeUpNext()
        assertTrue("remove 1 $KITCHEN" in players.commands)
    }

    @Test
    fun sameTrackComparesIdsOrElseTitles() {
        assertTrue(QueueEdit.sameTrack(sampleTrack(3), sampleTrack(3)))
        assertFalse(QueueEdit.sameTrack(sampleTrack(3), sampleTrack(4)))
        assertFalse(QueueEdit.sameTrack(null, sampleTrack(4)))
        val radio = sampleTrack(1).copy(id = null, title = "Radio")
        assertTrue(QueueEdit.sameTrack(radio.copy(), radio))
        assertFalse(QueueEdit.sameTrack(radio.copy(title = "Other"), radio))
    }

    // --- volume keys ------------------------------------------------------------------------

    @Test
    fun volumeKeysStepARemotePlayer() = vmTest {
        val players = FakePlayers()
        val vm = kitchen(players)
        // Each step builds on the volume shown (40 → 45 → 50).
        players.statusResult = ApiResult.Ok(sampleStatus().copy(volume = 45u))
        assertTrue(vm.volumeKey(up = true))
        assertTrue("volume 45 $KITCHEN" in players.commands)
        assertTrue(vm.volumeKey(up = true))
        assertTrue("volume 50 $KITCHEN" in players.commands)
        // Bounded at 0 and 100.
        players.statusResult = ApiResult.Ok(sampleStatus().copy(volume = 98u))
        vm.refresh()
        vm.volumeKey(up = true)
        assertTrue("volume 100 $KITCHEN" in players.commands)
        players.statusResult = ApiResult.Ok(sampleStatus().copy(volume = 2u))
        vm.refresh()
        vm.volumeKey(up = false)
        assertTrue("volume 0 $KITCHEN" in players.commands)
    }

    @Test
    fun aRemotePlayerWithAFixedLevelIgnoresTheVolumeKeysAndSlider() = vmTest {
        val players = FakePlayers()
        players.statusResult = ApiResult.Ok(sampleStatus().copy(fixedLevel = true))
        val vm = kitchen(players)
        assertTrue(vm.state.value.status?.fixedLevel == true)
        // The key is used (the phone's own volume doesn't move either), but nothing is sent.
        assertTrue(vm.volumeKey(up = true))
        assertTrue(vm.volumeKey(up = false))
        vm.setVolume(60)
        assertFalse(players.commands.any { it.startsWith("volume") })
        // Its volume control back in LMS: the keys step it again.
        players.statusResult = ApiResult.Ok(sampleStatus())
        vm.refresh()
        assertTrue(vm.volumeKey(up = true))
        assertTrue("volume 45 $KITCHEN" in players.commands)
    }

    @Test
    fun volumeKeysAreLeftToTheSystemForThisPhoneOrNoPlayer() = vmTest {
        val players = FakePlayers()
        players.players = players.players + samplePlayer(PHONE)
        val local = FakeLocalPlayer()
        local.state.value = local.state.value.copy(enabled = true, status = localStatus(PHONE))
        val settings = FakeSettings()
        val vm = PlayerViewModel(players, FakeEvents(), settings, local) { 0L }.also { vms += it }
        // No player yet.
        assertFalse(vm.volumeKey(up = true))
        vm.start()
        vm.select(PHONE)
        assertTrue(vm.state.value.localSelected)
        assertFalse(vm.volumeKey(up = true))
        assertFalse(players.commands.any { it.startsWith("volume") })
    }

    // --- clearing ratings -------------------------------------------------------------------

    private class Ratings : RatingsRepository {
        var mine: UInt? = 80u
        var clearResult: ApiResult<Long> = ApiResult.Ok(12)
        override suspend fun style() = ApiResult.Ok(RatingStyle.STARS)
        override suspend fun setStyle(style: RatingStyle) = ApiResult.Ok(Unit)
        override suspend fun ratings(trackIds: List<Long>) =
            ApiResult.Ok(trackIds.map { TrackRating(it, mine, null, mine) })
        override suspend fun rate(trackId: Long, value: Int): ApiResult<TrackRating?> = ApiResult.Ok(null)
        override suspend fun clear(): ApiResult<Long> {
            if (clearResult is ApiResult.Ok) mine = null
            return clearResult
        }
    }

    @Test
    fun clearingRatingsForgetsTheCachedOnes() = vmTest {
        val players = FakePlayers()
        val ratings = Ratings()
        val vm = kitchen(players, ratings = ratings)
        assertEquals(80u, vm.state.value.rating?.mine)
        assertEquals(12L, vm.clearRatings())
        // Read afresh, not from the cache: no rating any more.
        assertNull(vm.state.value.rating?.mine)
        // A later track change back to it doesn't bring the stale rating back either.
        players.statusResult = ApiResult.Ok(sampleStatus().copy(current = sampleTrack(2, 1)))
        vm.refresh()
        players.statusResult = ApiResult.Ok(sampleStatus())
        vm.refresh()
        assertNull(vm.state.value.rating?.mine)
    }

    @Test
    fun aFailedClearSaysSoAndKeepsTheRatings() = vmTest {
        val players = FakePlayers()
        val ratings = Ratings().apply { clearResult = ApiResult.Err(AppError.Offline()) }
        val vm = kitchen(players, ratings = ratings)
        assertNull(vm.clearRatings())
        assertEquals(80u, vm.state.value.rating?.mine)
    }
}

/** Players whose queue edits wait for [gate]. */
private class HeldPlayers(
    private val inner: FakePlayers,
    private val gate: kotlinx.coroutines.CompletableDeferred<Unit>,
) : io.github.jamezuk.lyrionanywhere.data.PlayersRepository by inner {
    override suspend fun moveInQueue(player: String, from: Int, to: Int): ApiResult<Unit> {
        gate.await()
        return inner.moveInQueue(player, from, to)
    }

    override suspend fun moveIfStill(player: String, from: Int, to: Int, trackId: Long): ApiResult<Boolean> {
        gate.await()
        return inner.moveIfStill(player, from, to, trackId)
    }
}
