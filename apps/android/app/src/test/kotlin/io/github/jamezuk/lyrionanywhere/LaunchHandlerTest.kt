package io.github.jamezuk.lyrionanywhere

import android.app.AlertDialog
import android.app.Application
import android.app.SearchManager
import android.content.Intent
import android.net.Uri
import android.os.Looper
import android.provider.MediaStore
import androidx.activity.ComponentActivity
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowAlertDialog

/**
 * What [MainActivity] does with the intents it is started with, through [LaunchHandler] in a
 * real activity (MainActivity itself needs the native library). A voice search
 * (MEDIA_PLAY_FROM_SEARCH, which any app can send) plays on this phone straight away only
 * when "Play on this phone" is already on; otherwise the user is asked first, and nothing
 * changes unless they agree (review, 15 October).
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class LaunchHandlerTest {
    private class Host(var onPhone: Boolean) : LaunchHost {
        val played = mutableListOf<String?>()
        val links = mutableListOf<String>()

        override suspend fun playsOnPhone() = onPhone

        override fun playFromSearch(query: String?) {
            played += query
        }

        override fun offerPairingLink(link: String) {
            links += link
        }
    }

    private fun search(query: String?) = Intent(MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH).apply {
        if (query != null) putExtra(SearchManager.QUERY, query)
    }

    private fun handle(host: Host, intent: Intent): ComponentActivity {
        val activity = Robolectric.buildActivity(ComponentActivity::class.java, intent).setup().get()
        LaunchHandler(activity, host).handleOnce(activity.intent)
        shadowOf(Looper.getMainLooper()).idle()
        return activity
    }

    private fun dialog(): AlertDialog? = ShadowAlertDialog.getLatestAlertDialog()?.takeIf { it.isShowing }

    @Test
    fun alreadyPlayingOnThisPhoneAVoiceSearchPlaysAtOnce() {
        val host = Host(onPhone = true)
        handle(host, search("some album"))
        assertEquals(listOf<String?>("some album"), host.played)
        assertNull(dialog())
    }

    @Test
    fun theFirstTimeTheUserIsAskedAndNothingHappensUntilTheyAgree() {
        val host = Host(onPhone = false)
        handle(host, search("some album"))
        assertTrue("nothing plays (and no setting changes) before the user agrees", host.played.isEmpty())
        val asked = dialog()!!
        asked.getButton(AlertDialog.BUTTON_POSITIVE).performClick()
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(listOf<String?>("some album"), host.played)
    }

    @Test
    fun saidNoNothingPlays() {
        val host = Host(onPhone = false)
        handle(host, search("some album"))
        dialog()!!.getButton(AlertDialog.BUTTON_NEGATIVE).performClick()
        shadowOf(Looper.getMainLooper()).idle()
        assertTrue(host.played.isEmpty())
    }

    @Test
    fun theQueryIsBoundedAndCleanedBeforeAnythingSeesIt() {
        val host = Host(onPhone = true)
        handle(host, search("\u0000  " + "x".repeat(5_000)))
        assertEquals(LaunchIntents.MAX_QUERY, host.played.single()!!.length)
    }

    @Test
    fun reopenedFromRecentsItIsNotActedOnAgain() {
        val host = Host(onPhone = true)
        handle(host, search("some album").addFlags(Intent.FLAG_ACTIVITY_LAUNCHED_FROM_HISTORY))
        assertTrue(host.played.isEmpty())
        assertNull(dialog())
    }

    @Test
    fun aHandledIntentIsReplacedSoARecreatedActivityDoesNothing() {
        val host = Host(onPhone = true)
        val activity = handle(host, search("some album"))
        assertEquals(Intent.ACTION_MAIN, activity.intent.action)
        LaunchHandler(activity, host).handleOnce(activity.intent)
        shadowOf(Looper.getMainLooper()).idle()
        assertEquals(1, host.played.size)
    }

    @Test
    fun aPairingLinkOnlyFillsInThePairingScreen() {
        val host = Host(onPhone = false)
        val link = "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK"
        handle(host, Intent(Intent.ACTION_VIEW, Uri.parse(link)))
        assertEquals(listOf(link), host.links)
        assertTrue(host.played.isEmpty())
        // Another scheme is ignored.
        handle(host, Intent(Intent.ACTION_VIEW, Uri.parse("https://music.example.com/pair")))
        assertEquals(1, host.links.size)
    }

    @Test
    fun otherActionsOnlyOpenTheApp() {
        val host = Host(onPhone = true)
        handle(host, Intent(Intent.ACTION_MAIN))
        assertTrue(host.played.isEmpty())
        assertFalse(host.links.isNotEmpty())
    }
}
