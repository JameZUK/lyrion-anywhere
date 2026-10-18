package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.core.OfflineTrackRef
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.resolveOfflineTracks
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.AuthState
import io.github.jamezuk.lyrionanywhere.data.ConnectionSource
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.WRONG_ACCOUNT
import io.github.jamezuk.lyrionanywhere.data.apiCall
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/*
 * "Continue on <player>" (SPEC §8.4): back online, the queue the offline player was playing
 * from this phone's downloads can be handed to an LMS player. Never by itself: the app
 * offers it, and the user picks the player or says "Not now".
 *
 * 1. Each queued download is checked with GET /v1/download/track/{id}/info (the id it was
 *    downloaded with, through the Rust layer's `resolveOfflineTracks`, in the queue owner's
 *    own library, which must be the signed-in account's): the id is used only when the
 *    gateway says it still names the download's own file (its stable key). A track
 *    LMS no longer has, or whose id now names another file, is skipped and counted. Any
 *    other failure stops the hand-off before anything is sent to a player.
 * 2. The confirmed ids go to the player in play order through the gateway's RPC policy:
 *    power on, shuffle off, the id lists (`Players.loadTrackList`), the position, and a
 *    pause if the downloads were paused.
 * 3. Only then is the offline player stopped. A failure on the way leaves it as it was.
 */

/** A queued download: the id it was downloaded with, and its stable key. */
data class QueuedDownload(val trackId: Long, val key: String)

/** The offline queue at one moment, in play order. */
data class OfflineQueueSnapshot(
    /** Whose downloads they are: only that account may continue them. */
    val owner: Owner,
    val tracks: List<QueuedDownload>,
    /** The current track (an index into [tracks]). */
    val index: Int,
    val positionMs: Long,
    val playing: Boolean,
)

/** The offline player's queue now (in play order), or null with nothing queued. */
fun snapshotOf(state: OfflinePlayerState, owner: Owner?): OfflineQueueSnapshot? {
    val s = state.status ?: return null
    val o = owner ?: return null
    val current = s.index?.toInt() ?: return null
    val order = s.order.map { it.toInt() }.ifEmpty { s.keys.indices.toList() }
    val tracks = order.mapNotNull { i ->
        s.keys.getOrNull(i)?.let { k -> QueuedDownload(state.tracks[k]?.trackId ?: 0, k) }
    }
    val at = order.indexOf(current)
    if (tracks.isEmpty() || at !in tracks.indices) return null
    return OfflineQueueSnapshot(o, tracks, at, s.elapsedMs.toLong(), state.playing)
}

/**
 * Looks [owner]'s queued downloads up: each one's current LMS id, in order (null: not in the
 * library as that file).
 */
fun interface TrackResolver {
    suspend fun resolve(owner: Owner, tracks: List<QueuedDownload>): ApiResult<List<Long?>>
}

/**
 * The real [TrackResolver]: `/info` through the signed-in session, in [owner]'s library
 * ([storeOf]: the open one if it is theirs). The Rust layer refuses a library that isn't the
 * signed-in account's (review M14), and looks up only tracks that library holds.
 */
class RustTrackResolver(
    private val source: ConnectionSource,
    private val storeOf: (Owner) -> OfflineStore?,
) : TrackResolver {
    override suspend fun resolve(owner: Owner, tracks: List<QueuedDownload>): ApiResult<List<Long?>> {
        val c = source.connection.value ?: return ApiResult.Err(AppError.SignedOut)
        val library = (storeOf(owner) as? RustOfflineStore)?.library
            ?: return ApiResult.Err(AppError.Forbidden(WRONG_ACCOUNT))
        return apiCall { resolveOfflineTracks(c.session, library, tracks.map { OfflineTrackRef(it.trackId, it.key) }) }
    }
}

/** What goes to the player. */
data class HandOffPlan(
    val ids: List<Long>,
    /** Index into [ids] to play from. */
    val start: Int,
    /** Into that track (0 when the current track itself was skipped). */
    val seekMs: Long,
    val paused: Boolean,
    /** Tracks that couldn't be handed over. */
    val skipped: Int,
)

/** What happened. */
sealed interface HandOffResult {
    data class Moved(val moved: Int, val skipped: Int) : HandOffResult

    /** Nothing queued. */
    data object Empty : HandOffResult

    /** None of the tracks is in the library any more: nothing sent. */
    data object NoneResolved : HandOffResult

    /** The queue kept changing (or changed hands) while it was checked: nothing sent. */
    data object Changed : HandOffResult

    /** It failed; the downloads carry on here. */
    data class Failed(val error: AppError) : HandOffResult
}

object HandOff {
    /** Most tracks handed over: a longer queue keeps this many from around the current track. */
    const val MAX_TRACKS = 1000

    /** A second or two in is as good as the start. */
    const val MIN_SEEK_MS = 2_000L

    /** Pure: what to load, from the snapshot and each track's resolved id; null when nothing resolved. */
    fun plan(s: OfflineQueueSnapshot, resolved: List<Long?>): HandOffPlan? {
        val n = s.tracks.size
        val from = if (n > MAX_TRACKS) s.index.coerceAtMost(n - MAX_TRACKS).coerceAtLeast(0) else 0
        val to = minOf(n, from + MAX_TRACKS)
        val ids = ArrayList<Long>()
        var start = -1
        var exact = false
        for (i in from until to) {
            val id = resolved.getOrNull(i) ?: continue
            if (i == s.index) {
                start = ids.size
                exact = true
            } else if (i > s.index && start < 0) {
                start = ids.size
            }
            ids.add(id)
        }
        if (ids.isEmpty()) return null
        // The current track and everything after it skipped: from the last one before it.
        if (start < 0) start = ids.size - 1
        return HandOffPlan(ids, start, if (exact) s.positionMs.coerceAtLeast(0) else 0, !s.playing, n - ids.size)
    }
}

/**
 * Hands the offline queue to a player. [current] is the queue now (read again after the
 * lookup, as it may change meanwhile).
 */
class ContinueOnUseCase(
    private val players: PlayersRepository,
    private val resolver: TrackResolver,
    private val stopLocal: () -> Unit,
    private val current: () -> OfflineQueueSnapshot?,
    /** A monotonic clock (ms), to allow for the time the hand-off takes. */
    private val clock: () -> Long,
) {
    suspend fun continueOn(target: String): HandOffResult {
        var snap = current() ?: return HandOffResult.Empty
        val known = HashMap<QueuedDownload, Long?>()
        var tries = 0
        while (true) {
            val todo = snap.tracks.filter { it !in known }.distinct()
            if (todo.isNotEmpty()) {
                when (val r = resolver.resolve(snap.owner, todo)) {
                    is ApiResult.Err -> return HandOffResult.Failed(r.error)
                    is ApiResult.Ok -> {
                        if (r.value.size != todo.size) return HandOffResult.Failed(AppError.Unexpected("lookup answer"))
                        todo.zip(r.value).forEach { (t, id) -> known[t] = id?.takeIf { it == t.trackId } }
                    }
                }
            }
            val fresh = current() ?: return HandOffResult.Empty
            if (fresh.owner != snap.owner) return HandOffResult.Changed
            val same = fresh.tracks == snap.tracks
            snap = fresh
            if (same) break
            if (++tries > 2) return HandOffResult.Changed
        }
        val plan = HandOff.plan(snap, snap.tracks.map { known[it] }) ?: return HandOffResult.NoneResolved
        val readAt = clock()
        val steps = buildList<suspend () -> ApiResult<Unit>> {
            add { players.setPower(target, true) }
            // The list goes in play order, so the player must not shuffle it again.
            add { players.setShuffle(target, ShuffleMode.OFF) }
            add { players.loadTrackList(target, plan.ids, plan.start) }
            if (plan.seekMs >= HandOff.MIN_SEEK_MS) {
                add {
                    // Playing: where the downloads have got to by now, not where they were.
                    val ms = plan.seekMs + if (plan.paused) 0 else (clock() - readAt).coerceAtLeast(0)
                    players.seek(target, (ms / 1000).toDouble())
                }
            }
            if (plan.paused) add { players.setPaused(target, true) }
        }
        for (step in steps) {
            val r = step()
            if (r is ApiResult.Err) return HandOffResult.Failed(r.error)
        }
        stopLocal()
        return HandOffResult.Moved(plan.ids.size, plan.skipped)
    }
}

/**
 * When to offer the hand-off (pure; tested on the JVM). The app was offline and is signed in
 * and online again, with a queue of downloads: offer it once. A queue that went with an
 * ended sign-in is remembered (in memory) and offered once the same account is back; never
 * to anyone else.
 */
class HandOffOffer {
    var offered: Owner? = null
        private set
    private var wasOffline = false
    private var kept: OfflineQueueSnapshot? = null

    fun wentOffline() {
        wasOffline = true
    }

    /** The queue is going with the sign-in (not a sign-out the user asked for): keep it, paused. */
    fun keep(s: OfflineQueueSnapshot?) {
        if (s != null) kept = s.copy(playing = false)
    }

    /** Signed in as [owner] and online again; [live] is the offline player's queue now. */
    fun backOnline(owner: Owner, live: OfflineQueueSnapshot?) {
        if (!wasOffline) return
        wasOffline = false
        if (kept?.owner != owner) kept = null
        offered = owner.takeIf { live?.owner == owner || kept != null }
    }

    /** The queue to hand over now (the live one first), or null: then there's nothing to offer. */
    fun queue(live: OfflineQueueSnapshot?): OfflineQueueSnapshot? {
        val o = offered ?: return null
        return live?.takeIf { it.owner == o } ?: kept?.takeIf { it.owner == o }
    }

    /** "Not now", or done. */
    fun dismiss() {
        offered = null
        kept = null
    }

    /** Signed out on purpose: forget everything. */
    fun reset() {
        dismiss()
        wasOffline = false
    }
}

/** What the prompt shows. */
data class ContinueOnState(
    /** Offered: the queue's size and whether it is playing (null: not offered). */
    val tracks: Int? = null,
    val playing: Boolean = false,
    /** Handing over to this player (by name). */
    val busy: String? = null,
    /** How the last attempt went (shown until dismissed or tried again). */
    val result: HandOffResult? = null,
    /** The player [result] is about. */
    val resultTarget: String = "",
) {
    val offered: Boolean get() = tracks != null
}

/**
 * Watches the link and the offline player for "Continue on <player>", and runs it. One per
 * process (the offer outlives the screens: offline mode takes the whole screen meanwhile).
 */
class ContinueOnController(
    private val scope: CoroutineScope,
    private val auth: StateFlow<AuthState>,
    private val connection: StateFlow<ConnectionUi>,
    private val network: StateFlow<NetworkKind>,
    private val offlinePlayer: OfflinePlayerRepository,
    private val owner: StateFlow<OfflineState>,
    private val players: PlayersRepository,
    private val resolver: TrackResolver,
    private val localPlayer: LocalPlayerRepository? = null,
    private val clock: () -> Long = { System.nanoTime() / 1_000_000 },
    /** Waits between checks for this phone's player to appear (tests pass a short one). */
    private val phoneWaitMs: Long = 500,
) {
    private val offer = HandOffOffer()
    private val _state = MutableStateFlow(ContinueOnState())
    val state: StateFlow<ContinueOnState> = _state.asStateFlow()

    private fun live(): OfflineQueueSnapshot? = snapshotOf(offlinePlayer.state.value, owner.value.owner)

    private fun signedInOwner(a: AuthState): Owner? =
        (a as? AuthState.SignedInState)?.let { Owner(it.server.url, it.who.user.username) }

    fun start() {
        scope.launch {
            combine(auth, connection, network) { a, c, n ->
                val offline = a is AuthState.Unreachable || c.takeover || c.unreachable || n == NetworkKind.OFFLINE
                Triple(a, offline, signedInOwner(a)?.takeIf { !offline })
            }.distinctUntilChanged().collect { (a, offline, online) ->
                when {
                    offline -> offer.wentOffline()
                    online != null -> offer.backOnline(online, live())
                    // The sign-in ended (not on purpose: that calls [forget] first): keep the queue.
                    a is AuthState.SignedOut -> if (a.revoked) offer.reset() else offer.keep(lastLive)
                }
                publish()
            }
        }
        scope.launch {
            offlinePlayer.state.collect {
                live()?.let { s -> lastLive = s }
                publish()
            }
        }
    }

    /** The last queue seen, for an ended sign-in (the player is released with the library). */
    @Volatile private var lastLive: OfflineQueueSnapshot? = null

    private fun publish() {
        val q = offer.queue(live())
        if (q == null && offer.offered != null && _state.value.busy == null) offer.dismiss()
        _state.update { it.copy(tracks = q?.tracks?.size, playing = q?.playing == true) }
    }

    /** "Not now" (also hides the last result). */
    fun dismiss() {
        offer.dismiss()
        _state.update { ContinueOnState() }
    }

    /** Hide the last result. */
    fun clearResult() = _state.update { it.copy(result = null, resultTarget = "") }

    /** The user signs out: forget the queue and the offer. */
    fun forget() {
        offer.reset()
        lastLive = null
        _state.update { ContinueOnState() }
    }

    /**
     * Hand the queue to [targetId] (a player from the "Play on…" list), or with [targetId]
     * null to this phone's own player, started first if need be. [onDone] gets the result
     * and the player it went to.
     */
    fun continueOn(targetId: String?, targetName: String, onDone: (HandOffResult, String?) -> Unit = { _, _ -> }) {
        if (_state.value.busy != null || !_state.value.offered) return
        _state.update { it.copy(busy = targetName, result = null, resultTarget = "") }
        scope.launch {
            var id = targetId
            val r = try {
                id = id ?: phoneId()
                val to = id
                if (to == null) {
                    HandOffResult.Failed(AppError.Unexpected("this phone couldn’t start as a player"))
                } else {
                    ContinueOnUseCase(players, resolver, { offlinePlayer.stop() }, { offer.queue(live()) }, clock)
                        .continueOn(to)
                }
            } catch (e: kotlinx.coroutines.CancellationException) {
                throw e
            } catch (e: Exception) {
                HandOffResult.Failed(AppError.Unexpected(e.message ?: "failed"))
            }
            if (r is HandOffResult.Moved) offer.dismiss()
            _state.update {
                it.copy(busy = null, result = r, resultTarget = targetName, tracks = if (r is HandOffResult.Moved) null else it.tracks)
            }
            publish()
            onDone(r, id)
        }
    }

    /** This phone's LMS player once it is on and listed (switched on and waited for if need be). */
    private suspend fun phoneId(): String? {
        val lp = localPlayer ?: return null
        if (!lp.state.value.enabled) lp.setEnabled(true)
        repeat(PHONE_TRIES) {
            val id = lp.state.value.playerId
            if (id != null && (players.list() as? ApiResult.Ok)?.value?.any { it.id == id } == true) return id
            delay(phoneWaitMs)
        }
        return null
    }

    companion object {
        /** Checks for this phone's player to appear in LMS (about 15 s). */
        const val PHONE_TRIES = 30
    }
}
