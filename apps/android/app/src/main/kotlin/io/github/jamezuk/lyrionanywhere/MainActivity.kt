package io.github.jamezuk.lyrionanywhere

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
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
            showCrashReport(runCatching { CrashLog.pending(this) }.getOrNull() ?: getString(R.string.crash_not_started))
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
        if (savedInstanceState == null) {
            launches.handleOnce(intent)
            // One crash (not a loop): its report, once, over the app that carried on.
            app.takeCrashReport()?.let(::showCrashOnce)
        }
    }

    private val launches by lazy { LaunchHandler(this, application as LyrionApp) }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        val app = application as LyrionApp
        if (!app.safeMode && app.started) launches.handleOnce(intent)
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

    /** A single crash's report, shown once in a dialog (the app is running normally). */
    private fun showCrashOnce(report: String) {
        val pad = (16 * resources.displayMetrics.density).toInt()
        val body = TextView(this).apply {
            text = report
            typeface = Typeface.MONOSPACE
            textSize = 11f
            setTextIsSelectable(true)
            setPadding(pad, pad / 2, pad, pad / 2)
        }
        val content = LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            addView(TextView(this@MainActivity).apply {
                text = getString(R.string.crash_once_body)
                setPadding(pad, pad / 2, pad, 0)
            })
            addView(ScrollView(this@MainActivity).apply { addView(HorizontalScrollView(this@MainActivity).apply { addView(body) }) })
        }
        android.app.AlertDialog.Builder(this)
            .setTitle(R.string.crash_once_title)
            .setView(content)
            .setPositiveButton(R.string.crash_share) { _, _ -> share(report) }
            .setNeutralButton(R.string.crash_copy) { _, _ -> copy(report) }
            .setNegativeButton(R.string.crash_once_close, null)
            .show()
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
        clipboard.setPrimaryClip(ClipData.newPlainText(getString(R.string.crash_subject), text))
    }

    private fun share(text: String) {
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_SUBJECT, getString(R.string.crash_subject))
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
