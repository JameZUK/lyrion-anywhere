package io.github.jamezuk.lyrionanywhere

import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.content.Context
import android.os.Build
import androidx.core.content.edit
import java.io.File

/**
 * Crash reports for test builds. A crash (an uncaught Kotlin/Java exception, or a panic in
 * the Rust layer, which writes its own file) is saved in the app's private storage. On the
 * next start the app opens in a safe mode that shows the report with Copy and Share, instead
 * of starting normally and crashing again.
 *
 * Reports hold the device model, Android version, app version and the stack trace. The app
 * never puts tokens or passwords in exception messages.
 */
object CrashLog {
    private const val JVM_FILE = "crash-jvm.txt"
    private const val NATIVE_FILE = "crash-native.txt"
    private const val MAX_CHARS = 32 * 1024

    /** Where the Rust layer writes its panic report. */
    fun nativeFile(context: Context): File = File(context.filesDir, NATIVE_FILE)

    private fun jvmFile(context: Context) = File(context.filesDir, JVM_FILE)

    /** Record uncaught exceptions on any thread, then let the default handling carry on. */
    fun install(context: Context) {
        // In attachBaseContext the application context is not set yet; the base context has
        // the same private storage.
        val app = context.applicationContext ?: context
        val previous = Thread.getDefaultUncaughtExceptionHandler()
        Thread.setDefaultUncaughtExceptionHandler { thread, error ->
            runCatching { record(app, "Crash on thread ${thread.name}", error) }
            previous?.uncaughtException(thread, error)
        }
    }

    /** Save a report for [error] (also used for failures that didn't end the app). */
    fun record(context: Context, title: String, error: Throwable) {
        val text = buildString {
            appendLine(title)
            appendLine(header(context))
            appendLine()
            append(error.stackTraceToString())
        }.take(MAX_CHARS)
        jvmFile(context).writeText(text)
    }

    /** The saved report(s), or null when the last run ended normally. */
    fun pending(context: Context): String? {
        val parts = listOf(jvmFile(context), nativeFile(context))
            .filter { it.isFile }
            .mapNotNull { runCatching { it.readText().take(MAX_CHARS) }.getOrNull() }
            .filter { it.isNotBlank() }
            .toMutableList()
        runCatching { lastCrashExit(context) }.getOrNull()?.let { parts += it }
        return if (parts.isEmpty()) null else parts.joinToString("\n\n----\n\n")
    }

    fun clear(context: Context) {
        jvmFile(context).delete()
        nativeFile(context).delete()
        // Android's record of the last exit stays; remember it has been seen.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            runCatching { latestExit(context)?.let { e -> seen(context).edit { putLong(SEEN_EXIT, e.timestamp) } } }
        }
    }

    private const val SEEN_EXIT = "seen_exit"

    private fun seen(context: Context) = context.getSharedPreferences("crash_log", Context.MODE_PRIVATE)

    private fun latestExit(context: Context): ApplicationExitInfo? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        val am = context.getSystemService(ActivityManager::class.java) ?: return null
        return am.getHistoricalProcessExitReasons(context.packageName, 0, 1).firstOrNull()
    }

    /**
     * Android's own record of the last time the app died, if it crashed and hasn't been
     * shown yet (Android 11+). This works even for crashes before any app code ran, and for
     * native crashes, whose tombstone is reduced to its readable strings.
     */
    private fun lastCrashExit(context: Context): String? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        val exit = latestExit(context) ?: return null
        val kind = when (exit.reason) {
            ApplicationExitInfo.REASON_CRASH -> "Java/Kotlin crash"
            ApplicationExitInfo.REASON_CRASH_NATIVE -> "Native crash"
            ApplicationExitInfo.REASON_ANR -> "Not responding (ANR)"
            ApplicationExitInfo.REASON_INITIALIZATION_FAILURE -> "Initialisation failure"
            else -> return null
        }
        if (exit.timestamp <= seen(context).getLong(SEEN_EXIT, 0L)) return null
        val trace = runCatching {
            exit.traceInputStream?.use { it.readBytes() }
        }.getOrNull()?.let { readable(it) }.orEmpty()
        return buildString {
            appendLine("$kind (reported by Android)")
            appendLine(header(context))
            appendLine("description: ${exit.description ?: "(none)"}")
            appendLine("status: ${exit.status} · at ${java.util.Date(exit.timestamp)}")
            if (trace.isNotBlank()) {
                appendLine()
                append(trace)
            }
        }.take(MAX_CHARS)
    }

    /** The readable runs of characters in a binary trace (e.g. a native tombstone). */
    private fun readable(bytes: ByteArray): String {
        val out = StringBuilder()
        val run = StringBuilder()
        for (b in bytes.take(256 * 1024)) {
            val c = b.toInt() and 0xff
            if (c in 0x20..0x7e || c == 0x0a || c == 0x09) {
                run.append(c.toChar())
            } else {
                if (run.length >= 6) out.append(run.trim()).append('\n')
                run.setLength(0)
            }
            if (out.length > MAX_CHARS) break
        }
        if (run.length >= 6) out.append(run.trim())
        return out.toString()
    }

    private fun header(context: Context): String {
        val version = runCatching {
            context.packageManager.getPackageInfo(context.packageName, 0).versionName
        }.getOrNull() ?: "?"
        return "Lyrion Anywhere $version · Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}) · " +
            "${Build.MANUFACTURER} ${Build.MODEL}"
    }
}
