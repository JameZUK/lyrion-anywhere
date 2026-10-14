package io.github.jamezuk.lyrionanywhere.playback

import android.os.Handler
import android.os.Looper
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.Player
import androidx.media3.common.SimpleBasePlayer
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import com.google.common.util.concurrent.ListenableFuture
import com.google.common.util.concurrent.SettableFuture
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.android.asCoroutineDispatcher
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** What the session's buttons do. LMS owns playback: every action is an LMS command. */
interface SessionActions {
    suspend fun play()

    suspend fun pause()

    suspend fun next()

    suspend fun previous()

    suspend fun seekTo(seconds: Double)

    suspend fun jumpTo(queueIndex: Int)
}

/**
 * The Media3 player behind the notification, lock screen, Bluetooth and headset buttons.
 * It plays nothing itself (the Rust engine does); it mirrors LMS's state for this phone's
 * player and turns button presses into LMS commands.
 */
@OptIn(UnstableApi::class)
class LmsSessionPlayer(
    looper: Looper,
    private val snapshots: StateFlow<SessionSnapshot>,
    private val actions: SessionActions,
    private val scope: CoroutineScope,
    /** What the car asks to play (none: the session plays nothing by id). */
    private val requests: PlayRequests? = null,
) : SimpleBasePlayer(looper) {
    private var snapshot = snapshots.value

    /** A play request is under way: it starts playback itself. */
    private var requesting = false
    private val main = Handler(looper).asCoroutineDispatcher()

    init {
        scope.launch(main) {
            snapshots.collect {
                snapshot = it
                invalidateState()
            }
        }
    }

    override fun getState(): State = buildState(snapshot, requests != null)

    private fun run(block: suspend () -> Unit): ListenableFuture<*> {
        val done = SettableFuture.create<Any?>()
        scope.launch(main) {
            try {
                block()
            } finally {
                // The state follows from LMS's answer, not from the command.
                done.set(null)
            }
        }
        return done
    }

    override fun handleSetPlayWhenReady(playWhenReady: Boolean): ListenableFuture<*> {
        // Media3 follows a play request with "play": the request already plays.
        val covered = playWhenReady && requesting
        return run {
            when {
                covered -> {}
                playWhenReady -> actions.play()
                else -> actions.pause()
            }
        }
    }

    override fun handlePrepare(): ListenableFuture<*> = SettableFuture.create<Any?>().apply { set(null) }

    override fun handleSetMediaItems(mediaItems: MutableList<MediaItem>, startIndex: Int, startPositionMs: Long): ListenableFuture<*> {
        val r = requests
        val item = requestedItem(mediaItems, startIndex)
        if (r == null || item == null) return SettableFuture.create<Any?>().apply { set(null) }
        requesting = true
        return run {
            try {
                r.play(item.mediaId, item.requestMetadata.searchQuery)
            } finally {
                requesting = false
            }
        }
    }

    override fun handleStop(): ListenableFuture<*> = run { actions.pause() }

    override fun handleSeek(mediaItemIndex: Int, positionMs: Long, seekCommand: Int): ListenableFuture<*> {
        val mapped = mapSnapshot(snapshot)
        return run {
            when (seekCommand) {
                Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM -> actions.next()
                Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM -> actions.previous()
                else -> {
                    val target = mapped.items.getOrNull(mediaItemIndex)
                    if (target != null && mediaItemIndex != mapped.currentIndex) {
                        actions.jumpTo(target.queueIndex)
                    } else {
                        actions.seekTo(positionMs.coerceAtLeast(0) / 1000.0)
                    }
                }
            }
        }
    }

    override fun handleRelease(): ListenableFuture<*> = SettableFuture.create<Any?>().apply { set(null) }

    companion object {
        /** The session's state for a snapshot. [playRequests]: the car may play items by id. */
        fun buildState(snapshot: SessionSnapshot, playRequests: Boolean = false): State {
            val s = mapSnapshot(snapshot)
            val commands = Player.Commands.Builder().addAll(
                Player.COMMAND_GET_CURRENT_MEDIA_ITEM,
                Player.COMMAND_GET_TIMELINE,
                Player.COMMAND_GET_METADATA,
            )
            // Only what will work (with the server away, what the phone can do itself).
            if (s.canPlayPause) commands.addAll(Player.COMMAND_PLAY_PAUSE, Player.COMMAND_STOP)
            if (s.canNext) commands.addAll(Player.COMMAND_SEEK_TO_NEXT, Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM)
            if (s.canPrevious) commands.addAll(Player.COMMAND_SEEK_TO_PREVIOUS, Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM)
            if (s.canJump) commands.add(Player.COMMAND_SEEK_TO_MEDIA_ITEM)
            if (s.canSeek) commands.add(Player.COMMAND_SEEK_IN_CURRENT_MEDIA_ITEM)
            if (playRequests) commands.addAll(Player.COMMAND_SET_MEDIA_ITEM, Player.COMMAND_PREPARE)
            val playlist = s.items.mapIndexed { i, item ->
                val meta = MediaMetadata.Builder()
                    .setTitle(item.title)
                    .setArtist(item.artist)
                    .setAlbumTitle(item.album)
                    .setIsPlayable(true)
                    .setIsBrowsable(false)
                val art = snapshot.artwork
                if (i == s.currentIndex && art != null && art.key == item.artworkKey) {
                    meta.setArtworkData(art.bytes, MediaMetadata.PICTURE_TYPE_FRONT_COVER)
                }
                MediaItemData.Builder(item.uid)
                    .setMediaItem(MediaItem.Builder().setMediaId(item.uid).setMediaMetadata(meta.build()).build())
                    .setDurationUs(item.durationMs?.let { it * 1000 } ?: androidx.media3.common.C.TIME_UNSET)
                    .setIsSeekable(s.canSeek && i == s.currentIndex)
                    .build()
            }
            val position = if (s.advancing) {
                // Extrapolate from when LMS reported it.
                val since = (android.os.SystemClock.elapsedRealtime() - snapshot.statusAt).coerceAtLeast(0)
                PositionSupplier.getExtrapolating(s.positionMs + since, 1f)
            } else {
                PositionSupplier.getConstant(s.positionMs)
            }
            return State.Builder()
                .setAvailableCommands(commands.build())
                .setPlayWhenReady(s.playWhenReady, Player.PLAY_WHEN_READY_CHANGE_REASON_REMOTE)
                .setPlaybackState(s.playbackState)
                .setPlaylist(playlist)
                .setCurrentMediaItemIndex(if (playlist.isEmpty()) 0 else s.currentIndex)
                .setContentPositionMs(position)
                .build()
        }
    }
}
