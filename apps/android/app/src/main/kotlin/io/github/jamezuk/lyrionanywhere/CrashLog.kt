package io.github.jamezuk.lyrionanywhere

import android.content.Context
import android.os.Build
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
        return if (parts.isEmpty()) null else parts.joinToString("\n\n----\n\n")
    }

    fun clear(context: Context) {
        jvmFile(context).delete()
        nativeFile(context).delete()
    }

    private fun header(context: Context): String {
        val version = runCatching {
            context.packageManager.getPackageInfo(context.packageName, 0).versionName
        }.getOrNull() ?: "?"
        return "Lyrion Anywhere $version · Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}) · " +
            "${Build.MANUFACTURER} ${Build.MODEL}"
    }
}
