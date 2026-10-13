package io.github.jamezuk.lyrionanywhere

import android.app.SearchManager
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
import android.provider.MediaStore
import android.view.KeyEvent
import android.view.ViewGroup.LayoutParams.MATCH_PARENT
import android.view.ViewGroup.LayoutParams.WRAP_CONTENT
import android.widget.Button
import android.widget.HorizontalScrollView
import android.widget.LinearLayout
import android.widget.ScrollView
import android.widget.TextView
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import io.github.jamezuk.lyrionanywhere.navigation.AppRoot
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import io.github.jamezuk.lyrionanywhere.ui.components.ProvideHaptics
import io.github.jamezuk.lyrionanywhere.data.DevicePrefs
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.flow.map

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        val app = application as LyrionApp
        if (app.safeMode || !app.started) {
            super.onCreate(savedInstanceState)
            showCrashReport(runCatching { CrashLog.pending(this) }.getOrNull() ?: "The app didn't finish starting.")
            return
        }
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        // The volume keys (left to the system while this phone plays) set the media volume,
        // the one Now playing's volume control shows, even between tracks.
        volumeControlStream = android.media.AudioManager.STREAM_MUSIC
        setContent {
            // This device's choices (Wave 2 §5.4, §5.5): the wallpaper's colours and haptics.
            val prefs = androidx.compose.runtime.remember { app.container.settings.settings.map { it.device } }
            val device by prefs.collectAsStateWithLifecycle(DevicePrefs())
            LyrionTheme(dynamicColour = device.dynamicColour) {
                ProvideHaptics(device.haptics) {
                    AppRoot(app.container)
                }
            }
        }
        if (savedInstanceState == null) handleOnce(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        val app = application as LyrionApp
        if (!app.safeMode && app.started) handleOnce(intent)
    }

    /**
     * Act on a voice search or pairing link once: not when the app is reopened from Recents
     * (Android hands back the intent it was first started with), and not again when the
     * activity is recreated (the handled intent is replaced with a plain one).
     */
    private fun handleOnce(intent: Intent?) {
        if (intent == null || !LaunchIntents.shouldHandle(intent.action, intent.flags)) return
        playFromSearch(intent)
        openPairingLink(intent)
        setIntent(Intent(Intent.ACTION_MAIN).setPackage(packageName))
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val app = application as LyrionApp
        if (app.started && !app.safeMode && app.container.volumeKeys.onKeyDown(keyCode)) return true
        return super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean {
        val app = application as LyrionApp
        if (app.started && !app.safeMode && app.container.volumeKeys.onKeyUp(keyCode)) return true
        return super.onKeyUp(keyCode, event)
    }

    /**
     * A tapped `lyrion-anywhere://pair?…` link. Only the link's text is read (no extras), and
     * it only fills in the pairing screen: PairingLinks checks it strictly, and the user
     * confirms the server before anything is sent (or, if signed in, first agrees to sign out).
     */
    private fun openPairingLink(intent: Intent?) {
        if (intent?.action != Intent.ACTION_VIEW) return
        val link = runCatching { intent.dataString }.getOrNull() ?: return
        if (!link.startsWith("lyrion-anywhere:", ignoreCase = true)) return
        (application as LyrionApp).container.pairingLinks.offer(link)
    }

    /** "Play … on Lyrion Anywhere": only the spoken query is read (see [LyrionApp.playFromSearch]). */
    private fun playFromSearch(intent: Intent?) {
        if (intent?.action != MediaStore.INTENT_ACTION_MEDIA_PLAY_FROM_SEARCH) return
        val query = runCatching { intent.getStringExtra(SearchManager.QUERY) }.getOrNull()
        (application as LyrionApp).playFromSearch(LaunchIntents.searchQuery(query))
    }

    /**
     * The crash report, built from plain Android views (no Compose, no app theme), so it
     * shows even when the crash was in the app's own UI.
     */
    private fun showCrashReport(report: String) {
        val pad = (16 * resources.displayMetrics.density).toInt()
        val root = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setPadding(pad, pad * 2, pad, pad)
            fitsSystemWindows = true
        }
        root.addView(TextView(this).apply {
            text = getString(R.string.crash_title)
            textSize = 22f
            setTypeface(typeface, Typeface.BOLD)
        })
        root.addView(TextView(this).apply {
            text = getString(R.string.crash_body)
            setPadding(0, pad / 2, 0, pad / 2)
        })
        val buttons = LinearLayout(this).apply { orientation = LinearLayout.HORIZONTAL }
        buttons.addView(Button(this).apply {
            text = getString(R.string.crash_share)
            setOnClickListener { share(report) }
        })
        buttons.addView(Button(this).apply {
            text = getString(R.string.crash_copy)
            setOnClickListener { copy(report) }
        })
        buttons.addView(Button(this).apply {
            text = getString(R.string.crash_restart)
            setOnClickListener { restart() }
        })
        root.addView(buttons)
        val body = TextView(this).apply {
            text = report
            typeface = Typeface.MONOSPACE
            textSize = 11f
            setTextIsSelectable(true)
        }
        root.addView(
            ScrollView(this).apply { addView(HorizontalScrollView(this@MainActivity).apply { addView(body) }) },
            LinearLayout.LayoutParams(MATCH_PARENT, 0, 1f),
        )
        setContentView(root, LinearLayout.LayoutParams(MATCH_PARENT, WRAP_CONTENT))
    }

    private fun copy(text: String) {
        val clipboard = getSystemService(ClipboardManager::class.java) ?: return
        clipboard.setPrimaryClip(ClipData.newPlainText("Lyrion Anywhere crash report", text))
    }

    private fun share(text: String) {
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_SUBJECT, "Lyrion Anywhere crash report")
            putExtra(Intent.EXTRA_TEXT, text)
        }
        startActivity(Intent.createChooser(send, null))
    }

    /** Forget the crash report and start the app afresh, in a new process. */
    private fun restart() {
        CrashLog.clear(this)
        val launch = packageManager.getLaunchIntentForPackage(packageName) ?: return
        startActivity(Intent.makeRestartActivityTask(launch.component))
        Runtime.getRuntime().exit(0)
    }
}
