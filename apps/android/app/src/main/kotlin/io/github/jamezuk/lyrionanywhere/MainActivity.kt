package io.github.jamezuk.lyrionanywhere

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.graphics.Typeface
import android.os.Bundle
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
        setContent {
            LyrionTheme {
                AppRoot(app.container)
            }
        }
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
