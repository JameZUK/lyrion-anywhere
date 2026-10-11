package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.PlaysSyncOutcome
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.launch

/**
 * Sends plays heard offline to the server (SPEC D6) whenever it can: as soon as the app is
 * signed in and online with plays waiting, then again with growing waits while some are
 * left (the server asked to retry, failed, or is an older one without `/v1/plays`).
 *
 * Sending is idempotent (each play has a key), so a retry never counts a play twice.
 */
class PlaysSender(
    private val scope: CoroutineScope,
    /** Signed in, online and with plays waiting. */
    private val ready: Flow<Boolean>,
    /** One attempt; null when it couldn't be made at all. */
    private val send: suspend () -> PlaysSyncOutcome?,
) {
    /** The last attempt's outcome (null before any). */
    private val _last = MutableStateFlow<PlaysSyncOutcome?>(null)
    val last: StateFlow<PlaysSyncOutcome?> = _last.asStateFlow()

    fun start() {
        scope.launch {
            ready.distinctUntilChanged().collectLatest { go ->
                if (!go) return@collectLatest
                var attempt = 0
                while (true) {
                    val r = send()
                    _last.value = r
                    if (r != null && !r.unsupported && r.pending == 0u) break
                    delay(waitAfter(attempt++, r?.unsupported == true))
                }
            }
        }
    }

    companion object {
        /** 1, 5, 15 then 60 minutes; an older server is asked again hourly. */
        fun waitAfter(attempt: Int, unsupported: Boolean): Long = when {
            unsupported -> 60 * MINUTE
            attempt == 0 -> MINUTE
            attempt == 1 -> 5 * MINUTE
            attempt == 2 -> 15 * MINUTE
            else -> 60 * MINUTE
        }

        private const val MINUTE = 60_000L
    }
}
