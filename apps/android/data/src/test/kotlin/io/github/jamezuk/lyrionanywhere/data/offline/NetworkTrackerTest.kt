package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Test

/**
 * Android reports the default network lost and back within a moment whenever it picks the
 * default again (live, 2026-10-15: the player's socket was dropped about once a minute for
 * it). A loss only counts once no default network has come back for the grace; Wi-Fi and
 * mobile data still change at once (the downloads policy follows them).
 */
@OptIn(ExperimentalCoroutinesApi::class)
class NetworkTrackerTest {
    private val wifi = "wifi"
    private val cell = "cell"
    private val grace = DefaultNetworkTracker.OFFLINE_GRACE_MS

    /** A tracker on Wi-Fi, and every value it shows from then on. */
    private fun TestScope.tracker(): Pair<DefaultNetworkTracker, List<NetworkKind>> {
        val t = DefaultNetworkTracker(wifi, NetworkKind.WIFI, backgroundScope)
        val seen = mutableListOf<NetworkKind>()
        backgroundScope.launch(UnconfinedTestDispatcher(testScheduler)) { t.kind.toList(seen) }
        return t to seen
    }

    @Test
    fun aLossFollowedByANetworkWithinTheGraceIsNeverOffline() = runTest {
        val (t, seen) = tracker()
        t.onLost(wifi)
        advanceTimeBy(300)
        runCurrent()
        t.onCapabilities(wifi, NetworkKind.WIFI)
        advanceTimeBy(grace * 3)
        runCurrent()
        assertEquals(listOf(NetworkKind.WIFI), seen)
        assertEquals(NetworkKind.WIFI, t.kind.value)
    }

    @Test
    fun aLossAloneIsOfflineAfterTheGrace() = runTest {
        val (t, seen) = tracker()
        t.onLost(wifi)
        advanceTimeBy(grace - 1)
        runCurrent()
        assertEquals(NetworkKind.WIFI, t.kind.value)
        advanceTimeBy(2)
        runCurrent()
        assertEquals(NetworkKind.OFFLINE, t.kind.value)
        assertEquals(listOf(NetworkKind.WIFI, NetworkKind.OFFLINE), seen)
        // Back: at once.
        t.onCapabilities(cell, NetworkKind.MOBILE)
        runCurrent()
        assertEquals(listOf(NetworkKind.WIFI, NetworkKind.OFFLINE, NetworkKind.MOBILE), seen)
    }

    @Test
    fun aWifiToMobileHandoverNeverPassesThroughOffline() = runTest {
        val (t, seen) = tracker()
        t.onLost(wifi)
        t.onCapabilities(cell, NetworkKind.MOBILE)
        runCurrent()
        assertEquals(NetworkKind.MOBILE, t.kind.value)
        advanceTimeBy(grace * 3)
        runCurrent()
        assertEquals(listOf(NetworkKind.WIFI, NetworkKind.MOBILE), seen)
    }

    /** The new default's capabilities can come before the old one's loss: that loss is old news. */
    @Test
    fun aLateLossOfTheReplacedNetworkIsIgnored() = runTest {
        val (t, seen) = tracker()
        t.onCapabilities(cell, NetworkKind.MOBILE)
        t.onLost(wifi)
        advanceTimeBy(grace * 3)
        runCurrent()
        assertEquals(listOf(NetworkKind.WIFI, NetworkKind.MOBILE), seen)
    }

    /** Capabilities that keep INTERNET (validation, metering, signal) change the kind at most. */
    @Test
    fun capabilityChangesThatKeepTheInternetAreNeverOffline() = runTest {
        val (t, seen) = tracker()
        repeat(5) {
            t.onCapabilities(wifi, NetworkKind.WIFI)
            t.onCapabilities(wifi, NetworkKind.MOBILE)
            t.onCapabilities(wifi, NetworkKind.WIFI)
        }
        advanceTimeBy(grace * 3)
        runCurrent()
        assertEquals(NetworkKind.WIFI, t.kind.value)
        assertEquals(false, NetworkKind.OFFLINE in seen)
    }

    /** The default network losing INTERNET counts as a loss: the same grace. */
    @Test
    fun losingTheInternetCapabilityIsALossWithTheSameGrace() = runTest {
        val (t, seen) = tracker()
        t.onCapabilities(wifi, NetworkKind.OFFLINE)
        advanceTimeBy(grace / 2)
        runCurrent()
        t.onCapabilities(wifi, NetworkKind.WIFI)
        advanceTimeBy(grace * 3)
        runCurrent()
        assertEquals(listOf(NetworkKind.WIFI), seen)
        t.onCapabilities(wifi, NetworkKind.OFFLINE)
        advanceTimeBy(grace + 1)
        runCurrent()
        assertEquals(NetworkKind.OFFLINE, t.kind.value)
    }

    /** Repeated losses don't stack up timers: one OFFLINE, one grace after the first. */
    @Test
    fun repeatedLossesGiveOneOffline() = runTest {
        val (t, seen) = tracker()
        t.onLost(wifi)
        advanceTimeBy(grace / 2)
        t.onLost(wifi)
        t.onLost(cell)
        advanceTimeBy(grace / 2 + 1)
        runCurrent()
        assertEquals(listOf(NetworkKind.WIFI, NetworkKind.OFFLINE), seen)
        advanceTimeBy(grace * 3)
        runCurrent()
        assertEquals(listOf(NetworkKind.WIFI, NetworkKind.OFFLINE), seen)
    }

    @Test
    fun startingWithNoNetworkIsOfflineAtOnce() = runTest {
        val t = DefaultNetworkTracker(null, NetworkKind.OFFLINE, backgroundScope)
        assertEquals(NetworkKind.OFFLINE, t.kind.value)
        t.onCapabilities(wifi, NetworkKind.WIFI)
        assertEquals(NetworkKind.WIFI, t.kind.value)
    }
}
