package io.github.jamezuk.lyrionanywhere.ui.offline

import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.core.OfflineTrack
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.Player
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.RepeatMode
import io.github.jamezuk.lyrionanywhere.core.ShuffleMode
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.offline.ConnectionUi
import io.github.jamezuk.lyrionanywhere.ui.art.offlineArtKey
import io.github.jamezuk.lyrionanywhere.ui.player.PlayerUiState
import io.github.jamezuk.lyrionanywhere.ui.player.QueueActions
import io.github.jamezuk.lyrionanywhere.ui.player.QueueEntry
import io.github.jamezuk.lyrionanywhere.ui.player.TransferState

/**
 * Downloads playing on this phone, shown with the main mini player, Now playing and Queue
 * (Wave 3.5 item 14: one face, two engines). The downloads engine stays its own
 * ([io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository]); this only says
 * what it plays in the shape the main player screens read, as a player of its own.
 */
object DownloadsPlayer {
    /** The id of the downloads "player" (never an LMS player's: those are MAC addresses). */
    const val ID = "downloads"

    /**
     * The main player screens' state for the downloads queue, or null when nothing is
     * loaded. [name] names it ("Downloads"); [base] lends what isn't the player's own (this
     * phone's details, sync groups, the other players for *Play on…*).
     */
    fun state(ui: OfflinePlayerUi, name: String, base: PlayerUiState = PlayerUiState()): PlayerUiState? {
        val s = ui.state
        val status = s.status ?: return null
        val index = status.index?.toInt() ?: return null
        val order = status.order.map { it.toInt() }
        fun at(listIndex: Int): OfflineTrack? = status.keys.getOrNull(listIndex)?.let(s.tracks::get)
        val current = at(index) ?: return null
        val here = order.indexOf(index)
        val queue = order.mapNotNull { i -> at(i)?.let { QueueEntry(i.toLong(), track(it)) } }
        val next = order.getOrNull(here + 1)?.let(::at)
        val playerStatus = PlayerStatus(
            name = name,
            connected = true,
            powered = true,
            mode = when (status.playback) {
                LocalPlayback.PLAYING, LocalPlayback.BUFFERING -> PlaybackMode.PLAYING
                LocalPlayback.PAUSED -> PlaybackMode.PAUSED
                LocalPlayback.STOPPED -> PlaybackMode.STOPPED
            },
            elapsedSecs = status.elapsedMs.toLong() / 1000.0,
            durationSecs = (status.durationMs.toLong() / 1000.0).takeIf { it > 0 } ?: current.durationSecs.takeIf { it > 0 },
            volume = 0u,
            muted = false,
            shuffle = if (status.shuffle) ShuffleMode.SONGS else ShuffleMode.OFF,
            repeat = when (status.repeat) {
                LocalRepeat.OFF -> RepeatMode.OFF
                LocalRepeat.ALL -> RepeatMode.ALL
                LocalRepeat.ONE -> RepeatMode.ONE
            },
            queueIndex = here.coerceAtLeast(0).toUInt(),
            queueLength = order.size.toUInt(),
            canSeek = true,
            current = track(current),
            next = next?.let(::track),
        )
        return base.copy(
            players = listOf(Player(ID, name, null, true, true, s.playing)) + base.players,
            selectedId = ID,
            status = playerStatus,
            statusAt = ui.at,
            queue = queue,
            loading = false,
            error = null,
            waveform = null,
            transfer = TransferState.Idle,
            lyrics = null,
            lyricsTrackId = null,
            rating = null,
            ratingFailed = false,
            upNext = null,
            nowPlaying = ui.nowPlaying,
            credits = null,
            creditsTrackId = null,
            canSaveQueue = false,
            saveQueue = null,
            // The engine is on the phone: every control works, whatever the server.
            connection = ConnectionUi(),
            downloaded = emptySet(),
            onDevice = emptySet(),
            downloads = true,
            diagram = ui.diagram,
            downloadsPath = s.signalPath,
        )
    }

    /** A downloaded track as the main screens show a track (its cover from the phone). */
    fun track(t: OfflineTrack): Track = Track(
        id = t.trackId.takeIf { it > 0 },
        title = t.title,
        artist = t.artist.ifEmpty { null },
        album = t.album.ifEmpty { null },
        albumId = t.albumId,
        artistId = null,
        durationSecs = t.durationSecs.takeIf { it > 0 },
        trackNumber = t.trackNumber,
        artwork = offlineArtKey(OfflineAlbumViewModel.refOf(t)),
        queueIndex = null,
        discNumber = null,
        discCount = null,
        genre = null,
        format = t.codec.ifEmpty { null },
        year = null,
    )

    /**
     * The queue's actions for the downloads queue: a row plays from it ([QueueEntry.key] is
     * its place in the engine's list); the queue itself can't be edited.
     */
    fun queueActions(onJump: (Int) -> Unit, queue: List<QueueEntry>): QueueActions =
        QueueActions(onJump = { pos -> queue.getOrNull(pos)?.let { onJump(it.key.toInt()) } })
}
