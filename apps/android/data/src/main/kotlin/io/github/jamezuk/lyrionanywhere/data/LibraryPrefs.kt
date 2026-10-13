package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.AlbumSort

/** The Library page's type chips (Wave 2 §3.2). None chosen: the overview. */
enum class LibraryChip { ALBUMS, ARTISTS, PLAYLISTS, FAVOURITES, FILTERS, DOWNLOADED }

/**
 * An album order the Library offers (Wave 2 §3.2): one of LMS's, or [RECENTLY_PLAYED], the
 * user's own recently played albums from their listening history on the gateway. LMS's
 * `recentlyplayed` order is never used: it is the whole server's, so it would show what
 * everyone else in the household has been playing (and the gateway refuses it anyway).
 *
 * The names are what is stored; the LMS ones match the [AlbumSort] names stored before
 * "Recently played" existed, so an older saved choice still loads.
 */
enum class LibraryAlbumOrder(
    /** The LMS order to ask for; null for one that isn't LMS's. */
    val lmsSort: AlbumSort?,
) {
    NEWEST(AlbumSort.NEWEST),
    RECENTLY_PLAYED(null),
    TITLE(AlbumSort.TITLE),
    ARTIST(AlbumSort.ARTIST),
    YEAR(AlbumSort.YEAR),
    ;

    companion object {
        val DEFAULT = TITLE

        /** The orders as offered: recently added, recently played, A–Z, artist and year. */
        val OFFERED: List<LibraryAlbumOrder> = listOf(NEWEST, RECENTLY_PLAYED, TITLE, ARTIST, YEAR)

        /** At most this many recently played albums are shown (newest first), as on the web. */
        const val RECENTLY_PLAYED_MAX = 50

        /** A stored name: anything unknown (an older or newer app's, or tampered) is the default. */
        fun fromStored(name: String?): LibraryAlbumOrder = entries.firstOrNull { it.name == name } ?: DEFAULT
    }
}

/** How the Library page was left, remembered on this device: chip, album order and view. */
data class LibraryPrefs(
    val chip: LibraryChip? = null,
    val albumOrder: LibraryAlbumOrder = LibraryAlbumOrder.DEFAULT,
    /** Albums as a grid (false: a list). */
    val albumGrid: Boolean = true,
) {
    /** Only orders the Library offers are kept (random has a page of its own). */
    fun sanitised(): LibraryPrefs = if (albumOrder in LibraryAlbumOrder.OFFERED) this else copy(albumOrder = LibraryAlbumOrder.DEFAULT)
}

/**
 * Recent searches (Wave 2 §3.3): kept on this device only, never sent anywhere. Newest
 * first, at most [MAX], each once (whatever its case), as plain one-line text.
 */
object RecentSearches {
    const val MAX = 20

    /** The longest term kept (the gateway takes 200 characters of a search). */
    const val MAX_LENGTH = 200

    /** How the list is stored: terms never contain control characters, so this can't clash. */
    const val SEPARATOR = '\u001F'

    /** A term as kept: control characters made spaces, spaces collapsed, at most 200 characters. */
    fun clean(term: String): String =
        term.map { if (it.isISOControl()) ' ' else it }.joinToString("")
            .split(' ').filter { it.isNotEmpty() }.joinToString(" ")
            .take(MAX_LENGTH).trim()

    /** [term] put first (moved up if it was there already). */
    fun add(list: List<String>, term: String): List<String> {
        val t = clean(term)
        if (t.isEmpty()) return sanitise(list)
        return sanitise(listOf(t) + list.filter { !it.equals(t, ignoreCase = true) })
    }

    fun remove(list: List<String>, term: String): List<String> =
        sanitise(list.filter { !it.equals(clean(term), ignoreCase = true) })

    /** A list fit to show: clean, each once, at most [MAX]. */
    fun sanitise(list: List<String>): List<String> {
        val seen = HashSet<String>()
        return list.asSequence().map(::clean).filter { it.isNotEmpty() && seen.add(it.lowercase()) }.take(MAX).toList()
    }

    fun encode(list: List<String>): String = sanitise(list).joinToString(SEPARATOR.toString())

    /** What was stored, checked (an older, newer or tampered file shows nothing odd). */
    fun decode(stored: String?): List<String> =
        if (stored.isNullOrEmpty()) emptyList() else sanitise(stored.split(SEPARATOR).take(MAX * 4))
}
