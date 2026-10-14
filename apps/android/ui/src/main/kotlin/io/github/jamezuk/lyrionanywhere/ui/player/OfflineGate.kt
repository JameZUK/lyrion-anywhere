package io.github.jamezuk.lyrionanywhere.ui.player

import io.github.jamezuk.lyrionanywhere.core.LocalAvailability
import io.github.jamezuk.lyrionanywhere.core.OfflineControls
import io.github.jamezuk.lyrionanywhere.core.PlaybackMode
import io.github.jamezuk.lyrionanywhere.core.PlayerStatus
import io.github.jamezuk.lyrionanywhere.data.offline.Reachability

/*
 * Controls that work, or honestly don't, while the server can't be reached (the offline
 * controls spec). Pure, so every rule is tested on the JVM:
 *
 * | Control                   | Another player | This phone                              |
 * |---------------------------|----------------|-----------------------------------------|
 * | Play / Pause              | off            | on: acts on the phone, position kept    |
 * | Next / Previous           | off            | on when that entry is on the phone      |
 * | A queue row (jump)        | off            | on only for rows on the phone           |
 * | Seek                      | off            | within a track the phone has whole      |
 * | Volume                    | off            | off (LMS's volume; the phone's keys work)|
 * | Shuffle, repeat, edits,   | off            | off                                     |
 * |   sleep, power, sync, play|                |                                         |
 *
 * What the phone can do is the engine's to say ([OfflineControls], from Rust's `outage`):
 * the UI only greys out what it says won't work.
 */

/** Where the transport's actions go. */
enum class ControlRoute {
    /** LMS, as always. */
    LMS,

    /** This phone's player, by itself: the server can't be reached. */
    PHONE,

    /**
     * This phone's output only (pause and play at once, by holding it): the music server is
     * busy, so LMS hears nothing for now but the link to the gateway is up.
     */
    HOLD,

    /** Nowhere: another player, with the server away. */
    NONE,
}

/** Which controls work now, and where they go. */
data class ControlGate(
    val route: ControlRoute,
    val playPause: Boolean,
    val next: Boolean,
    val previous: Boolean,
    val seek: Boolean,
    val volume: Boolean,
    /** Shuffle, repeat, queue edits, the sleep timer, power, sync and library "play" actions. */
    val other: Boolean,
    /** Stopped at the end of a track: the next one needs the server. */
    val waitingForServer: Boolean = false,
    /** Whether the phone plays now, as it says (null: as LMS says). */
    val playing: Boolean? = null,
) {
    /** Whether a queue row with [mark] can be tapped to play it. */
    fun canJump(mark: QueueMark): Boolean = when (route) {
        ControlRoute.LMS -> true
        ControlRoute.PHONE -> mark != QueueMark.NONE
        ControlRoute.HOLD, ControlRoute.NONE -> false
    }

    /** Everything works, through LMS. */
    val online: Boolean get() = route == ControlRoute.LMS

    companion object {
        val ONLINE = ControlGate(ControlRoute.LMS, playPause = true, next = true, previous = true, seek = true, volume = true, other = true)

        val NOTHING = ControlGate(ControlRoute.NONE, playPause = false, next = false, previous = false, seek = false, volume = false, other = false)
    }
}

/**
 * The controls for the selected player now: everything while [reachability] is online;
 * otherwise, for another player nothing, and for this phone ([localSelected], its player
 * running) what its engine says it can do ([controls]). With the link up but the music
 * server busy the engine has nothing to say, and only play and pause work (by holding the
 * output).
 */
fun controlGate(
    reachability: Reachability,
    localSelected: Boolean,
    localRunning: Boolean,
    controls: OfflineControls?,
): ControlGate = when {
    reachability == Reachability.ONLINE -> ControlGate.ONLINE
    !localSelected || !localRunning -> ControlGate.NOTHING
    controls != null -> ControlGate(
        route = ControlRoute.PHONE,
        playPause = controls.playPause,
        next = controls.next,
        previous = controls.previous,
        seek = controls.seek,
        volume = false,
        other = false,
        waitingForServer = controls.waitingForServer,
        playing = controls.playing,
    )
    else -> ControlGate(
        route = ControlRoute.HOLD,
        playPause = true,
        next = false,
        previous = false,
        seek = false,
        volume = false,
        other = false,
    )
}

/** A queue row's mark: where its track is on the phone. */
enum class QueueMark {
    NONE,

    /** Downloaded (shown for any player). */
    DOWNLOADED,

    /** Kept from streaming on this phone (shown only while the phone is the player). */
    ON_DEVICE,
}

/**
 * The mark for a track: downloaded (in [downloaded]) for any player; on the phone (in
 * [onDevice], from the engine) only while it is the player.
 */
fun queueMark(trackId: Long?, downloaded: Set<Long>, onDevice: Set<Long>, localSelected: Boolean): QueueMark = when {
    trackId == null || trackId <= 0 -> QueueMark.NONE
    trackId in downloaded -> QueueMark.DOWNLOADED
    localSelected && trackId in onDevice -> QueueMark.ON_DEVICE
    else -> QueueMark.NONE
}

/** The tracks the engine says are on the phone (any kind), from its answer for [ids]. */
fun onDeviceIds(ids: List<Long>, answer: List<LocalAvailability>): Set<Long> =
    ids.zip(answer).filter { (_, a) -> a != LocalAvailability.NONE }.mapTo(HashSet()) { it.first }

/**
 * LMS's last status for this phone, as the phone plays while the server is away: its own
 * play state and position, and (after a skip on the phone) the entry it plays, from the
 * queue as last read. Unchanged without [controls] (online).
 */
fun offlineStatus(status: PlayerStatus?, queue: List<QueueEntry>, controls: OfflineControls?): PlayerStatus? {
    if (status == null || controls == null) return status
    val index = controls.index?.toInt()
    val moved = index != null && index != status.queueIndex?.toInt()
    val entry = index?.let { i -> queue.firstOrNull { it.track.queueIndex?.toInt() == i } ?: queue.getOrNull(i) }
    val track = if (moved) entry?.track ?: status.current else status.current
    return status.copy(
        mode = if (controls.playing) PlaybackMode.PLAYING else PlaybackMode.PAUSED,
        elapsedSecs = controls.elapsedMs.toDouble() / 1000.0,
        queueIndex = index?.toUInt() ?: status.queueIndex,
        current = track,
        durationSecs = if (moved) track?.durationSecs else status.durationSecs,
        // "Up next" is LMS's: after a skip on the phone it isn't known.
        next = if (moved) null else status.next,
        canSeek = controls.seek,
    )
}
