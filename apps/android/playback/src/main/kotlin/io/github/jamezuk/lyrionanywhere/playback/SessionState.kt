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

    /** Where Android's refusals to keep playback in the foreground are noted (none: not noted). */
    val backgroundPlayback: io.github.jamezuk.lyrionanywhere.data.BackgroundPlayback? get() = null

    /** Whether the app is in the foreground (an activity started); none: not known. */
    val appForeground: kotlinx.coroutines.flow.StateFlow<Boolean>? get() = null

    /** Covers for Android Auto's lists, served by the app's own provider (none: no art). */
    val autoArt: io.github.jamezuk.lyrionanywhere.playback.auto.AutoArtCache? get() = null

    /** Where the session's caller decisions are noted for Settings › About (none: not noted). */
    val carConnections: io.github.jamezuk.lyrionanywhere.playback.auto.CarConnectionLog? get() = null
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
    /**
     * What works (offline controls): everything while the phone's player reaches LMS; with
     * the server away, only what the phone can do by itself, so the notification, the lock
     * screen and the car offer nothing that would fail.
     */
    val canPlayPause: Boolean = true,
    val canNext: Boolean = true,
    val canPrevious: Boolean = true,
    /** Jumping to another queue entry from the session's list. */
    val canJump: Boolean = true,
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
    val online = SessionState(
        playbackState = state,
        playWhenReady = lmsPlaying,
        items = items,
        currentIndex = index,
        positionMs = (status.elapsedSecs.coerceAtLeast(0.0) * 1000).toLong(),
        advancing = lmsPlaying && state == Player.STATE_READY,
        canSeek = status.canSeek,
    )
    // The server away: what the phone does, and only what it can.
    val c = s.local.offlineControls ?: return online
    val here = c.index?.toInt()?.let { i -> items.indexOfFirst { it.queueIndex == i } }?.takeIf { it >= 0 } ?: index
    return online.copy(
        playbackState = Player.STATE_READY,
        playWhenReady = c.playing,
        currentIndex = here,
        positionMs = c.elapsedMs.toLong(),
        advancing = c.playing,
        canSeek = c.seek,
        canPlayPause = c.playPause,
        canNext = c.next,
        canPrevious = c.previous,
        canJump = false,
    )
}

/** The engine is starting or playing a track (whatever LMS last said). */
val LocalPlayerState.engineActive: Boolean
    get() = running && status?.playback.let { it == LocalPlayback.BUFFERING || it == LocalPlayback.PLAYING }

/**
 * Keeps the session's playlist through a moment with none, while this phone is still a
 * player. Changing the queue from another device (LMS stops, clears the playlist, then
 * loads the new one, which can take a while on a slow library) reads for a moment as no
 * playlist at all. An empty timeline makes Media3 take the notification away and leave
 * the foreground at once, and Android doesn't let a backgrounded app go back to the
 * foreground, so with the screen off the next track would wait for the phone to wake.
 *
 * For [holdMs] the last playlist shows instead: buffering while LMS says it plays (or the
 * engine is already starting the next track), paused otherwise. Not thread-safe: used
 * from the session's looper.
 */
class SessionHoldover(private val holdMs: Long = HOLD_MS) {
    private var last: SessionState? = null
    private var emptySince: Long? = null

    /**
     * What to show for [mapped] (from [snapshot]) at [now] (a monotonic clock, ms), and how
     * long until that changes by itself (null: it won't).
     */
    fun apply(mapped: SessionState, snapshot: SessionSnapshot, now: Long): Pair<SessionState, Long?> {
        if (mapped.items.isNotEmpty()) {
            last = mapped
            emptySince = null
            return mapped to null
        }
        val held = last
        // No longer a player (switched off, stopped, signed out): nothing to keep.
        if (!snapshot.local.running || held == null) {
            forget()
            return mapped to null
        }
        val since = emptySince ?: now.also { emptySince = it }
        val left = holdMs - (now - since).coerceAtLeast(0)
        if (left <= 0) {
            forget()
            return mapped to null
        }
        val wants = snapshot.status?.mode == PlaybackMode.PLAYING || snapshot.local.engineActive
        val shown = held.copy(
            playbackState = if (wants) Player.STATE_BUFFERING else Player.STATE_READY,
            playWhenReady = wants,
            advancing = false,
        )
        return shown to left
    }

    private fun forget() {
        last = null
        emptySince = null
    }

    companion object {
        /** How long the last playlist shows while LMS has none (covers a slow load). */
        const val HOLD_MS = 90_000L
    }
}
