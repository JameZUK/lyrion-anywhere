package io.github.jamezuk.lyrionanywhere.playback

import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.ServerEvent
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.channels.BufferOverflow
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/**
 * Keeps a [SessionSnapshot] of this phone's player as LMS sees it, and turns session
 * actions into LMS commands for it.
 *
 * Refreshes when LMS reports a change for this player (event stream), when the local
 * engine's state changes, and on a slow poll as a safety net.
 */
class LocalPlaybackController(
    private val local: LocalPlayerRepository,
    private val players: PlayersRepository,
    private val events: EventsRepository,
    private val art: ArtRepository,
    private val scope: CoroutineScope,
    private val clock: () -> Long,
) : SessionActions {
    private val _snapshot = MutableStateFlow(SessionSnapshot(local = local.state.value))
    val snapshot: StateFlow<SessionSnapshot> = _snapshot.asStateFlow()

    private data class Refresh(val queue: Boolean)

    private val refreshes = MutableSharedFlow<Refresh>(
        extraBufferCapacity = 4,
        onBufferOverflow = BufferOverflow.DROP_OLDEST,
    )
    private var queueWanted = true

    fun start() {
        // The refresh collector first: requests made before it subscribes would be lost.
        scope.launch(start = CoroutineStart.UNDISPATCHED) {
            refreshes.collectLatest { r ->
                queueWanted = queueWanted || r.queue
                // Coalesce bursts (LMS sends several events per change).
                delay(COALESCE_MS)
                refresh()
            }
        }
        scope.launch {
            local.state.collect { s ->
                _snapshot.update { it.copy(local = s) }
            }
        }
        scope.launch {
            // A change of player id or engine state means LMS has something new to say.
            local.state.map { it.playerId to it.status?.playback }.distinctUntilChanged().collect {
                request(queue = true)
            }
        }
        scope.launch {
            events.updates().collect { u ->
                val id = local.state.value.playerId ?: return@collect
                when (u) {
                    is StreamUpdate.Event -> when (val e = u.event) {
                        is ServerEvent.PlayerChanged ->
                            if (e.playerId == id) request(queue = e.name == "playlist" || e.name == "newmetadata")
                        ServerEvent.Resync -> request(queue = true)
                        // The music server answers again: catch up.
                        ServerEvent.LmsOk -> request(queue = true)
                        is ServerEvent.LibraryChanged, is ServerEvent.LmsBusy -> {}
                    }
                    is StreamUpdate.State -> {}
                }
            }
        }
        scope.launch {
            while (true) {
                val playing = local.state.value.status?.playback == LocalPlayback.PLAYING
                delay(if (playing) POLL_PLAYING_MS else POLL_IDLE_MS)
                if (local.state.value.running) request(queue = false)
            }
        }
    }

    private fun request(queue: Boolean) {
        refreshes.tryEmit(Refresh(queue))
    }

    /** Re-read LMS's view of this player now. */
    suspend fun refresh() {
        val id = local.state.value.playerId
        if (id == null) {
            _snapshot.update { it.copy(status = null, queue = emptyList(), artwork = null) }
            return
        }
        val at = clock()
        val status = (players.status(id) as? ApiResult.Ok)?.value ?: return
        val index = status.queueIndex?.toInt()
        val loaded = _snapshot.value.queue
        val needQueue = queueWanted ||
            (index != null && loaded.none { it.queueIndex?.toInt() == index }) ||
            loaded.size.toUInt() < minOf(status.queueLength, QUEUE_PAGE.toUInt())
        val queue = if (needQueue) {
            (players.queue(id, 0, QUEUE_PAGE) as? ApiResult.Ok)?.value ?: loaded
        } else {
            loaded
        }
        queueWanted = false
        val key = status.current?.artwork
        val artwork = when {
            key == null -> null
            _snapshot.value.artwork?.key == key -> _snapshot.value.artwork
            else -> (art.image(key, ART_PX) as? ApiResult.Ok)?.value?.let { Artwork(key, it.bytes) }
        }
        _snapshot.update { it.copy(status = status, statusAt = at, queue = queue, artwork = artwork) }
    }

    private suspend fun command(block: suspend PlayersRepository.(String) -> ApiResult<Unit>) {
        val id = local.state.value.playerId ?: return
        players.block(id)
        request(queue = false)
    }

    override suspend fun play() {
        // An explicit play from this phone also lifts any local hold at once.
        local.setHold(false)
        command { play(it) }
    }

    override suspend fun pause() = command { setPaused(it, true) }

    override suspend fun next() = command { next(it) }

    override suspend fun previous() = command { previous(it) }

    override suspend fun seekTo(seconds: Double) = command { seek(it, seconds) }

    override suspend fun jumpTo(queueIndex: Int) = command { jumpTo(it, queueIndex) }

    /** Headphones unplugged (or Bluetooth lost): silence now, then pause LMS. */
    suspend fun becomingNoisy() {
        local.setHold(true)
        command { setPaused(it, true) }
    }

    companion object {
        const val COALESCE_MS = 150L
        const val POLL_PLAYING_MS = 10_000L
        const val POLL_IDLE_MS = 30_000L
        const val QUEUE_PAGE = 100
        const val ART_PX = 512
    }
}
