package io.github.jamezuk.lyrionanywhere.playback.auto

import android.content.Context
import io.github.jamezuk.lyrionanywhere.core.BrowseNode
import io.github.jamezuk.lyrionanywhere.playback.R

/** The tree's fixed titles, from the app's resources. */
class ResourceLabels(private val context: Context) : AutoLabels {
    override fun title(node: BrowseNode): String = context.getString(
        when (node) {
            BrowseNode.Home -> R.string.auto_home
            BrowseNode.Library -> R.string.auto_library
            BrowseNode.Downloads -> R.string.auto_downloads
            BrowseNode.Resume -> R.string.auto_resume
            BrowseNode.NewMusic -> R.string.auto_new_music
            BrowseNode.Albums -> R.string.auto_albums
            BrowseNode.Artists -> R.string.auto_artists
            BrowseNode.Playlists -> R.string.auto_playlists
            BrowseNode.Favourites -> R.string.auto_favourites
            else -> R.string.auto_root
        },
    )
}
