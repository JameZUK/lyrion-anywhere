package io.github.jamezuk.lyrionanywhere

import android.content.Intent
import android.provider.MediaStore

/**
 * What [MainActivity] takes from the intents it is started with: a voice search ("Play …
 * on Lyrion Anywhere") and pairing links. Pure, so the rules are tested without Android.
 */
object LaunchIntents {
    /** The longest search query used (as the car's search). */
    const val MAX_QUERY = 200

    /** The actions acted on (anything else, such as the launcher's MAIN, only opens the app). */
    private val ACTED_ON = setOf(MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH, Intent.ACTION_VIEW)

    /**
     * Whether an intent with [action] and [flags] should be acted on. Not when the app was
     * reopened from Recents: Android hands back the intent it was first started with, and a
     * voice search or pairing link must not run again.
     */
    fun shouldHandle(action: String?, flags: Int): Boolean =
        action in ACTED_ON && flags and Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY == 0

    /**
     * A spoken query as plain text: bounded before anything else looks at it, without
     * control characters, trimmed, at most [MAX_QUERY] characters; null if nothing is left.
     */
    fun searchQuery(raw: String?): String? {
        if (raw == null) return null
        // Bound the work first: an extra can be as large as the binder allows.
        val text = raw.take(MAX_QUERY * 4).filter { !it.isISOControl() }.trim().take(MAX_QUERY)
        return text.ifEmpty { null }
    }
}
