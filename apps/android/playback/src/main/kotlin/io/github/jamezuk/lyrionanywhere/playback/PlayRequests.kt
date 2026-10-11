package io.github.jamezuk.lyrionanywhere.playback

import androidx.media3.common.C
import androidx.media3.common.MediaItem

/**
 * Something to play, chosen in the car (or by voice): a browse-tree id, or a spoken query
 * with no id. It always plays on this phone (SPEC D20). False: refused or failed.
 */
fun interface PlayRequests {
    suspend fun play(mediaId: String, query: String?): Boolean
}

/** The item a `setMediaItems` call asks to start with. */
fun requestedItem(items: List<MediaItem>, startIndex: Int): MediaItem? =
    items.getOrNull(if (startIndex == C.INDEX_UNSET) 0 else startIndex) ?: items.firstOrNull()
