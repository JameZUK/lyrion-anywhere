package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.DownloadFailure
import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.DownloadItem
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.OfflineAlbum
import io.github.jamezuk.lyrionanywhere.core.OfflinePlayerStatus
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ApiResult
import io.github.jamezuk.lyrionanywhere.data.AppError
import io.github.jamezuk.lyrionanywhere.data.offline.DownloadSettings
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflineState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.update

fun dlTrack(n: Int, albumId: Long? = 7, trackNumber: Int? = n) = OfflineTrack(
    "%064x".format(n), 100L + n, "Song $n", "Band", "Record", albumId, trackNumber?.toUInt(), 125.0,
    DownloadFormat.ORIGINAL, "flc", "flac", 2_000_000uL, 1_800_000_000uL, DownloadOrigin.MANUAL,
)

fun dlAlbum(ref: String = "a7", title: String = "Record", tracks: Int = 2) =
    OfflineAlbum(ref, ref.drop(1).toLongOrNull(), title, "Band", tracks.toUInt(), 4_000_000uL, false, 1_800_000_000uL, DownloadOrigin.MANUAL)

fun dlItem(id: Long, state: DownloadState, received: Long = 0, total: Long = 0, message: String = "", failure: DownloadFailure? = null) =
    DownloadItem(id, 7, "Song $id", state, received.toULong(), total.toULong(), DownloadFormat.ORIGINAL, DownloadOrigin.MANUAL, failure, message)

class FakeOffline(initial: OfflineState = OfflineState()) : OfflineRepository {
    val flow = MutableStateFlow(initial)
    override val state: StateFlow<OfflineState> = flow
    val calls = mutableListOf<String>()
    var albumResult: ApiResult<Int> = ApiResult.Ok(2)
    var smartResult: ApiResult<Unit> = ApiResult.Ok(Unit)

    override fun downloadTracks(tracks: List<Track>) {
        calls += "tracks ${tracks.mapNotNull { it.id }}"
    }

    override suspend fun downloadAlbum(albumId: Long): ApiResult<Int> {
        calls += "album $albumId"
        return albumResult
    }

    override suspend fun downloadPlaylist(playlistId: Long): ApiResult<Int> {
        calls += "playlist $playlistId"
        return albumResult
    }

    override fun pause(trackId: Long?) {
        calls += "pause $trackId"
    }

    override fun resume(trackId: Long?) {
        calls += "resume $trackId"
    }

    override fun cancel(trackId: Long?) {
        calls += "cancel $trackId"
    }

    override fun clearFinished() {
        calls += "clear"
    }

    override suspend fun remove(keys: List<String>) {
        calls += "remove $keys"
    }

    override suspend fun removeAlbum(albumRef: String) {
        calls += "removeAlbum $albumRef"
    }

    override suspend fun removeAll() {
        calls += "removeAll"
    }

    override suspend fun artwork(albumRef: String): ByteArray? = null

    override suspend fun setSettings(settings: DownloadSettings) {
        calls += "settings"
        flow.update { it.copy(settings = settings) }
    }

    override suspend fun refreshSmart(): ApiResult<Unit> {
        calls += "smart"
        return smartResult
    }

    override suspend fun signOut(removeDownloads: Boolean) {
        calls += "signOut $removeDownloads"
    }
}

fun dlStatus(
    index: Int? = 0,
    keys: List<String> = listOf(dlTrack(1).key, dlTrack(2).key),
    playback: LocalPlayback = LocalPlayback.PLAYING,
    shuffle: Boolean = false,
    repeat: LocalRepeat = LocalRepeat.OFF,
    elapsedMs: Long = 10_000,
    error: String? = null,
) = OfflinePlayerStatus(
    playback, index?.toUInt(), keys, keys.indices.map { it.toUInt() }, elapsedMs.toULong(), 125_000uL,
    shuffle, repeat, 400uL, 44_100u, 0uL, false, error,
)

class FakeOfflinePlayer(initial: OfflinePlayerState = OfflinePlayerState()) : OfflinePlayerRepository {
    val flow = MutableStateFlow(initial)
    override val state: StateFlow<OfflinePlayerState> = flow
    val calls = mutableListOf<String>()

    override fun play(tracks: List<OfflineTrack>, start: Int, shuffle: Boolean) {
        calls += "queue ${tracks.map { it.trackId }} $start $shuffle"
    }

    override fun play() {
        calls += "play"
    }

    override fun pause() {
        calls += "pause"
    }

    override fun next() {
        calls += "next"
    }

    override fun previous() {
        calls += "previous"
    }

    override fun seekTo(ms: Long) {
        calls += "seek $ms"
    }

    override fun jumpTo(index: Int) {
        calls += "jump $index"
    }

    override fun setShuffle(on: Boolean) {
        calls += "shuffle $on"
    }

    override fun setRepeat(repeat: LocalRepeat) {
        calls += "repeat $repeat"
    }

    override fun stop() {
        calls += "stop"
    }
}

val SIGNED_OUT_ERROR: AppError = AppError.SignedOut
