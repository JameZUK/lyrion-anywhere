package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.ApiException
import kotlin.random.Random
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.TestScope
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

@OptIn(ExperimentalCoroutinesApi::class)
class ServerBusyTest {
    private fun TestScope.tracker() = ServerBusyTracker(
        CoroutineScope(backgroundScope.coroutineContext + UnconfinedTestDispatcher(testScheduler)),
    ) { testScheduler.currentTime }

    @Test
    fun busyFromTheStreamLastsUntilOk() = runTest {
        val t = tracker()
        val a = Any()
        val b = Any()
        assertFalse(t.busy.value)
        t.streamBusy(a)
        t.streamBusy(b)
        assertTrue(t.busy.value)
        advanceTimeBy(10 * 60_000L)
        runCurrent()
        assertTrue("no timeout: the stream says when it is over", t.busy.value)
        // One subscription reconnecting: the other still says busy.
        t.streamGone(a)
        assertTrue(t.busy.value)
        // "ok" on any subscription: it is one server.
        t.streamOk()
        assertFalse(t.busy.value)
    }

    @Test
    fun aSubscriptionThatReconnectsNoLongerCounts() = runTest {
        val t = tracker()
        val s = Any()
        t.streamBusy(s)
        t.streamGone(s)
        assertFalse("the gateway says again after subscribing if still busy", t.busy.value)
        t.streamGone(Any())
        assertFalse(t.busy.value)
    }

    @Test
    fun aRefusalCountsForItsRetryAfterAndALittleMore() = runTest {
        val t = tracker()
        t.refused(2)
        assertTrue(t.busy.value)
        assertEquals(2_000L, t.retryAfterMillis)
        advanceTimeBy(2_000L + ServerBusyTracker.HOLD_MS - 1)
        runCurrent()
        assertTrue(t.busy.value)
        advanceTimeBy(1)
        runCurrent()
        assertFalse("it ends by itself", t.busy.value)
    }

    @Test
    fun anotherRefusalExtendsItAndAnAnswerEndsIt() = runTest {
        val t = tracker()
        t.refused(2)
        advanceTimeBy(10_000L)
        t.refused(2)
        advanceTimeBy(5_000L)
        runCurrent()
        assertTrue("the first one's expiry doesn't end the second", t.busy.value)
        t.answered()
        assertFalse(t.busy.value)
        // An answer doesn't override the stream, which says "ok" itself.
        t.streamBusy(this)
        t.answered()
        assertTrue(t.busy.value)
    }

    @Test
    fun hostileRetryAfterIsBounded() = runTest {
        val t = tracker()
        t.refused(Long.MAX_VALUE)
        assertEquals(ServerBusyTracker.MAX_RETRY_S * 1000, t.retryAfterMillis)
        t.refused(-5)
        assertEquals(1_000L, t.retryAfterMillis)
        t.refused(0)
        assertEquals(1_000L, t.retryAfterMillis)
        advanceTimeBy(1_000L + ServerBusyTracker.HOLD_MS)
        runCurrent()
        assertFalse(t.busy.value)
    }

    @Test
    fun resultsFeedIt() = runTest {
        val t = tracker()
        assertEquals(ApiResult.Ok(1), t.observe(ApiResult.Ok(1)))
        assertFalse(t.busy.value)
        val busy = ApiResult.Err(AppError.ServerBusy(3))
        assertEquals(busy, t.observe(busy))
        assertTrue(t.busy.value)
        assertEquals(3_000L, t.retryAfterMillis)
        // Other failures say nothing about it either way.
        t.observe(ApiResult.Err(AppError.Offline("x")))
        t.observe(ApiResult.Err(AppError.RateLimited(5)))
        t.observe(ApiResult.Err(AppError.Server(502, "x")))
        assertTrue(t.busy.value)
        t.observe(ApiResult.Ok(Unit))
        assertFalse(t.busy.value)
        t.observe(ApiResult.Err(AppError.Server(503, "x")))
        t.observe(ApiResult.Err(AppError.RateLimited(2)))
        assertFalse("a plain 503 or 429 is not busy", t.busy.value)
    }

    @Test
    fun theRustErrorMapsToItsOwnAppError() {
        assertEquals(AppError.ServerBusy(2), ApiException.ServerBusy(2u).toAppError())
        assertEquals(AppError.RateLimited(2), ApiException.RateLimited(2u).toAppError())
    }

    /** Arbitrary sequences: never a crash, and busy is exactly "a stream says so or a refusal is live". */
    @Test
    fun robustOnArbitrarySequences() = runTest {
        val rnd = Random(20261014)
        val t = tracker()
        val subs = List(3) { Any() }
        val busySubs = mutableSetOf<Any>()
        var refusedUntil: Long? = null
        repeat(2_000) {
            when (rnd.nextInt(6)) {
                0 -> subs.random(rnd).let { t.streamBusy(it); busySubs += it }
                1 -> { t.streamOk(); busySubs.clear(); refusedUntil = null }
                2 -> subs.random(rnd).let { t.streamGone(it); busySubs -= it }
                3 -> rnd.nextLong(-10, 400).let { s ->
                    t.refused(s)
                    refusedUntil = testScheduler.currentTime + s.coerceIn(1, ServerBusyTracker.MAX_RETRY_S) * 1000 + ServerBusyTracker.HOLD_MS
                }
                4 -> { t.answered(); refusedUntil = null }
                else -> advanceTimeBy(rnd.nextLong(0, 20_000))
            }
            runCurrent()
            if (refusedUntil?.let { testScheduler.currentTime >= it } == true) refusedUntil = null
            assertEquals(busySubs.isNotEmpty() || refusedUntil != null, t.busy.value)
        }
    }
}
