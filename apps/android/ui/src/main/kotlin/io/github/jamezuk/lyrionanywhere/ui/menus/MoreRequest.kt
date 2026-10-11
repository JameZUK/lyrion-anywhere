package io.github.jamezuk.lyrionanywhere.ui.menus

import io.github.jamezuk.lyrionanywhere.core.Album
import io.github.jamezuk.lyrionanywhere.core.Artist
import io.github.jamezuk.lyrionanywhere.core.FolderEntry
import io.github.jamezuk.lyrionanywhere.core.FolderKind
import io.github.jamezuk.lyrionanywhere.core.InfoKind
import io.github.jamezuk.lyrionanywhere.core.SavedPlaylist
import io.github.jamezuk.lyrionanywhere.core.Selection
import io.github.jamezuk.lyrionanywhere.core.Track

/** A request for the "More…" sheet: what it shows, LMS's menu for it, and what plays. */
data class MoreRequest(
    val target: SheetTarget,
    val kind: InfoKind,
    val id: Long,
    /** What Play now / Play next / Add to queue enqueue; null for none. */
    val selection: Selection?,
)

object More {
    fun album(a: Album) = MoreRequest(
        SheetTarget(a.title, a.artist, a.artwork),
        InfoKind.ALBUM,
        a.id,
        Selection.Album(a.id),
    )

    fun artist(a: Artist) = MoreRequest(SheetTarget(a.name), InfoKind.ARTIST, a.id, Selection.Artist(a.id))

    /** Library tracks only (radio streams have no id). */
    fun track(t: Track): MoreRequest? {
        val id = t.id ?: return null
        return MoreRequest(
            SheetTarget(t.title, listOfNotNull(t.artist, t.album).joinToString(" · ").ifEmpty { null }, t.artwork),
            InfoKind.TRACK,
            id,
            Selection.Track(id),
        )
    }

    fun genre(id: Long, name: String) = MoreRequest(SheetTarget(name), InfoKind.GENRE, id, Selection.Genre(id))

    fun year(year: UInt, title: String) =
        MoreRequest(SheetTarget(title), InfoKind.YEAR, year.toLong(), Selection.Year(year))

    fun playlist(p: SavedPlaylist) = MoreRequest(SheetTarget(p.name), InfoKind.PLAYLIST, p.id, Selection.Playlist(p.id))

    fun folder(f: FolderEntry): MoreRequest = when (f.kind) {
        FolderKind.TRACK -> MoreRequest(SheetTarget(f.name, artwork = f.artwork), InfoKind.TRACK, f.id, Selection.Track(f.id))
        else -> MoreRequest(SheetTarget(f.name), InfoKind.FOLDER, f.id, Selection.Folder(f.id))
    }
}
