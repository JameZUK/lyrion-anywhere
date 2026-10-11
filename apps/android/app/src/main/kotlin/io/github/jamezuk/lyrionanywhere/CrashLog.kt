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
 * never puts tokens or passwords in exception messages, and every report is passed through
 * [redact] before it is saved or shown, since the user is invited to share it:
 * - Android's record of a native crash (its tombstone) holds memory dumps, which can hold
 *   tokens, the server's address or anything else in memory. Only the backtrace's library
 *   and function names are kept from it ([nativeSummary]);
 * - bearer tokens, the gateway's secrets (long hex runs), token fields, URLs' hosts and IP
 *   addresses, and home folders are replaced (pentest 2026-10, A7).
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
        jvmFile(context).writeText(redact(text))
    }

    /** The saved report(s), or null when the last run ended normally. */
    fun pending(context: Context): String? {
        val parts = listOf(jvmFile(context), nativeFile(context))
            .filter { it.isFile }
            .mapNotNull { runCatching { it.readText().take(MAX_CHARS) }.getOrNull() }
            .filter { it.isNotBlank() }
            .toMutableList()
        runCatching { lastCrashExit(context) }.getOrNull()?.let { parts += it }
        return report(parts)
    }

    /** The report shown and shared: the parts joined, then [redact]ed; null when empty. */
    internal fun report(parts: List<String>): String? =
        if (parts.isEmpty()) null else redact(parts.joinToString("\n\n----\n\n"))

    private val REDACTIONS: List<Pair<Regex, String>> = listOf(
        // Authorization headers, whatever the token looks like.
        Regex("""(?i)\b(bearer|basic)\s+[^\s"',;]+""") to "$1 <redacted>",
        // JSON or key=value fields holding secrets.
        Regex(""""(access_token|refresh_token|token|secret|password|code|csrf|sig)"\s*:\s*"[^"]*"""", RegexOption.IGNORE_CASE) to
            "\"$1\": \"<redacted>\"",
        Regex("""(?i)\b(access_token|refresh_token|token|secret|password|csrf|sig)=[^&\s"']+""") to "$1=<redacted>",
        // The gateway's tokens and secrets are 32 or 64 hex digits (stable track keys look
        // the same and go too).
        Regex("""\b[0-9a-fA-F]{32,}\b""") to "<hex>",
        // Where the server is.
        Regex("""(?i)\b(https?|wss?)://[^\s/"'<>]+""") to "$1://<server>",
        Regex("""\b(?:\d{1,3}\.){3}\d{1,3}\b""") to "<address>",
        Regex("""(?i)\b(?:[0-9a-f]{1,4}:){4,7}[0-9a-f]{1,4}\b""") to "<address>",
        // Build or user folders.
        Regex("""/(home|Users)/[^/\s]+/""") to "/$1/~/",
    )

    /** Remove secrets, addresses and personal folders from a report (see the class notes). */
    internal fun redact(text: String): String =
        REDACTIONS.fold(text) { acc, (pattern, replacement) -> pattern.replace(acc, replacement) }

    /**
     * What is kept of a native tombstone's readable strings: native library paths and the
     * (mangled) function names of the backtrace. Everything else, memory dumps included,
     * is dropped.
     */
    internal fun nativeSummary(readable: String): String =
        readable.lineSequence()
            .map { it.trim() }
            .filter { line ->
                line.length <= 300 && (
                    NATIVE_LIBRARY.containsMatchIn(line) ||
                        line.startsWith("_Z") || line.startsWith("_R") ||
                        line.startsWith("Abort message") || line.startsWith("signal ")
                    )
            }
            .distinct()
            .take(200)
            .joinToString("\n")

    private val NATIVE_LIBRARY = Regex("""^[\w./+=~-]*\.so\b|^/(apex|system|vendor|data/app)/[\w./+=~-]+$""")

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
        }.getOrNull()?.let { readable(it) }?.let {
            // A native tombstone holds memory dumps: keep only its backtrace's names.
            if (exit.reason == ApplicationExitInfo.REASON_CRASH_NATIVE) nativeSummary(it) else it
        }.orEmpty()
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
