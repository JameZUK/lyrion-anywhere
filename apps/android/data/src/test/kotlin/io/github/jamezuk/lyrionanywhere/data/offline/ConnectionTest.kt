package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionJudge.Companion.FOREGROUND_GRACE_MS
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionJudge.Companion.NETWORK_GRACE_MS
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionJudge.Companion.TAKEOVER_MS
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionJudge.Companion.TROUBLE_GRACE_MS
import io.github.jamezuk.lyrionanywhere.data.status
import kotlin.random.Random
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
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

/** The connection decision (SPEC §8.4): what the screen shows about the link to the server. */
class ConnectionJudgeTest {
    private val signedIn = AuthState.SignedInState(SERVER, who())

    /** Drives a judge through time; the inputs are changed with [set] and time with [at]. */
    private class Rig(auth: AuthState) {
        val judge = ConnectionJudge()
        var now = 0L
        var inputs = ConnectionInputs(auth, netDown = false, trouble = false, foreground = true, playing = false)
        var ui = judge.update(now, inputs)

        fun set(f: (ConnectionInputs) -> ConnectionInputs): ConnectionUi {
            inputs = f(inputs)
            ui = judge.update(now, inputs)
            return ui
        }

        /** Move to [t] ms, re-deciding at each deadline on the way (as the controller does). */
        fun at(t: Long): ConnectionUi {
            while (true) {
                val next = judge.nextCheckIn(now)?.let { now + it }
                if (next == null || next > t) break
                now = next
                ui = judge.update(now, inputs)
            }
            now = t
            ui = judge.update(now, inputs)
            return ui
        }
    }

    @Test
    fun aSteadyConnectionShowsNothing() {
        val r = Rig(signedIn)
        assertEquals(ConnectionUi(), r.ui)
        assertEquals(ConnectionUi(), r.at(10 * 60_000L))
    }

    @Test
    fun troubleInTheBackgroundWhileNotPlayingIsIgnored() {
        val r = Rig(signedIn)
        r.set { it.copy(foreground = false) }
        // Android cuts the network: the phone's player reconnects for as long as it likes.
        r.set { it.copy(trouble = true) }
        assertEquals(ConnectionUi(), r.at(60 * 60_000L))
    }

    @Test
    fun backInTheForegroundTheReconnectHasItsGrace() {
        val r = Rig(signedIn)
        r.set { it.copy(foreground = false, trouble = true) }
        r.at(30 * 60_000L)
        val back = r.now
        // Opened with the player still reconnecting: the pill, nothing more, through the grace.
        val ui = r.set { it.copy(foreground = true) }
        assertEquals(ConnectionUi(reconnecting = true), ui)
        assertEquals(ConnectionUi(reconnecting = true), r.at(back + FOREGROUND_GRACE_MS - 1))
        // Reconnected within it: nothing at all, and the place is never left.
        r.set { it.copy(trouble = false) }
        assertEquals(ConnectionUi(), r.ui)
    }

    @Test
    fun afterTheGraceTroubleCountsTheBannerThenTheTakeover() {
        val r = Rig(signedIn)
        r.set { it.copy(foreground = false) }
        r.at(1_000)
        r.set { it.copy(foreground = true, trouble = true) }
        val back = r.now
        val counted = back + FOREGROUND_GRACE_MS
        assertFalse(r.at(counted + TROUBLE_GRACE_MS - 1).unreachable)
        // The banner, on the screen the user is on.
        assertEquals(ConnectionUi(unreachable = true), r.at(counted + TROUBLE_GRACE_MS))
        assertFalse(r.at(counted + TAKEOVER_MS - 1).takeover)
        // Unreachable for 15 s while in use: offline mode takes the screen.
        assertEquals(ConnectionUi(takeover = true), r.at(counted + TAKEOVER_MS))
        // … and keeps it until the link is back, then gives it back at once.
        assertTrue(r.at(counted + 10 * TAKEOVER_MS).takeover)
        assertEquals(ConnectionUi(), r.set { it.copy(trouble = false) })
    }

    @Test
    fun theFifteenSecondRuleWhileInUse() {
        val r = Rig(signedIn)
        r.at(FOREGROUND_GRACE_MS + 1) // long past the start-up grace
        val start = r.now
        r.set { it.copy(trouble = true) }
        assertEquals(ConnectionUi(reconnecting = true), r.ui)
        assertEquals(ConnectionUi(reconnecting = true), r.at(start + TROUBLE_GRACE_MS - 1))
        assertEquals(ConnectionUi(unreachable = true), r.at(start + TROUBLE_GRACE_MS))
        assertEquals(ConnectionUi(unreachable = true), r.at(start + TAKEOVER_MS - 1))
        assertEquals(ConnectionUi(takeover = true), r.at(start + TAKEOVER_MS))
    }

    @Test
    fun aShortDropNeverTakesOver() {
        val r = Rig(signedIn)
        r.at(FOREGROUND_GRACE_MS + 1)
        repeat(5) {
            r.set { it.copy(trouble = true) }
            r.at(r.now + TAKEOVER_MS - 1_000)
            assertFalse(r.ui.takeover)
            r.set { it.copy(trouble = false) }
            assertEquals(ConnectionUi(), r.ui)
            r.at(r.now + 1_000)
        }
    }

    @Test
    fun goingToTheBackgroundStopsTheTakeoverClock() {
        val r = Rig(signedIn)
        r.at(FOREGROUND_GRACE_MS + 1)
        r.set { it.copy(trouble = true) }
        r.at(r.now + TAKEOVER_MS - 1_000)
        r.set { it.copy(foreground = false) }
        val ui = r.at(r.now + 10 * TAKEOVER_MS)
        assertFalse(ui.takeover)
        assertFalse("ignored in the background", ui.unreachable)
    }

    @Test
    fun noNetworkIsNoticedAtOnceEvenInTheBackground() {
        val r = Rig(signedIn)
        r.set { it.copy(foreground = false) }
        r.at(5_000)
        r.set { it.copy(netDown = true) }
        assertFalse("a handover is ridden out", r.at(5_000 + NETWORK_GRACE_MS - 1).unreachable)
        assertEquals(ConnectionUi(unreachable = true), r.at(5_000 + NETWORK_GRACE_MS))
        // Opened with no network: offline mode at once.
        assertEquals(ConnectionUi(takeover = true), r.set { it.copy(foreground = true) })
        // The network back and the player fine: back to the app.
        assertEquals(ConnectionUi(), r.set { it.copy(netDown = false) })
    }

    @Test
    fun noNetworkWhileInUseIsTheBannerThenTheTakeover() {
        val r = Rig(signedIn)
        r.at(FOREGROUND_GRACE_MS + 1)
        val start = r.now
        // Even within the grace a real loss counts: here, well after it.
        assertEquals(ConnectionUi(reconnecting = true), r.set { it.copy(netDown = true) })
        assertEquals(ConnectionUi(unreachable = true), r.at(start + NETWORK_GRACE_MS))
        assertEquals(ConnectionUi(takeover = true), r.at(start + TAKEOVER_MS))
    }

    @Test
    fun noNetworkWithinTheGraceCountsToo() {
        val r = Rig(signedIn)
        r.set { it.copy(foreground = false) }
        r.at(1_000)
        r.set { it.copy(foreground = true) }
        val back = r.now
        r.set { it.copy(netDown = true) }
        assertTrue(r.at(back + NETWORK_GRACE_MS).unreachable)
    }

    @Test
    fun playingInTheBackgroundStillCounts() {
        val r = Rig(signedIn)
        r.set { it.copy(foreground = false, playing = true) }
        r.at(1_000)
        r.set { it.copy(trouble = true) }
        assertFalse(r.at(1_000 + TROUBLE_GRACE_MS - 1).unreachable)
        assertEquals(ConnectionUi(unreachable = true), r.at(1_000 + TROUBLE_GRACE_MS))
        // Never a takeover while out of sight: that waits for the app to be in use.
        assertFalse(r.at(1_000 + 10 * TAKEOVER_MS).takeover)
        val open = r.now
        val ui = r.set { it.copy(foreground = true) }
        assertEquals("playing: no grace on return", ConnectionUi(unreachable = true), ui)
        assertFalse(r.at(open + TAKEOVER_MS - 1).takeover)
        assertTrue(r.at(open + TAKEOVER_MS).takeover)
    }

    @Test
    fun theSignInDecidesFirst() {
        // A sign-in that couldn't be checked: offline mode (with downloads to show).
        assertEquals(ConnectionUi(takeover = true), Rig(AuthState.Unreachable(SERVER)).ui)
        // Signed out, no server, starting: nothing about the server.
        for (a in listOf(AuthState.SignedOut(SERVER), AuthState.NoServer, AuthState.Starting)) {
            val r = Rig(a)
            r.set { it.copy(netDown = true, trouble = true) }
            assertEquals(a.toString(), ConnectionUi(), r.at(10 * TAKEOVER_MS))
        }
        // A retry in progress keeps what was shown (no flicker).
        val r = Rig(AuthState.Unreachable(SERVER))
        assertTrue(r.set { it.copy(auth = AuthState.Restoring(SERVER)) }.takeover)
        assertEquals(ConnectionUi(), Rig(AuthState.Restoring(SERVER)).ui)
        // The retry worked: back to the app, the link being fine.
        assertEquals(ConnectionUi(), r.set { it.copy(auth = signedIn) })
    }

    @Test
    fun nextCheckInIsTheNearestDeadline() {
        val j = ConnectionJudge()
        val i = ConnectionInputs(signedIn, netDown = false, trouble = false, foreground = true, playing = false)
        j.update(0, i)
        assertEquals(FOREGROUND_GRACE_MS, j.nextCheckIn(0))
        j.update(FOREGROUND_GRACE_MS, i)
        assertNull("nothing pending", j.nextCheckIn(FOREGROUND_GRACE_MS))
        j.update(FOREGROUND_GRACE_MS, i.copy(trouble = true))
        assertEquals(TROUBLE_GRACE_MS, j.nextCheckIn(FOREGROUND_GRACE_MS))
        assertNotNull(j.nextCheckIn(FOREGROUND_GRACE_MS + TROUBLE_GRACE_MS))
    }

    /** Arbitrary inputs and times (backwards too): never a crash, and the outputs stay coherent. */
    @Test
    fun robustOnArbitraryInputs() {
        val rnd = Random(20261013)
        val auths = listOf(
            AuthState.Starting,
            AuthState.NoServer,
            AuthState.SignedOut(SERVER),
            AuthState.Restoring(SERVER),
            AuthState.Unreachable(SERVER),
            signedIn,
        )
        repeat(200) {
            val j = ConnectionJudge()
            var now = rnd.nextLong(-1_000_000, 1_000_000)
            repeat(200) {
                now += rnd.nextLong(-5_000, 40_000)
                val i = ConnectionInputs(auths.random(rnd), rnd.nextBoolean(), rnd.nextBoolean(), rnd.nextBoolean(), rnd.nextBoolean())
                val ui = j.update(now, i)
                if (ui.takeover) assertFalse(ui.unreachable || ui.reconnecting)
                assertFalse(ui.unreachable && ui.reconnecting)
                if (i.auth !is AuthState.SignedInState) assertFalse(ui.unreachable || ui.reconnecting)
                if (i.auth is AuthState.Unreachable) assertTrue(ui.takeover)
                if (!i.foreground) assertFalse(ui.reconnecting)
                j.nextCheckIn(now)?.let { assertTrue(it > 0) }
            }
        }
    }
}

@OptIn(ExperimentalCoroutinesApi::class)
class ConnectionControllerTest {
    private class Rig(scope: TestScope) {
        val auth = MutableStateFlow<AuthState>(AuthState.Starting)
        val network = MutableStateFlow(NetworkKind.WIFI)
        val trouble = MutableStateFlow(false)
        val foreground = MutableStateFlow(true)
        val playing = MutableStateFlow(false)
        var retries = 0
        var kicks = 0
        val c = OfflineModeController(
            CoroutineScope(scope.backgroundScope.coroutineContext + UnconfinedTestDispatcher(scope.testScheduler)),
            auth,
            network,
            trouble,
            retry = { retries++ },
            foreground = foreground,
            playing = playing,
            reconnectNow = { kicks++ },
            now = { scope.testScheduler.currentTime },
        ).also { it.start() }
    }

    @Test
    fun theScreenFollowsTheJudgeOverTime() = runTest {
        val r = Rig(this)
        r.auth.value = AuthState.SignedInState(SERVER, who())
        advanceTimeBy(FOREGROUND_GRACE_MS + 1)
        r.trouble.value = true
        runCurrent()
        assertEquals(ConnectionUi(reconnecting = true), r.c.ui.value)
        advanceTimeBy(TROUBLE_GRACE_MS)
        runCurrent()
        assertEquals(ConnectionUi(unreachable = true), r.c.ui.value)
        advanceTimeBy(TAKEOVER_MS - TROUBLE_GRACE_MS)
        runCurrent()
        assertEquals(ConnectionUi(takeover = true), r.c.ui.value)
        r.trouble.value = false
        runCurrent()
        assertEquals(ConnectionUi(), r.c.ui.value)
    }

    @Test
    fun backgroundTroubleDoesNotReachTheScreenButAutoStillSeesIt() = runTest {
        val r = Rig(this)
        r.auth.value = AuthState.SignedInState(SERVER, who())
        r.foreground.value = false
        r.trouble.value = true
        advanceTimeBy(10 * OfflineModeController.TROUBLE_GRACE_MS)
        runCurrent()
        assertEquals(ConnectionUi(), r.c.ui.value)
        // Android Auto's measure is unchanged (it browses the downloads instead).
        assertTrue(r.c.offline.value)
    }

    @Test
    fun returningToTheForegroundReconnectsAtOnce() = runTest {
        val r = Rig(this)
        r.auth.value = AuthState.SignedInState(SERVER, who())
        runCurrent()
        assertEquals(0, r.kicks)
        r.foreground.value = false
        runCurrent()
        r.foreground.value = true
        runCurrent()
        assertEquals("the phone's player is kicked", 1, r.kicks)
        assertEquals(0, r.retries)
        // A sign-in that couldn't be checked is tried again on return (with a network).
        r.auth.value = AuthState.Unreachable(SERVER)
        r.foreground.value = false
        runCurrent()
        r.foreground.value = true
        runCurrent()
        assertEquals(1, r.retries)
        assertEquals(1, r.kicks)
        r.network.value = NetworkKind.OFFLINE
        r.foreground.value = false
        runCurrent()
        r.foreground.value = true
        runCurrent()
        assertEquals("no network: no retry", 1, r.retries)
    }

    @Test
    fun playingIsThePhonePlayerPlaying() {
        assertFalse(OfflineModeController.playingOf(LocalPlayerState()))
        assertTrue(OfflineModeController.playingOf(LocalPlayerState(enabled = true, status = status())))
        val paused = status().copy(playback = LocalPlayback.PAUSED)
        assertFalse(OfflineModeController.playingOf(LocalPlayerState(enabled = true, status = paused)))
        val buffering = status(connection = PlayerConnection.Reconnecting(1_000uL)).copy(playback = LocalPlayback.BUFFERING)
        assertTrue(OfflineModeController.playingOf(LocalPlayerState(enabled = true, status = buffering)))
        assertFalse("switched off", OfflineModeController.playingOf(LocalPlayerState(enabled = false, status = status())))
    }
}
