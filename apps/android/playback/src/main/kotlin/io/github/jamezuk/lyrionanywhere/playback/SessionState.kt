package io.github.jamezuk.lyrionanywhere.playback

import androidx.media3.common.Player
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.core.Track
import io.github.jamezuk.lyrionanywhere.data.ArtRepository
import io.github.jamezuk.lyrionanywhere.data.EventsRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerRepository
import io.github.jamezuk.lyrionanywhere.data.LocalPlayerState
import io.github.jamezuk.lyrionanywhere.data.NowPlaying
import io.github.jamezuk.lyrionanywhere.data.NowPlayingHub
import io.github.jamezuk.lyrionanywhere.data.PlayersRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository

/** What the playback service needs from the app (implemented by the `Application`). */
interface PlaybackHost {
    val localPlayer: LocalPlayerRepository
    val players: PlayersRepository
    val events: EventsRepository
    val art: ArtRepository

    /** Where the widget and the Quick Settings tile learn what this phone plays (none: not shared). */
    val nowPlaying: NowPlayingHub? get() = null

    /** Downloads played from the app's own queue (none: not offered). */
    val offlinePlayer: OfflinePlayerRepository? get() = null

    /** A downloaded album's cached cover, by album reference. */
    suspend fun offlineArtwork(albumRef: String): ByteArray? = null

    /** The normal start-up ran (the repositories exist). False in safe mode, after a crash:
     *  the session then accepts no one. */
    val playbackReady: Boolean get() = true

    /** What the car's browse tree reads and plays (none: no Android Auto). */
    val autoSource: io.github.jamezuk.lyrionanywhere.playback.auto.AutoSource? get() = null

    /** Covers for Android Auto's lists, served by the app's own provider (none: no art). */
    val autoArt: io.github.jamezuk.lyrionanywhere.playback.auto.AutoArtCache? get() = null
}

/** What this phone's player plays, for the widget and tile (null until LMS has said). */
fun SessionSnapshot.nowPlaying(): NowPlaying? {
    val id = local.playerId ?: return null
    return NowPlaying.from(id, status?.name.orEmpty(), status, isPhone = true)
}

/** Cover art for the current track (compared by key, not by bytes). */
class Artwork(val key: String, val bytes: ByteArray) {
    override fun equals(other: Any?) = other is Artwork && other.key == key

    override fun hashCode() = key.hashCode()
}

/**
 * What the media session shows: LMS's view of this phone's player (LMS owns the queue,
 * SPEC D14), plus what the local engine is doing.
 */
data class SessionSnapshot(
    val local: LocalPlayerState = LocalPlayerState(),
    /** LMS's status for this phone's player. */
    val status: PlayerStatus? = null,
    /** When [status] was read (elapsed-realtime clock, ms). */
    val statusAt: Long = 0,
    /** The start of LMS's playlist. */
    val queue: List<Track> = emptyList(),
    val artwork: Artwork? = null,
)

/** One playlist entry for the session. */
data class SessionItem(
    /** Unique within the playlist: playlist index and track id. */
    val uid: String,
    /** LMS playlist index (for "jump to"). */
    val queueIndex: Int,
    val title: String,
    val artist: String?,
    val album: String?,
    val durationMs: Long?,
    val artworkKey: String?,
)

/** The session state in plain terms (mapped to Media3 by [LmsSessionPlayer]). */
data class SessionState(
    /** One of `Player.STATE_*`. */
    val playbackState: Int,
    val playWhenReady: Boolean,
    val items: List<SessionItem>,
    val currentIndex: Int,
    val positionMs: Long,
    /** The position advances with time (playing, not buffering). */
    val advancing: Boolean,
    val canSeek: Boolean,
) {
    val current: SessionItem? get() = items.getOrNull(currentIndex)
}

private fun Track.toItem(fallbackIndex: Int, durationSecs: Double?) = SessionItem(
    uid = "${queueIndex?.toInt() ?: fallbackIndex}-${id ?: 0}",
    queueIndex = queueIndex?.toInt() ?: fallbackIndex,
    title = title,
    artist = artist,
    album = album,
    durationMs = (durationSecs ?: this.durationSecs)?.takeIf { it > 0 }?.let { (it * 1000).toLong() },
    artworkKey = artwork,
)

/** Map a snapshot to the session's state. Pure: unit-tested on the JVM. */
fun mapSnapshot(s: SessionSnapshot): SessionState {
    val status = s.status
    val idle = SessionState(Player.STATE_IDLE, false, emptyList(), 0, 0, false, false)
    if (!s.local.running || status == null) return idle

    val currentIndex = status.queueIndex?.toInt()
    // The loaded part of LMS's playlist, in order; the current track always included.
    var items = s.queue.mapIndexed { i, t ->
        t.toItem(i, if (t.queueIndex?.toInt() == currentIndex) status.durationSecs else null)
    }
    val current = status.current
    if (current != null && items.none { it.queueIndex == (currentIndex ?: -1) }) {
        items = listOf(current.toItem(currentIndex ?: 0, status.durationSecs))
    }
    if (items.isEmpty()) return idle.copy(playbackState = Player.STATE_IDLE)
    val index = items.indexOfFirst { it.queueIndex == (currentIndex ?: items.first().queueIndex) }
        .coerceAtLeast(0)

    val lmsPlaying = status.mode == PlaybackMode.PLAYING
    val localPlayback = s.local.status?.playback
    val state = when {
        status.mode == PlaybackMode.STOPPED -> Player.STATE_READY
        lmsPlaying && (localPlayback == LocalPlayback.BUFFERING || localPlayback == LocalPlayback.STOPPED) ->
            Player.STATE_BUFFERING
        else -> Player.STATE_READY
    }
    return SessionState(
        playbackState = state,
        playWhenReady = lmsPlaying,
        items = items,
        currentIndex = index,
        positionMs = (status.elapsedSecs.coerceAtLeast(0.0) * 1000).toLong(),
        advancing = lmsPlaying && state == Player.STATE_READY,
        canSeek = status.canSeek,
    )
}
