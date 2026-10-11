package io.github.jamezuk.lyrionanywhere.ui.player

import io.github.jamezuk.lyrionanywhere.core.SyncGroup
import io.github.jamezuk.lyrionanywhere.core.Track

/** A queue entry with a key that stays the same while the list is edited (for the UI). */
data class QueueEntry(val key: Long, val track: Track)

/**
 * Queue editing, as the web app does it (`queue-edit.ts`). LMS's `playlist move <from> <to>`
 * takes the entry out at `from` and puts it back so that it ends up at `to` (in the new
 * order); [reorder] does the same to a local copy, so the list shown at once matches what
 * LMS will have once the command lands.
 */
object QueueEdit {
    private fun <T> valid(list: List<T>, i: Int) = i in list.indices

    /** A copy of [list] with the item at [from] moved so it ends up at [to]. */
    fun <T> reorder(list: List<T>, from: Int, to: Int): List<T> {
        if (!valid(list, from) || !valid(list, to) || from == to) return list.toList()
        val copy = list.toMutableList()
        val item = copy.removeAt(from)
        copy.add(to, item)
        return copy
    }

    /** A copy of [list] without the item at [index]. */
    fun <T> removeAt(list: List<T>, index: Int): List<T> =
        if (valid(list, index)) list.filterIndexed { i, _ -> i != index } else list.toList()

    /** A copy of [list] with [item] put back at [index] (clamped to the list). */
    fun <T> insertAt(list: List<T>, index: Int, item: T): List<T> {
        val copy = list.toMutableList()
        copy.add(index.coerceIn(0, copy.size), item)
        return copy
    }

    /**
     * Where a row dragged from [from] by [offsetPx] lands, in a list of [size] rows each
     * [rowPx] tall: whole rows passed, rounded to the nearest, kept within the list.
     */
    fun dropIndex(from: Int, offsetPx: Float, rowPx: Float, size: Int): Int {
        if (size <= 0 || from !in 0 until size) return from
        if (!offsetPx.isFinite() || !rowPx.isFinite() || rowPx <= 0f) return from
        val moved = Math.round(offsetPx / rowPx)
        return (from + moved).coerceIn(0, size - 1)
    }

    /** Only library tracks can be put back after a removal (radio and app items can't). */
    fun canRestore(track: Track): Boolean = (track.id ?: 0) > 0
}

/** Sync groups, as the "Play on…" sheet shows and changes them (`sync.ts`). */
object SyncChoice {
    /** Whether [other] plays in the same group as [selected]. */
    fun together(selected: String?, other: String, groups: List<SyncGroup>): Boolean {
        selected ?: return false
        if (selected == other) return false
        return groups.any { selected in it.playerIds && other in it.playerIds }
    }

    /**
     * The groups as they will be once [other] joins [selected]'s group ([on]) or leaves its
     * own: shown at once, before LMS confirms. Groups of one are dropped, as LMS lists them.
     */
    fun toggled(groups: List<SyncGroup>, selected: String, other: String, on: Boolean): List<SyncGroup> {
        if (selected == other) return groups
        val without = groups.map { SyncGroup(it.playerIds.filter { id -> id != other }) }
        val result = if (!on) {
            without
        } else if (without.any { selected in it.playerIds }) {
            without.map { if (selected in it.playerIds) SyncGroup(it.playerIds + other) else it }
        } else {
            without + SyncGroup(listOf(selected, other))
        }
        return result.filter { it.playerIds.size > 1 }
    }
}
