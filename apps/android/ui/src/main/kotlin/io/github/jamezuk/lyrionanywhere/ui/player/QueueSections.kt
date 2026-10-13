package io.github.jamezuk.lyrionanywhere.ui.player

import io.github.jamezuk.lyrionanywhere.core.Track

/**
 * The tracks this device put straight after the current one with Play next (Wave 2 §3.4):
 * the "Up next" section. LMS keeps one flat list, so this is the app's own record, per
 * device and best effort: [current] is the queue index playing when it was last brought up
 * to date, and the block is the entries right after it, whose track ids are [ids] (null for
 * a stream). Anything the app can't account for drops it, and the queue shows flat.
 */
data class UpNext(val current: Int, val ids: List<Long?>) {
    /** Queue indexes of the block. */
    val range: IntRange get() = (current + 1)..(current + ids.size)
}

/** Keeping [UpNext] in step with the queue: pure, so every rule is unit-tested. */
object UpNextTracker {
    /** The most entries tracked (a bound, whatever is inserted). */
    const val MAX = 500

    private fun of(current: Int, ids: List<Long?>): UpNext? =
        if (current < 0 || ids.isEmpty()) null else UpNext(current, ids.take(MAX))

    /**
     * [added] went in right after the current track ([current]), ahead of any earlier Play
     * next tracks (LMS's `insert` puts each one there).
     */
    fun inserted(state: UpNext?, current: Int, added: List<Track>): UpNext? {
        if (added.isEmpty() || current < 0) return state
        val before = state?.takeIf { it.current == current }?.ids.orEmpty()
        return of(current, added.map { it.id } + before)
    }

    /**
     * The player moved on to [current]: the Up next tracks it has reached are played (no
     * longer up next). Going back, or past the block, ends it.
     */
    fun advanced(state: UpNext?, current: Int?): UpNext? {
        state ?: return null
        current ?: return null
        val moved = current - state.current
        return when {
            moved == 0 -> state
            moved < 0 || moved > state.ids.size -> null
            else -> of(current, state.ids.drop(moved))
        }
    }

    /**
     * The queue as read now: the block must still be the same tracks right after the
     * current one (only the entries read are compared). Otherwise it is dropped.
     */
    fun verified(state: UpNext?, current: Int?, queue: List<Track>): UpNext? {
        val s = advanced(state, current) ?: return null
        for ((i, id) in s.ids.withIndex()) {
            val at = queue.getOrNull(s.current + 1 + i) ?: break
            if (at.id != id) return null
        }
        return s
    }

    /** This device took the entry at [index] out. */
    fun removed(state: UpNext?, index: Int): UpNext? {
        state ?: return null
        return when {
            index == state.current -> null
            index < state.current -> of(state.current - 1, state.ids)
            index in state.range -> of(state.current, state.ids.filterIndexed { i, _ -> i != index - state.current - 1 })
            else -> state
        }
    }

    /**
     * A removal was undone: track [id] is back at [index]. An Up next track put back where it
     * was rejoins the block; anything else just shifts it.
     */
    fun restored(state: UpNext?, index: Int, id: Long?, wasUpNext: Boolean): UpNext? {
        state ?: return null
        val at = index - state.current - 1
        return when {
            index <= state.current -> of(state.current + 1, state.ids)
            at in 0..state.ids.size && wasUpNext -> of(state.current, state.ids.toMutableList().apply { add(at, id) })
            at in 0 until state.ids.size -> null
            else -> state
        }
    }

    /**
     * This device moved the entry at [from] so it ends up at [to] (LMS's `playlist move`).
     * An entry moved to just after the current track, or between two Up next tracks, joins
     * them; an Up next track moved away leaves them.
     */
    fun moved(state: UpNext?, from: Int, to: Int, current: Int?, movedId: Long?): UpNext? {
        if (from == to || from < 0 || to < 0) return state
        val cur = state?.current ?: current ?: return null
        if (cur < 0) return null
        val ids = state?.ids.orEmpty()
        // Tags for the entries up to the furthest one involved.
        val size = maxOf(from, to, cur + ids.size) + 1
        val tags = MutableList<Tag>(size) { Tag.Other }
        tags[cur] = Tag.Current
        ids.forEachIndexed { i, id -> tags[cur + 1 + i] = Tag.Up(id) }
        val item = tags.removeAt(from)
        tags.add(to, item)
        if (item !is Tag.Current) {
            val prev = tags.getOrNull(to - 1)
            val next = tags.getOrNull(to + 1)
            val joins = prev == Tag.Current || (prev is Tag.Up && next is Tag.Up)
            tags[to] = if (joins) Tag.Up(if (item is Tag.Up) item.id else movedId) else Tag.Other
        }
        // The block: the Up next tags straight after the current track.
        val newCurrent = tags.indexOf(Tag.Current)
        val block = tags.drop(newCurrent + 1).takeWhile { it is Tag.Up }.map { (it as Tag.Up).id }
        return of(newCurrent, block)
    }

    private sealed interface Tag {
        data object Current : Tag

        data object Other : Tag

        data class Up(val id: Long?) : Tag
    }
}

/** The queue's sections as shown: Now playing, Up next and Then, or flat. */
data class QueueSections(
    /** Index of the playing entry (the "Now playing" section). */
    val current: Int,
    /** Indexes of the Up next entries. */
    val upNext: IntRange,
) {
    /** Where "Then" starts. */
    val thenFrom: Int get() = upNext.last + 1

    companion object {
        /** Sections for a queue of [size] entries read, or null for a flat queue. */
        fun of(size: Int, current: Int?, upNext: UpNext?): QueueSections? {
            if (current == null || upNext == null || upNext.current != current) return null
            if (current !in 0 until size) return null
            val last = minOf(upNext.range.last, size - 1)
            if (last < upNext.range.first) return null
            return QueueSections(current, upNext.range.first..last)
        }
    }
}

/** Gapless markers (Wave 2 §3.5): presentation only, no promise about the player. */
object Gapless {
    /**
     * Whether [b] follows on from [a]: the same album (and disc, where known) and the next
     * track number.
     */
    fun follows(a: Track, b: Track): Boolean {
        val album = a.albumId ?: return false
        if (b.albumId != album) return false
        val ta = a.trackNumber ?: return false
        val tb = b.trackNumber ?: return false
        if (tb == UInt.MAX_VALUE || ta == UInt.MAX_VALUE) return false
        val da = a.discNumber
        val db = b.discNumber
        if (da != null && db != null && da != db) return false
        return tb == ta + 1u
    }
}
