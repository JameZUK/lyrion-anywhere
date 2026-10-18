package io.github.jamezuk.lyrionanywhere

import android.app.AlertDialog
import android.app.SearchManager
import android.content.Intent
import android.provider.MediaStore
import androidx.activity.ComponentActivity
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.launch

/** What [LaunchHandler] needs from the app. */
interface LaunchHost {
    /** "Play on this phone" is on: a voice search may play here without asking. */
    suspend fun playsOnPhone(): Boolean

    /** Play the best match for [query] on this phone (null: carry on playing). */
    fun playFromSearch(query: String?)

    /** A `lyrion-anywhere:` link, for the pairing screen to check. */
    fun offerPairingLink(link: String)
}

/**
 * [MainActivity]'s launch intents: a voice search ("Play … on Lyrion Anywhere") and pairing
 * links, each acted on once (see [LaunchIntents]).
 *
 * Any app may send the voice search. It plays straight away only while "Play on this
 * phone" is on; otherwise the user is asked first, since playing here turns that setting on
 * and connects this phone's player. Nothing is changed unless they agree.
 */
class LaunchHandler(private val activity: ComponentActivity, private val host: LaunchHost) {
    /**
     * Act on a voice search or pairing link once: not when the app is reopened from Recents
     * (Android hands back the intent it was first started with), and not again when the
     * activity is recreated (the handled intent is replaced with a plain one).
     */
    fun handleOnce(intent: Intent?) {
        if (intent == null || !LaunchIntents.shouldHandle(intent.action, intent.flags)) return
        playFromSearch(intent)
        openPairingLink(intent)
        activity.intent = Intent(Intent.ACTION_MAIN).setPackage(activity.packageName)
    }

    /**
     * A tapped `lyrion-anywhere://pair?…` link. Only the link's text is read (no extras), and
     * it only fills in the pairing screen: PairingLinks checks it strictly, and the user
     * confirms the server before anything is sent (or, if signed in, first agrees to sign out).
     */
    private fun openPairingLink(intent: Intent) {
        if (intent.action != Intent.ACTION_VIEW) return
        val link = runCatching { intent.dataString }.getOrNull() ?: return
        if (!link.startsWith("lyrion-anywhere:", ignoreCase = true)) return
        host.offerPairingLink(link)
    }

    /** "Play … on Lyrion Anywhere": only the spoken query is read, bounded and cleaned. */
    private fun playFromSearch(intent: Intent) {
        if (intent.action != MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH) return
        val query = LaunchIntents.searchQuery(runCatching { intent.getStringExtra(SearchManager.QUERY) }.getOrNull())
        activity.lifecycleScope.launch {
            if (host.playsOnPhone()) host.playFromSearch(query) else ask(query)
        }
    }

    /** The one-off question: play here (and so turn "Play on this phone" on)? */
    private fun ask(query: String?) {
        if (activity.isFinishing) return
        val message = if (query == null) {
            activity.getString(R.string.voice_play_here_resume)
        } else {
            activity.getString(R.string.voice_play_here_body, query)
        }
        AlertDialog.Builder(activity)
            .setTitle(R.string.voice_play_here_title)
            .setMessage(message)
            .setPositiveButton(R.string.voice_play_here_yes) { _, _ -> host.playFromSearch(query) }
            .setNegativeButton(R.string.voice_play_here_no, null)
            .show()
    }
}
