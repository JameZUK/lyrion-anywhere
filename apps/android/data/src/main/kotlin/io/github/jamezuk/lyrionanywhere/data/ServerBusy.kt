package io.github.jamezuk.lyrionanywhere.data

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Whether the music server (LMS) behind the gateway is busy: it stopped answering for a
 * while (its music share stalled, say) although the gateway itself is reachable. The app
 * then says so calmly, keeps playing what it has buffered, doesn't count it as the server
 * being unreachable (no offline mode), and backs off its polling.
 *
 * Learnt from two places, both sent only by a gateway that knows about it (otherwise this
 * stays false and nothing changes):
 * - the event stream's `{"lms":"busy"}` / `{"lms":"ok"}` ([streamBusy], [streamOk]); a
 *   subscription that reconnects or ends no longer counts ([streamGone]): the gateway says
 *   again right after subscribing if LMS is still busy;
 * - a command refused with "busy" ([refused]): busy for the Retry-After and a little more,
 *   unless another refusal follows; a command answered ([answered]) or `ok` ends it.
 */
class ServerBusyTracker(
    private val scope: CoroutineScope,
    /** A monotonic clock (ms). */
    private val now: () -> Long,
) {
    private val lock = Any()
    private val streams = mutableSetOf<Any>()
    private var refusedUntil: Long? = null
    private var retryAfterMs = DEFAULT_RETRY_MS
    private var expiry: Job? = null

    private val _busy = MutableStateFlow(false)

    /** The music server is busy (and the gateway reachable). */
    val busy: StateFlow<Boolean> = _busy.asStateFlow()

    /** How long to wait before asking again while busy (the gateway's Retry-After). */
    val retryAfterMillis: Long get() = synchronized(lock) { retryAfterMs }

    /** The event subscription [stream] says the music server stopped answering. */
    fun streamBusy(stream: Any) {
        synchronized(lock) {
            streams += stream
            publish()
        }
    }

    /** The music server answers again (told by any subscription: it is one server). */
    fun streamOk() {
        synchronized(lock) {
            streams.clear()
            refusedUntil = null
            expiry?.cancel()
            publish()
        }
    }

    /** [stream] reconnected or ended: what it said no longer holds. */
    fun streamGone(stream: Any) {
        synchronized(lock) {
            if (streams.remove(stream)) publish()
        }
    }

    /** A command was refused because the music server is busy. */
    fun refused(retryAfterSeconds: Long) {
        synchronized(lock) {
            retryAfterMs = retryAfterSeconds.coerceIn(1, MAX_RETRY_S) * 1000
            val until = now() + retryAfterMs + HOLD_MS
            refusedUntil = until
            publish()
            expiry?.cancel()
            expiry = scope.launch {
                delay(until - now())
                synchronized(lock) {
                    if (refusedUntil == until) {
                        refusedUntil = null
                        publish()
                    }
                }
            }
        }
    }

    /** A command was answered: the music server is back (as far as refusals go). */
    fun answered() {
        synchronized(lock) {
            if (refusedUntil != null) {
                refusedUntil = null
                expiry?.cancel()
                publish()
            }
        }
    }

    /** Feed a call's result: a busy refusal or an answer. */
    fun <T> observe(result: ApiResult<T>): ApiResult<T> {
        when (result) {
            is ApiResult.Ok -> answered()
            is ApiResult.Err -> (result.error as? AppError.ServerBusy)?.let { refused(it.retryAfterSeconds) }
        }
        return result
    }

    private fun publish() {
        _busy.value = streams.isNotEmpty() || refusedUntil != null
    }

    companion object {
        /** The Retry-After assumed when there is none. */
        const val DEFAULT_RETRY_MS = 2_000L

        /** A refusal counts this long past its Retry-After (unless another one follows). */
        const val HOLD_MS = 10_000L

        /** A longer Retry-After than this is not believed. */
        const val MAX_RETRY_S = 300L
    }
}
