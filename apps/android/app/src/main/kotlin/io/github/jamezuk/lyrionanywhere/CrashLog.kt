package io.github.jamezuk.lyrionanywhere

import android.app.ActivityManager
import android.app.ApplicationExitInfo
import android.content.Context
import android.os.Build
import androidx.core.content.edit
import java.io.File

/**
 * Crash reports. A crash (an uncaught Kotlin/Java exception, or a panic in the Rust layer,
 * which writes its own file) is saved in the app's private storage. What the next start does
 * ([start]):
 * - after one crash, an ANR or a failure that didn't end the app, it starts normally and
 *   shows the report once, with Copy and Share;
 * - after repeated crashes (two within [REPEAT_WINDOW_MS], the sign of a crash at every
 *   start) it opens in a safe mode that shows only the report, with Restart, instead of
 *   starting normally and crashing again.
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
            runCatching { record(app, "Crash on thread ${thread.name}", error, fatal = true) }
            previous?.uncaughtException(thread, error)
        }
    }

    /**
     * Save a report for [error] (also used for failures that didn't end the app). A [fatal]
     * one is also counted towards safe mode where Android can't say when the app crashed
     * (Android 10).
     */
    fun record(context: Context, title: String, error: Throwable, fatal: Boolean = false) {
        val text = buildString {
            appendLine(title)
            appendLine(header(context))
            appendLine()
            append(error.stackTraceToString())
        }.take(MAX_CHARS)
        jvmFile(context).writeText(redact(text))
        if (fatal) {
            val times = decodeTimes(seen(context).getString(JVM_CRASHES, null)) + System.currentTimeMillis()
            // The process is about to end: written now, not in the background.
            seen(context).edit(commit = true) { putString(JVM_CRASHES, times.takeLast(MAX_TIMES).joinToString(",")) }
        }
    }

    /** How the app starts after the last run (see the class notes). */
    enum class StartMode {
        /** Nothing to report. */
        NORMAL,

        /** Start normally and show the report once. */
        REPORT_ONCE,

        /** Repeated crashes: start nothing and show only the report, with Restart. */
        SAFE,
    }

    /** The start decided at launch, and the report to show (null: none). */
    data class Start(val mode: StartMode, val report: String?)

    /** Two crashes this close together (the last one not yet seen) are a crash loop. */
    const val REPEAT_WINDOW_MS = 5 * 60_000L

    private const val MAX_TIMES = 8
    private const val JVM_CRASHES = "jvm_crashes"

    /** Decide how this launch starts (never throws: a failure here is a normal start). */
    fun start(context: Context): Start {
        val report = runCatching { pending(context) }.getOrNull()
        val times = runCatching { crashTimes(context) }.getOrDefault(emptyList())
        val seenUpTo = runCatching { seen(context).getLong(SEEN_EXIT, 0L) }.getOrDefault(0L)
        return Start(startMode(times, seenUpTo, report != null), report)
    }

    /**
     * Safe mode only when the latest crash hasn't been seen and another came within
     * [REPEAT_WINDOW_MS] before it (seen or not: a report shown once, then the same crash
     * again, is a loop). Otherwise a report is shown once.
     */
    internal fun startMode(crashTimes: List<Long>, seenUpTo: Long, hasReport: Boolean): StartMode {
        val latest = crashTimes.maxOrNull()
        if (latest != null && latest > seenUpTo && crashTimes.count { it in (latest - REPEAT_WINDOW_MS)..latest } >= 2) {
            return StartMode.SAFE
        }
        return if (hasReport) StartMode.REPORT_ONCE else StartMode.NORMAL
    }

    /** Exits that are crashes (an ANR, or being stopped for memory or by the user, are not). */
    internal fun isCrash(reason: Int): Boolean = reason == ApplicationExitInfo.REASON_CRASH ||
        reason == ApplicationExitInfo.REASON_CRASH_NATIVE ||
        reason == ApplicationExitInfo.REASON_INITIALIZATION_FAILURE

    /** When the app crashed lately: Android's record (11+), else the app's own (10). */
    private fun crashTimes(context: Context): List<Long> {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            val am = context.getSystemService(ActivityManager::class.java) ?: return emptyList()
            return am.getHistoricalProcessExitReasons(context.packageName, 0, MAX_TIMES)
                .filter { isCrash(it.reason) }
                .map { it.timestamp }
        }
        return decodeTimes(seen(context).getString(JVM_CRASHES, null))
    }

    private fun decodeTimes(stored: String?): List<Long> =
        stored.orEmpty().split(',').mapNotNull { it.trim().toLongOrNull() }.takeLast(MAX_TIMES)

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

    /** The report was shown (or Restart was pressed): forget it, and remember it was seen. */
    fun clear(context: Context) {
        jvmFile(context).delete()
        nativeFile(context).delete()
        // Android's record of the last exit stays; remember it has been seen.
        runCatching {
            val latest = maxOf(
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) latestExit(context)?.timestamp ?: 0L else 0L,
                decodeTimes(seen(context).getString(JVM_CRASHES, null)).maxOrNull() ?: 0L,
            )
            if (latest > 0) seen(context).edit(commit = true) { putLong(SEEN_EXIT, latest) }
        }
    }

    private const val SEEN_EXIT = "seen_exit"

    private fun seen(context: Context) = context.getSharedPreferences("crash_log", Context.MODE_PRIVATE)

    private fun latestExit(context: Context): ApplicationExitInfo? {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.R) return null
        val am = context.getSystemService(ActivityManager::class.java) ?: return null
        return am.getHistoricalProcessExitReasons(context.packageName, 0, MAX_TIMES).maxByOrNull { it.timestamp }
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
