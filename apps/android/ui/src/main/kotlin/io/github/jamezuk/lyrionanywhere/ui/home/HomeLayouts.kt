package io.github.jamezuk.lyrionanywhere.ui.home

import io.github.jamezuk.lyrionanywhere.core.HomeLayout
import io.github.jamezuk.lyrionanywhere.core.HomeShelf
import io.github.jamezuk.lyrionanywhere.core.ShelfKind

/**
 * Editing the Home layout (docs/plans/wave1-design.md §7), as the gateway checks it
 * (`gateway/src/home.rs`): a fixed set of shelf kinds, ids for stations, pinned menus and
 * genres, each shelf once, at most [MAX_SHELVES]. Pure, so the edit mode is unit-tested.
 */
object HomeLayouts {
    const val MAX_SHELVES = 24

    /** The Home every user starts with. */
    val DEFAULT = HomeLayout(
        shelves = listOf(ShelfKind.RECENT, ShelfKind.STATIONS, ShelfKind.NEW, ShelfKind.MOST, ShelfKind.ON_THIS_DAY)
            .map { HomeShelf(it, null, false) },
        isDefault = true,
    )

    /** Shelves of these kinds name something by id. */
    fun takesId(kind: ShelfKind) = kind == ShelfKind.STATION || kind == ShelfKind.MENU || kind == ShelfKind.GENRE

    /** Shelves of these kinds can be added once each, without an id. */
    val SINGLES = listOf(
        ShelfKind.RECENT,
        ShelfKind.STATIONS,
        ShelfKind.NEW,
        ShelfKind.MOST,
        ShelfKind.ON_THIS_DAY,
        ShelfKind.RANDOM,
        ShelfKind.FAVOURITES,
        ShelfKind.PLAYLISTS,
    )

    /** Move the shelf at [from] to [to] (drag, or Move up/down). Out of range: unchanged. */
    fun move(l: HomeLayout, from: Int, to: Int): HomeLayout {
        if (from !in l.shelves.indices || to !in l.shelves.indices || from == to) return l
        val list = l.shelves.toMutableList()
        list.add(to, list.removeAt(from))
        return l.copy(shelves = list, isDefault = false)
    }

    fun setHidden(l: HomeLayout, index: Int, hidden: Boolean): HomeLayout {
        if (index !in l.shelves.indices) return l
        return l.copy(
            shelves = l.shelves.mapIndexed { i, s -> if (i == index) s.copy(hidden = hidden) else s },
            isDefault = false,
        )
    }

    fun remove(l: HomeLayout, index: Int): HomeLayout {
        if (index !in l.shelves.indices) return l
        return l.copy(shelves = l.shelves.filterIndexed { i, _ -> i != index }, isDefault = false)
    }

    /** Add [shelf] at the end, or null if the gateway would refuse the result. */
    fun add(l: HomeLayout, shelf: HomeShelf): HomeLayout? {
        val next = l.copy(shelves = l.shelves + shelf.copy(hidden = false), isDefault = false)
        return if (problem(next) == null) next else null
    }

    /** Is a shelf of this kind (and id) already on Home? */
    fun has(l: HomeLayout, kind: ShelfKind, id: Long? = null) = l.shelves.any { it.kind == kind && it.id == id }

    /** What the gateway would refuse, or null. */
    fun problem(l: HomeLayout): String? {
        if (l.shelves.size > MAX_SHELVES) return "too many shelves"
        val seen = HashSet<Pair<ShelfKind, Long?>>()
        for (s in l.shelves) {
            if (takesId(s.kind) != (s.id != null)) return "a shelf is missing what it shows"
            val id = s.id
            if (id != null && id !in 1..999_999_999_999) return "bad id"
            if (!seen.add(s.kind to s.id)) return "that shelf is already on Home"
        }
        return null
    }

    /** The kinds without an id that could still be added. */
    fun addableSingles(l: HomeLayout): List<ShelfKind> = SINGLES.filterNot { has(l, it) }
}
