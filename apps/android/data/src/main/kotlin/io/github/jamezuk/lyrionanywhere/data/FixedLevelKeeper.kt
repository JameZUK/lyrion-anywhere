package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch
import java.util.concurrent.atomic.AtomicBoolean

/**
 * Keeps LMS's "Output level is fixed at 100%" for this phone's player in step with the
 * Sound setting [SoundSettings.fixedOutputLevel], for as long as the app runs (the Sound
 * screen comes and goes).
 *
 * The triggers:
 * - the player becoming connected (the engine's connection state, so after the gateway has
 *   greeted it, not merely once it has an id): at start, and again after every reconnect;
 * - the setting changing while it is connected;
 * - a status the app read for this player ([observeStatus]) showing LMS disagreeing with
 *   the setting (a controller flipped it back): set again, at most every
 *   [RESEND_EVERY_MS].
 *
 * On a connection or setting trigger the player's status is read once and, if LMS disagrees,
 * the preference is set once. LMS may not know the player for a moment after the greeting,
 * so a failed read or set is tried once more after [RETRY_MS]; after that a failure is logged
 * and left until the next trigger: never retried in a loop.
 */
class FixedLevelKeeper(
    private val scope: CoroutineScope,
    /** This phone's player state (its id in LMS, its connection and the Sound settings). */
    private val local: Flow<LocalPlayerState>,
    private val players: PlayersRepository,
    private val log: (String) -> Unit = { android.util.Log.w(TAG, it) },
    /** A monotonic clock (ms), for [RESEND_EVERY_MS]. */
    private val nowMs: () -> Long = { System.nanoTime() / 1_000_000 },
) {
    /** The connected player and the setting for it (null: not connected). */
    @Volatile private var current: Pair<String, Boolean>? = null

    /** When the preference was last sent (null: never). */
    @Volatile private var sentAt: Long? = null

    /** A read or set is on its way (a status seen meanwhile is not acted on). */
    private val busy = AtomicBoolean(false)

    fun start(): Job = scope.launch {
        local.map { s -> s.connectedId() to s.sound.fixedOutputLevel }
            .distinctUntilChanged()
            .collectLatest { (id, wanted) ->
                current = id?.let { it to wanted }
                if (id == null) return@collectLatest
                if (!keep(id, wanted)) {
                    delay(RETRY_MS)
                    keep(id, wanted)
                }
            }
    }

    /**
     * A status the app read for [playerId]: when it is this phone's connected player and LMS
     * disagrees with the setting, set it back (at most every [RESEND_EVERY_MS]).
     */
    fun observeStatus(playerId: String, status: PlayerStatus) {
        val (id, wanted) = current ?: return
        if (playerId != id || status.fixedLevel == wanted || busy.get()) return
        val last = sentAt
        if (last != null && nowMs() - last < RESEND_EVERY_MS) return
        if (!busy.compareAndSet(false, true)) return
        scope.launch {
            try {
                set(id, wanted)
            } finally {
                busy.set(false)
            }
        }
    }

    /** One attempt to bring LMS in line with [wanted] for [player]; false when it failed. */
    internal suspend fun keep(player: String, wanted: Boolean): Boolean {
        busy.set(true)
        try {
            val fixed = when (val s = players.status(player)) {
                is ApiResult.Ok -> s.value.fixedLevel
                is ApiResult.Err -> {
                    log("could not read the player's status: ${s.error}")
                    return false
                }
            }
            return fixed == wanted || set(player, wanted)
        } finally {
            busy.set(false)
        }
    }

    private suspend fun set(player: String, wanted: Boolean): Boolean {
        sentAt = nowMs()
        val r = players.setFixedOutputLevel(player, wanted)
        if (r is ApiResult.Err) log("could not set the fixed output level: ${r.error}")
        return r is ApiResult.Ok
    }

    companion object {
        private const val TAG = "FixedLevelKeeper"

        /** The one retry after a failed read or set (LMS may not know the player yet). */
        const val RETRY_MS = 2_000L

        /** The least time between sends prompted by a status that disagrees. */
        const val RESEND_EVERY_MS = 30_000L
    }
}

/** This phone's player id while it is running and connected to LMS (null otherwise). */
private fun LocalPlayerState.connectedId(): String? =
    playerId?.takeIf { running && status?.connection is PlayerConnection.Connected }
