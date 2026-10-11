package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.transformLatest
import kotlinx.coroutines.launch

/** True only once it has stayed true for [ms]; false at once. */
@OptIn(ExperimentalCoroutinesApi::class)
fun Flow<Boolean>.sustained(ms: Long): Flow<Boolean> = distinctUntilChanged().transformLatest { v ->
    if (v) {
        delay(ms)
        emit(true)
    } else {
        emit(false)
    }
}

/**
 * Offline mode (SPEC §8.4): when the server can't be reached the app shows the downloaded
 * library and plays locally, with an "Offline" banner; when the server is back it switches
 * back by itself.
 *
 * Offline means: the saved sign-in can't be checked (the server is unreachable), or, while
 * signed in, the phone has no network, or this phone's player has been unable to reach the
 * server for a while. While offline before sign-in, the sign-in is retried now and then and
 * as soon as the network comes back.
 */
class OfflineModeController(
    private val scope: CoroutineScope,
    private val auth: StateFlow<AuthState>,
    private val network: StateFlow<NetworkKind>,
    /** The phone's LMS player keeps failing to reach the server. */
    private val serverTrouble: Flow<Boolean>,
    /** Try the saved sign-in again. */
    private val retry: suspend () -> Unit,
) {
    private val _offline = MutableStateFlow(false)
    val offline: StateFlow<Boolean> = _offline.asStateFlow()

    fun start() {
        scope.launch {
            val netDown = network.map { it == NetworkKind.OFFLINE }.sustained(NETWORK_GRACE_MS)
            val trouble = serverTrouble.sustained(TROUBLE_GRACE_MS)
            combine(auth, netDown, trouble) { a, down, t -> Triple(a, down, t) }.collect { (a, down, t) ->
                _offline.value = decide(a, down, t, _offline.value)
            }
        }
        scope.launch {
            // Before sign-in, offline: try again now and then (not while there's no network).
            val stuck = auth.map { it is AuthState.Unreachable || (it is AuthState.Restoring && _offline.value) }
            combine(stuck.distinctUntilChanged(), network) { s, n -> s && n != NetworkKind.OFFLINE }
                .distinctUntilChanged()
                .collectLatest { should ->
                    while (should) {
                        delay(RETRY_MS)
                        retry()
                    }
                }
        }
        scope.launch {
            // … and at once when the network comes back.
            var previous = network.value
            network.collect { n ->
                if (previous == NetworkKind.OFFLINE && n != NetworkKind.OFFLINE && auth.value is AuthState.Unreachable) {
                    retry()
                }
                previous = n
            }
        }
    }

    companion object {
        /** No network for this long while signed in: offline (rides out a handover). */
        const val NETWORK_GRACE_MS = 3_000L

        /** The phone's player failing to reach the server this long: offline. */
        const val TROUBLE_GRACE_MS = 20_000L

        /** Retry the saved sign-in this often while offline. */
        const val RETRY_MS = 30_000L

        /** Pure: tested on the JVM. A sign-in being checked keeps the current mode. */
        fun decide(a: AuthState, netDown: Boolean, trouble: Boolean, current: Boolean): Boolean = when (a) {
            is AuthState.Unreachable -> true
            is AuthState.SignedInState -> netDown || trouble
            is AuthState.Restoring -> current
            else -> false
        }

        /** The phone's player can't reach the server (it is reconnecting). */
        fun troubleOf(local: LocalPlayerState): Boolean =
            local.enabled && local.status?.connection is io.github.jamezuk.lyrionanywhere.core.PlayerConnection.Reconnecting
    }
}

/**
 * Only one of the phone's two players makes sound at a time: starting downloads pauses
 * this phone's LMS player, and LMS starting this phone's player pauses the downloads.
 */
class PlaybackArbiter(
    private val scope: CoroutineScope,
    private val local: LocalPlayerRepository,
    private val players: PlayersRepository,
    private val offline: OfflinePlayerRepository,
) {
    fun start() {
        scope.launch {
            offline.state.map { it.playing }.distinctUntilChanged().collect { playing ->
                val lp = local.state.value
                val id = lp.playerId
                if (playing && id != null && lp.status?.playback == LocalPlayback.PLAYING) {
                    local.setHold(true)
                    players.setPaused(id, true)
                }
            }
        }
        scope.launch {
            local.state.map { it.status?.playback == LocalPlayback.PLAYING }.distinctUntilChanged().collect { playing ->
                if (playing && offline.state.value.playing) offline.pause()
            }
        }
    }
}
