package io.github.jamezuk.lyrionanywhere.playback

import io.github.jamezuk.lyrionanywhere.core.ArtImage
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalPlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OutputRoute
import io.github.jamezuk.lyrionanywhere.core.Placement
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerConnection
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.StreamUpdate
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow

const val PHONE = "02:4c:41:00:00:01"

fun track(id: Long, index: Int, art: String? = "music/$id/cover") =
    Track(id, "Track $id", "Artist", "Album", 7, 3, 180.0, 1u, art, index.toUInt(), null, null, null, null, null)

fun lmsStatus(
    mode: PlaybackMode = PlaybackMode.PLAYING,
    index: Int = 1,
    elapsed: Double = 12.5,
    canSeek: Boolean = true,
    current: Track? = track(11, index),
) = PlayerStatus(
    name = "Phone",
    connected = true,
    powered = true,
    mode = mode,
    elapsedSecs = elapsed,
    durationSecs = 200.0,
    volume = 100u,
    muted = false,
    shuffle = ShuffleMode.OFF,
    repeat = RepeatMode.OFF,
    queueIndex = index.toUInt(),
    queueLength = 3u,
    canSeek = canSeek,
    current = current,
)

fun localStatus(
    playback: LocalPlayback = LocalPlayback.PLAYING,
    connection: PlayerConnection = PlayerConnection.Connected,
) = LocalPlayerStatus(connection, playback, PHONE, "Phone", 0u, 0u, 44_100u, 0u, 0u, false, null)

fun running(playback: LocalPlayback = LocalPlayback.PLAYING) =
    LocalPlayerState(enabled = true, status = localStatus(playback))

val QUEUE = listOf(track(10, 0), track(11, 1), track(12, 2))

class FakeLocal(initial: LocalPlayerState = running()) : LocalPlayerRepository {
    override val state = MutableStateFlow(initial)
    val calls = mutableListOf<String>()

    override suspend fun setEnabled(on: Boolean): ApiResult<Unit> {
        calls += "enabled $on"
        return ApiResult.Ok(Unit)
    }

    override fun setHold(hold: Boolean) {
        calls += "hold $hold"
    }

    override fun setDuck(duck: Boolean) {
        calls += "duck $duck"
    }

    override fun setOutputRoute(route: OutputRoute) {
        calls += "route ${route.device}"
    }

    override fun renegotiateOutput() {
        calls += "renegotiate"
    }
}

class FakePlayers : PlayersRepository {
    var status: PlayerStatus = lmsStatus()
    var queue: List<Track> = QUEUE
    val commands = mutableListOf<String>()
    var statusCalls = 0
    var queueCalls = 0

    private fun cmd(s: String): ApiResult<Unit> {
        commands += s
        return ApiResult.Ok(Unit)
    }

    override suspend fun list() = ApiResult.Ok(emptyList<Player>())

    override suspend fun syncGroups() = ApiResult.Ok(emptyList<io.github.jamezuk.lyrionanywhere.core.SyncGroup>())

    override suspend fun status(player: String): ApiResult<PlayerStatus> {
        statusCalls++
        return ApiResult.Ok(status)
    }

    override suspend fun queue(player: String, start: Int, count: Int): ApiResult<List<Track>> {
        queueCalls++
        return ApiResult.Ok(queue)
    }

    override suspend fun play(player: String) = cmd("play $player")

    override suspend fun setPaused(player: String, paused: Boolean) = cmd("pause $paused $player")

    override suspend fun next(player: String) = cmd("next $player")

    override suspend fun previous(player: String) = cmd("previous $player")

    override suspend fun jumpTo(player: String, index: Int) = cmd("jump $index $player")

    override suspend fun seek(player: String, seconds: Double) = cmd("seek $seconds $player")

    override suspend fun setVolume(player: String, volume: Int) = cmd("volume $volume")

    override suspend fun setShuffle(player: String, mode: ShuffleMode) = cmd("shuffle")

    override suspend fun setRepeat(player: String, mode: RepeatMode) = cmd("repeat")

    override suspend fun setPower(player: String, on: Boolean) = cmd("power $on")

    override suspend fun enqueue(player: String, selection: Selection, placement: Placement, playIndex: Int?) =
        cmd("enqueue")

    override suspend fun playFavourite(player: String, itemId: String, placement: Placement) = cmd("favourite")

    override suspend fun removeFromQueue(player: String, index: Int) = cmd("remove $index")

    override suspend fun moveInQueue(player: String, from: Int, to: Int) = cmd("move $from $to")

    override suspend fun clearQueue(player: String) = cmd("clear")

    override suspend fun restoreToQueue(player: String, trackId: Long, index: Int) = cmd("restore $trackId $index")

    override suspend fun setSynced(current: String, other: String, on: Boolean) = cmd("sync $other $on")

    override suspend fun transfer(
        source: String,
        target: String,
        onStage: (io.github.jamezuk.lyrionanywhere.core.TransferStage) -> Unit,
    ): ApiResult<io.github.jamezuk.lyrionanywhere.core.TransferOutcome> {
        commands += "transfer $source $target"
        return ApiResult.Ok(
            io.github.jamezuk.lyrionanywhere.core.TransferOutcome.Moved(io.github.jamezuk.lyrionanywhere.core.TransferMethod.GROUP),
        )
    }
}

class FakeEvents : EventsRepository {
    val flow = MutableSharedFlow<StreamUpdate>(extraBufferCapacity = 16)

    override fun updates(): Flow<StreamUpdate> = flow
}

class FakeArt : ArtRepository {
    val fetched = mutableListOf<String>()

    override suspend fun image(key: String, sizePx: Int): ApiResult<ArtImage> {
        fetched += key
        return ApiResult.Ok(ArtImage(byteArrayOf(1, 2, 3), "image/jpeg"))
    }
}

class RecordingActions : SessionActions {
    val calls = mutableListOf<String>()

    override suspend fun play() {
        calls += "play"
    }

    override suspend fun pause() {
        calls += "pause"
    }

    override suspend fun next() {
        calls += "next"
    }

    override suspend fun previous() {
        calls += "previous"
    }

    override suspend fun seekTo(seconds: Double) {
        calls += "seek $seconds"
    }

    override suspend fun jumpTo(queueIndex: Int) {
        calls += "jump $queueIndex"
    }
}

fun snapshotFlow(s: SessionSnapshot): StateFlow<SessionSnapshot> = MutableStateFlow(s)
