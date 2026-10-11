package io.github.jamezuk.lyrionanywhere.playback

import android.os.Handler
import android.os.Looper
import androidx.annotation.OptIn
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import androidx.media3.common.util.UnstableApi
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.SettableFuture
import io.github.jamezuk.lyrionanywhere.core.LocalPlayback
import io.github.jamezuk.lyrionanywhere.core.LocalRepeat
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerRepository
import io.github.jamezuk.lyrionanywhere.data.offline.OfflinePlayerState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.android.asCoroutineDispatcher
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/** What the session shows while downloads play: the offline player's state, when it was
 *  read (for extrapolating the position), and the current album's cached cover. */
data class OfflineSnapshot(
    val state: OfflinePlayerState = OfflinePlayerState(),
    /** When [state] arrived (elapsed-realtime clock, ms). */
    val statusAt: Long = 0,
    val artwork: Artwork? = null,
)

/** One playlist entry: a downloaded track in play order. */
data class OfflineItem(
    val uid: String,
    /** Its index in the list the queue was made from (for "jump to"). */
    val listIndex: Int,
    val title: String,
    val artist: String,
    val album: String,
    val durationMs: Long?,
    val albumRef: String,
)

/** The session's view of the local queue, in plain terms. */
data class OfflineSessionState(
    val playbackState: Int,
    val playWhenReady: Boolean,
    val items: List<OfflineItem>,
    val currentIndex: Int,
    val positionMs: Long,
    val advancing: Boolean,
    val shuffle: Boolean,
    /** One of `Player.REPEAT_MODE_*`. */
    val repeatMode: Int,
)

fun LocalRepeat.toMedia3(): Int = when (this) {
    LocalRepeat.OFF -> Player.REPEAT_MODE_OFF
    LocalRepeat.ALL -> Player.REPEAT_MODE_ALL
    LocalRepeat.ONE -> Player.REPEAT_MODE_ONE
}

fun repeatFromMedia3(mode: Int): LocalRepeat = when (mode) {
    Player.REPEAT_MODE_ALL -> LocalRepeat.ALL
    Player.REPEAT_MODE_ONE -> LocalRepeat.ONE
    else -> LocalRepeat.OFF
}

/** Map the offline player's state to the session's. Pure: unit-tested on the JVM. */
fun mapOffline(s: OfflinePlayerState): OfflineSessionState {
    val status = s.status
    val idle = OfflineSessionState(Player.STATE_IDLE, false, emptyList(), 0, 0, false, false, Player.REPEAT_MODE_OFF)
    if (status == null || status.index == null) return idle
    val items = status.order.mapNotNull { i ->
        val key = status.keys.getOrNull(i.toInt()) ?: return@mapNotNull null
        val t = s.tracks[key] ?: return@mapNotNull null
        OfflineItem(
            uid = "${i.toInt()}-${key.take(16)}",
            listIndex = i.toInt(),
            title = t.title,
            artist = t.artist,
            album = t.album,
            durationMs = (t.durationSecs * 1000).toLong().takeIf { it > 0 },
            albumRef = if (t.albumId != null) "a${t.albumId}" else "t${t.key.take(16)}",
        )
    }
    if (items.isEmpty()) return idle
    val current = items.indexOfFirst { it.listIndex == status.index!!.toInt() }.coerceAtLeast(0)
    val state = when (status.playback) {
        LocalPlayback.BUFFERING -> Player.STATE_BUFFERING
        LocalPlayback.STOPPED -> Player.STATE_ENDED
        else -> Player.STATE_READY
    }
    val playing = status.playback == LocalPlayback.PLAYING || status.playback == LocalPlayback.BUFFERING
    return OfflineSessionState(
        playbackState = state,
        playWhenReady = playing,
        items = items,
        currentIndex = current,
        positionMs = status.elapsedMs.toLong(),
        advancing = status.playback == LocalPlayback.PLAYING,
        shuffle = status.shuffle,
        repeatMode = status.repeat.toMedia3(),
    )
}

/**
 * Keeps an [OfflineSnapshot] for the session: the offline player's state, stamped with
 * when it arrived, and the current album's cover from the download store (works offline).
 */
class OfflinePlaybackController(
    private val offline: OfflinePlayerRepository,
    private val artwork: suspend (String) -> ByteArray?,
    private val scope: CoroutineScope,
    private val clock: () -> Long,
) {
    private val _snapshot = MutableStateFlow(OfflineSnapshot())
    val snapshot: StateFlow<OfflineSnapshot> = _snapshot.asStateFlow()

    fun start() {
        scope.launch {
            offline.state.collect { s ->
                _snapshot.value = _snapshot.value.copy(state = s, statusAt = clock())
            }
        }
        scope.launch {
            offline.state.map { s -> mapOffline(s).let { it.items.getOrNull(it.currentIndex)?.albumRef } }
                .distinctUntilChanged()
                .collect { ref ->
                    val art = ref?.let { r -> artwork(r)?.let { Artwork("offline:$r", it) } }
                    _snapshot.value = _snapshot.value.copy(artwork = art)
                }
        }
    }
}

/**
 * The Media3 player behind the notification, lock screen and Bluetooth buttons while
 * downloads play. Like [LmsSessionPlayer] it plays nothing itself (the Rust engine does); its
 * buttons drive the local queue.
 */
@OptIn(UnstableApi::class)
class OfflineSessionPlayer(
    looper: Looper,
    private val snapshots: StateFlow<OfflineSnapshot>,
    private val offline: OfflinePlayerRepository,
    private val scope: CoroutineScope,
    /** What the car asks to play (none: the session plays nothing by id). */
    private val requests: PlayRequests? = null,
) : SimpleBasePlayer(looper) {
    private var snapshot = snapshots.value
    private val main = Handler(looper).asCoroutineDispatcher()

    /** A play request is under way: it starts playback itself. */
    private var requesting = false

    init {
        scope.launch(main) {
            snapshots.collect {
                snapshot = it
                invalidateState()
            }
        }
    }

    override fun getState(): State = buildState(snapshot, requests != null)

    private fun done(block: () -> Unit): ListenableFuture<*> {
        block()
        return SettableFuture.create<Any?>().apply { set(null) }
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> =
        done {
            when {
                playWhenReady && requesting -> {}
                playWhenReady -> offline.play()
                else -> offline.pause()
            }
        }

    override fun handlePrepare(): ListenableFuture<*> = done {}

    override fun handleSetMediaItems(mediaItems: MutableList<MediaItem>, startIndex: Int, startPositionMs: Long): ListenableFuture<*> {
        val r = requests
        val item = requestedItem(mediaItems, startIndex)
        if (r == null || item == null) return done {}
        requesting = true
        val f = SettableFuture.create<Any?>()
        scope.launch(main) {
            try {
                r.play(item.mediaId, item.requestMetadata.searchQuery)
            } finally {
                requesting = false
                f.set(null)
            }
        }
        return f
    }

    override fun handleStop(): ListenableFuture<*> = done { offline.pause() }

    override fun handleSetShuffleModeEnabled(shuffleModeEnabled: Boolean): ListenableFuture<*> =
        done { offline.setShuffle(shuffleModeEnabled) }

    override fun handleSetRepeatMode(repeatMode: Int): ListenableFuture<*> =
        done { offline.setRepeat(repeatFromMedia3(repeatMode)) }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        val mapped = mapOffline(snapshot.state)
        return done {
            when (seekCommand) {
                Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> offline.next()
                Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> offline.previous()
                else -> {
                    val target = mapped.items.getOrNull(mediaItemIndex)
                    if (target != null && mediaItemIndex != mapped.currentIndex) {
                        offline.jumpTo(target.listIndex)
                    } else {
                        offline.seekTo(positionMs.coerceAtLeast(0))
                    }
                }
            }
        }
    }

    override fun handleRelease(): ListenableFuture<*> = SettableFuture.create<Any?>().apply { set(null) }

    companion object {
        fun buildState(snapshot: OfflineSnapshot, playRequests: Boolean = false): State {
            val s = mapOffline(snapshot.state)
            val commands = Player.Commands.Builder().addAll(
                Player.COMMAND_PLAY_PAUSE,
                Player.COMMAND_STOP,
                Player.COMMAND_GET_CURRENT_MEDIA_ITEM,
                Player.COMMAND_GET_TIMELINE,
                Player.COMMAND_GET_METADATA,
                Player.COMMAND_SEEK_TO_NEXT,
                Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM,
                Player.COMMAND_SEEK_TO_PREVIOUS,
                Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM,
                Player.COMMAND_SEEK_TO_MEDIA_ITEM,
                Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM,
                Player.COMMAND_SET_SHUFFLE_MODE,
                Player.COMMAND_SET_REPEAT_MODE,
            ).apply { if (playRequests) addAll(Player.COMMAND_SET_MEDIA_ITEM, Player.COMMAND_PREPARE) }.build()
            val playlist = s.items.mapIndexed { i, item ->
                val meta = MediaMetadata.Builder()
                    .setTitle(item.title)
                    .setArtist(item.artist)
                    .setAlbumTitle(item.album)
                    .setIsPlayable(true)
                    .setIsBrowsable(false)
                val art = snapshot.artwork
                if (i == s.currentIndex && art != null && art.key == "offline:${item.albumRef}") {
                    meta.setArtworkData(art.bytes, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
                }
                MediaItemData.Builder(item.uid)
                    .setMediaItem(MediaItem.Builder().setMediaId(item.uid).setMediaMetadata(meta.build()).build())
                    .setDurationUs(item.durationMs?.let { it * 1000 } ?: C.TIME_UNSET)
                    .setIsSeekable(i == s.currentIndex)
                    .build()
            }
            val position = if (s.advancing) {
                val since = (android.os.SystemClock.elapsedRealtime() - snapshot.statusAt).coerceAtLeast(0)
                PositionSupplier.getExtrapolating(s.positionMs + since, 1f)
            } else {
                PositionSupplier.getConstant(s.positionMs)
            }
            return State.Builder()
                .setAvailableCommands(commands)
                .setPlayWhenReady(s.playWhenReady, Player.PLAY_WHEN_READY_CHANGE_REASON_REMOTE)
                .setPlaybackState(s.playbackState)
                .setPlaylist(playlist)
                .setCurrentMediaItemIndex(if (playlist.isEmpty()) 0 else s.currentIndex)
                .setContentPositionMs(position)
                .setShuffleModeEnabled(s.shuffle)
                .setRepeatMode(s.repeatMode)
                .build()
        }
    }
}
