package io.github.jamezuk.lyrionanywhere

import android.app.Application
import android.content.Intent
import android.os.Bundle
import android.os.Looper
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import org.junit.Assert.assertEquals
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * MainActivity is exported (the launcher, voice search, pairing links), and Navigation reads
 * explicit deep links from the extras of the activity's intent: the screens to open and their
 * arguments (review, 12 November: "needs a test"). Navigation 2.10 follows them only from the
 * app itself (its calling package or referrer), never from another app, and never when the
 * intent names a referrer of its own. This holds the line if an upgrade ever loosens it.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class NavigationExtrasTest {
    /** Navigation's explicit deep link to "secret": the graph (id 0, no route), then the screen. */
    private fun deepLink(): Intent = Intent(Intent.ACTION_MAIN).apply {
        putExtra("android-support-nav:controller:deepLinkIds", intArrayOf(0, "android-app://androidx.navigation/secret".hashCode()))
        putExtra("android-support-nav:controller:deepLinkExtras", Bundle().apply { putString("planted", "yes") })
    }

    /** Where a NavHost opens for [intent], started by [caller] (null: another app, as Android reports it). */
    private fun opened(intent: Intent, caller: String?): String? {
        val controller = Robolectric.buildActivity(ComponentActivity::class.java, intent).create()
        val activity = controller.get()
        caller?.let { shadowOf(activity).setCallingPackage(it) }
        var nav: NavHostController? = null
        activity.setContent {
            val n = rememberNavController()
            nav = n
            NavHost(n, startDestination = "start") {
                composable("start") {}
                composable("secret") {}
            }
        }
        controller.start().resume().visible()
        shadowOf(Looper.getMainLooper()).idle()
        return nav?.currentDestination?.route
    }

    private val self: String get() = androidx.test.core.app.ApplicationProvider.getApplicationContext<Application>().packageName

    @Test
    fun anotherAppCanNotChooseTheScreen() {
        assertEquals("start", opened(deepLink(), caller = null))
        assertEquals("start", opened(deepLink(), caller = "io.example.other"))
        // An intent that names a referrer (which any sender can) is not believed, even naming the app.
        val spoofed = deepLink().putExtra(Intent.EXTRA_REFERRER_NAME, "android-app://$self")
        assertEquals("start", opened(spoofed, caller = self))
    }

    @Test
    fun theTestWouldSeeItHappen() {
        // The same intent from the app itself is followed: the ids above are right, and only
        // Navigation's check of the caller keeps the others out.
        assertEquals("secret", opened(deepLink(), caller = self))
    }
}
