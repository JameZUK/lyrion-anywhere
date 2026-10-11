package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.core.TransferOutcome
import io.github.jamezuk.lyrionanywhere.core.TransferStage
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository

/** Players: only pausing is used here. */
class FakePlayersPause(private val onPause: (String) -> Unit) : PlayersRepository {
    private fun <T> no(): ApiResult<T> = ApiResult.Err(AppError.Unexpected("not in this fake"))

    override suspend fun list() = no<List<Player>>()

    override suspend fun syncGroups() = no<List<SyncGroup>>()

    override suspend fun status(player: String) = no<PlayerStatus>()

    override suspend fun queue(player: String, start: Int, count: Int) = no<List<Track>>()

    override suspend fun play(player: String) = no<Unit>()

    override suspend fun setPaused(player: String, paused: Boolean): ApiResult<Unit> {
        if (paused) onPause(player)
        return ApiResult.Ok(Unit)
    }

    override suspend fun next(player: String) = no<Unit>()

    override suspend fun previous(player: String) = no<Unit>()

    override suspend fun jumpTo(player: String, index: Int) = no<Unit>()

    override suspend fun seek(player: String, seconds: Double) = no<Unit>()

    override suspend fun setVolume(player: String, volume: Int) = no<Unit>()

    override suspend fun setShuffle(player: String, mode: ShuffleMode) = no<Unit>()

    override suspend fun setRepeat(player: String, mode: RepeatMode) = no<Unit>()

    override suspend fun setPower(player: String, on: Boolean) = no<Unit>()

    override suspend fun enqueue(player: String, selection: Selection, placement: Placement, playIndex: Int?) = no<Unit>()

    override suspend fun playFavourite(player: String, itemId: String, placement: Placement) = no<Unit>()

    override suspend fun removeFromQueue(player: String, index: Int) = no<Unit>()

    override suspend fun moveInQueue(player: String, from: Int, to: Int) = no<Unit>()

    override suspend fun clearQueue(player: String) = no<Unit>()

    override suspend fun restoreToQueue(player: String, trackId: Long, index: Int) = no<Unit>()

    override suspend fun setSynced(current: String, other: String, on: Boolean) = no<Unit>()

    override suspend fun transfer(source: String, target: String, onStage: (TransferStage) -> Unit) = no<TransferOutcome>()
}
