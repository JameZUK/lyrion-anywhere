package io.github.jamezuk.lyrionanywhere

import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import io.github.jamezuk.lyrionanywhere.navigation.AppRoot
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme

class MainActivity : ComponentActivity() {
    /** Forget the crash report and start the app afresh, in a new process. */
    private fun restart() {
        CrashLog.clear(this)
        val launch = packageManager.getLaunchIntentForPackage(packageName) ?: return
        startActivity(Intent.makeRestartActivityTask(launch.component))
        Runtime.getRuntime().exit(0)
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        val app = application as LyrionApp
        if (app.safeMode || !app.started) {
            val report = CrashLog.pending(this) ?: "The app didn't finish starting."
            setContent { LyrionTheme { CrashReportScreen(report, onRestart = ::restart) } }
            return
        }
        setContent {
            LyrionTheme {
                AppRoot(app.container)
            }
        }
    }
}
