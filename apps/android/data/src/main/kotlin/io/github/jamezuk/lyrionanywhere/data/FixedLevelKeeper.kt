package io.github.jamezuk.lyrionanywhere.data

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/**
 * Keeps LMS's "Output level is fixed at 100%" for this phone's player in step with the
 * Sound setting [SoundSettings.fixedOutputLevel], for as long as the app runs (the Sound
 * screen comes and goes).
 *
 * Whenever this phone's player appears in LMS (or its id changes), and whenever the setting
 * changes while it is there, the player's status is read once; if LMS disagrees with the
 * setting, the preference is set once. A failure is logged and left until the next of those
 * triggers: never retried in a loop.
 */
class FixedLevelKeeper(
    private val scope: CoroutineScope,
    /** This phone's player state (its id in LMS and the Sound settings). */
    private val local: Flow<LocalPlayerState>,
    private val players: PlayersRepository,
    private val log: (String) -> Unit = { android.util.Log.w(TAG, it) },
) {
    fun start(): Job = scope.launch {
        local.map { it.playerId to it.sound.fixedOutputLevel }
            .distinctUntilChanged()
            .collectLatest { (id, wanted) -> if (id != null) keep(id, wanted) }
    }

    /** One attempt to bring LMS in line with [wanted] for [player]. */
    internal suspend fun keep(player: String, wanted: Boolean) {
        val fixed = when (val s = players.status(player)) {
            is ApiResult.Ok -> s.value.fixedLevel
            is ApiResult.Err -> {
                log("could not read the player's status: ${s.error}")
                return
            }
        }
        if (fixed == wanted) return
        val r = players.setFixedOutputLevel(player, wanted)
        if (r is ApiResult.Err) log("could not set the fixed output level: ${r.error}")
    }

    private companion object {
        const val TAG = "FixedLevelKeeper"
    }
}
