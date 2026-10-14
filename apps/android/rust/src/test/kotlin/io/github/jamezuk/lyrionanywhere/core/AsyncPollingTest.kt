package io.github.jamezuk.lyrionanywhere.core

import java.util.concurrent.Executors
import java.util.concurrent.Semaphore
import java.util.concurrent.TimeUnit
import kotlin.coroutines.CoroutineContext
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineDispatcher
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The generated driver for Rust futures ([uniffiRustCallAsync], patched by
 * anywhere-uniffi-bindgen) against a model of UniFFI's RustFuture: it calls a continuation
 * while holding the future's scheduler lock, and a std Mutex is not re-entrant.
 *
 * Regression (Android 1.12.0 ANR in airplane mode): on the main thread
 * (Dispatchers.Main.immediate), polling one future woke a second; the second coroutine was
 * resumed inline inside the callback, polled its future again, and blocked on the lock its own
 * thread held.
 */
class AsyncPollingTest {
    /** Like Dispatchers.Main.immediate: runs inline when already on its thread. */
    private class ImmediateDispatcher : CoroutineDispatcher() {
        @Volatile lateinit var thread: Thread
        val executor = Executors.newSingleThreadExecutor { r -> Thread(r, "main").also { thread = it } }
            .also { it.submit { }.get() }

        override fun isDispatchNeeded(context: CoroutineContext) = Thread.currentThread() !== thread

        override fun dispatch(context: CoroutineContext, block: Runnable) = executor.execute(block)
    }

    /** UniFFI 0.31's RustFuture, with `ready` standing for the Rust future's own poll. */
    private class FakeRustFuture(private val ready: () -> Boolean) {
        private val futureLock = Semaphore(1)
        private val schedulerLock = Semaphore(1)
        private var stored: Pair<UniffiRustFutureContinuationCallback, Long>? = null
        private var waked = false
        private var cancelled = false
        var deadlock: String? = null
        var polls = 0

        private inline fun <R> locked(lock: Semaphore, what: String, body: () -> R): R {
            if (!lock.tryAcquire(2, TimeUnit.SECONDS)) {
                deadlock = "$what lock never freed (held by this thread: deadlock)"
                throw IllegalStateException(deadlock)
            }
            try {
                return body()
            } finally {
                lock.release()
            }
        }

        fun poll(callback: UniffiRustFutureContinuationCallback, data: Long) {
            polls++
            val isCancelled = locked(schedulerLock, "scheduler") { cancelled }
            val done = isCancelled || locked(futureLock, "future") { ready() }
            if (done) {
                callback.callback(data, UNIFFI_RUST_FUTURE_POLL_READY)
            } else {
                locked(schedulerLock, "scheduler") {
                    if (waked) {
                        waked = false
                        callback.callback(data, UNIFFI_RUST_FUTURE_POLL_WAKE)
                    } else {
                        stored = callback to data
                    }
                }
            }
        }

        /** The Rust waker: the continuation is called with the scheduler lock held. */
        fun wake() = locked(schedulerLock, "scheduler") {
            val s = stored
            if (s != null) {
                stored = null
                s.first.callback(s.second, UNIFFI_RUST_FUTURE_POLL_WAKE)
            } else {
                waked = true
            }
        }

        fun free() = locked(schedulerLock, "scheduler") {
            cancelled = true
            stored?.let { (cb, data) ->
                stored = null
                cb.callback(data, UNIFFI_RUST_FUTURE_POLL_READY)
            }
        }
    }

    private val main = ImmediateDispatcher()
    private val scope = CoroutineScope(SupervisorJob() + main)

    @After
    fun tearDown() {
        main.executor.shutdownNow()
    }

    /** Drive [f] to completion or cancellation; never reaches the FFI (nothing to complete). */
    private suspend fun drive(f: FakeRustFuture, handle: Long) {
        uniffiRustCallAsync(
            handle,
            { _, cb, data -> f.poll(cb, data) },
            { _, _ -> error("not completed in these tests") },
            { _ -> f.free() },
            { it: Unit -> it },
            UniffiNullRustCallStatusErrorHandler,
        )
    }

    @Test
    fun polling_one_future_that_wakes_another_never_deadlocks() = runBlocking {
        val secondPolledAgain = CompletableDeferred<Unit>()
        lateinit var second: FakeRustFuture
        second = FakeRustFuture {
            // The second poll (after the wake) is the one that deadlocked.
            if (second.polls >= 2) secondPolledAgain.complete(Unit)
            false
        }
        // Polling the first future releases something the second waits for.
        var wokeOnce = false
        val first = FakeRustFuture {
            if (!wokeOnce) {
                wokeOnce = true
                second.wake()
            }
            false
        }

        val b = scope.launch { drive(second, 2) }
        // Wait until the second is parked with its continuation stored.
        withTimeout(5_000) { while (second.polls < 1) delay(5) }
        delay(50)
        val a = scope.launch { drive(first, 1) }

        withTimeout(5_000) { secondPolledAgain.await() }
        assertNull(second.deadlock)
        assertNull(first.deadlock)
        a.cancelAndJoin()
        b.cancelAndJoin()
    }

    @Test
    fun rust_futures_are_never_polled_on_the_callers_thread() = runBlocking {
        val pollThreads = mutableListOf<Thread>()
        val f = FakeRustFuture {
            synchronized(pollThreads) { pollThreads += Thread.currentThread() }
            false
        }
        val job = scope.launch { drive(f, 3) }
        withTimeout(5_000) { while (f.polls < 1) delay(5) }
        job.cancelAndJoin()
        synchronized(pollThreads) {
            assertTrue(pollThreads.isNotEmpty())
            assertTrue(pollThreads.none { it === main.thread })
        }
    }

    @Test
    fun a_future_cancelled_before_polling_is_still_freed() = runBlocking {
        var freed = 0
        var polled = false
        val job = scope.launch {
            // Cancelled already: switching to the polling thread fails before polling starts.
            coroutineContext[Job]!!.cancel()
            uniffiRustCallAsync(
                5,
                { _, _, _ -> polled = true },
                { _, _ -> error("never completed") },
                { _ -> freed++ },
                { it: Unit -> it },
                UniffiNullRustCallStatusErrorHandler,
            )
        }
        job.join()
        assertEquals(1, freed)
        assertEquals(false, polled)
    }
}
