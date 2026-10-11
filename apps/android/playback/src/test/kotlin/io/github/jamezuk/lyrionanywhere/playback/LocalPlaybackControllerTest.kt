package io.github.jamezuk.lyrionanywhere.playback

import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.ServerEvent
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class LocalPlaybackControllerTest {
    private val local = FakeLocal()
    private val players = FakePlayers()
    private val events = FakeEvents()
    private val art = FakeArt()

    private fun TestScope.controller() =
        LocalPlaybackController(local, players, events, art, backgroundScope) { testScheduler.currentTime }
            .also {
                it.start()
                runCurrent()
                advanceTimeBy(LocalPlaybackController.COALESCE_MS + 1)
            }

    @Test
    fun readsLmsStatusQueueAndArtForThisPhone() = runTest {
        val c = controller()
        val s = c.snapshot.value
        assertEquals(players.status, s.status)
        assertEquals(QUEUE, s.queue)
        assertEquals("music/11/cover", s.artwork?.key)
        assertEquals(listOf("music/11/cover"), art.fetched)
    }

    @Test
    fun eventsForThisPlayerRefreshOthersDoNot() = runTest {
        val c = controller()
        val before = players.statusCalls
        events.flow.emit(StreamUpdate.Event(ServerEvent.PlayerChanged("aa:bb:cc:dd:ee:01", "pause", emptyList())))
        advanceTimeBy(LocalPlaybackController.COALESCE_MS + 1)
        assertEquals(before, players.statusCalls)
        events.flow.emit(StreamUpdate.Event(ServerEvent.PlayerChanged(PHONE, "pause", listOf("1"))))
        advanceTimeBy(LocalPlaybackController.COALESCE_MS + 1)
        assertEquals(before + 1, players.statusCalls)
        assertTrue(c.snapshot.value.status != null)
    }

    @Test
    fun theSameArtIsNotFetchedTwice() = runTest {
        val c = controller()
        c.refresh()
        c.refresh()
        assertEquals(1, art.fetched.size)
    }

    @Test
    fun actionsGoToLmsForThisPlayer() = runTest {
        val c = controller()
        c.pause()
        c.play()
        c.next()
        c.previous()
        c.seekTo(42.0)
        c.jumpTo(2)
        assertEquals(
            listOf("pause true $PHONE", "play $PHONE", "next $PHONE", "previous $PHONE", "seek 42.0 $PHONE", "jump 2 $PHONE"),
            players.commands,
        )
        assertTrue("play lifts a local hold", local.calls.contains("hold false"))
    }

    @Test
    fun becomingNoisySilencesFirstThenPauses() = runTest {
        val c = controller()
        c.becomingNoisy()
        assertEquals("hold true", local.calls.first())
        assertEquals(listOf("pause true $PHONE"), players.commands)
    }

    @Test
    fun noPlayerNoCommandsAndAnEmptySnapshot() = runTest {
        local.state.value = LocalPlayerState(enabled = true)
        val c = controller()
        c.play()
        assertTrue(players.commands.isEmpty())
        c.refresh()
        assertNull(c.snapshot.value.status)
    }

    @Test
    fun anEngineStateChangeTriggersARefresh() = runTest {
        controller()
        val before = players.statusCalls
        local.state.value = running(LocalPlayback.PAUSED)
        runCurrent()
        advanceTimeBy(LocalPlaybackController.COALESCE_MS + 1)
        assertEquals(before + 1, players.statusCalls)
    }
}
