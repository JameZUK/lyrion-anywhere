package io.github.jamezuk.lyrionanywhere.data

import java.text.Normalizer

/** Why a name can't be used for a saved playlist (the gateway's rules, docs/PROTOCOL.md). */
enum class PlaylistNameProblem { EMPTY, TOO_LONG, HIDDEN_CHARACTERS, RESERVED_CHARACTERS, DOTS, LEADING_DOT, TRAILING_DOT, RESERVED_NAME }

/**
 * Names for saving the queue as a playlist (LMS writes the playlist to a file of that name):
 * the rules the gateway enforces, checked here first so the dialog can say what's wrong as
 * the name is typed, and a suggested name to start from (as the web app's `save-name.ts`).
 */
object PlaylistNames {
    /** Longest name, in characters (code points). */
    const val MAX = 100

    private val reserved = Regex("""[/\\:*?"<>|]""")
    private val hidden = Regex("[\\u0000-\\u001f\\u007f-\\u009f\\u200b-\\u200f\\u2028-\\u202e\\u2060-\\u206f\\ufeff\\ufff9-\\ufffb]")
    private val device = Regex("^(con|prn|aux|nul|com[1-9]|lpt[1-9])$", RegexOption.IGNORE_CASE)

    /** The name as it will be sent: composed (NFC) and trimmed. */
    fun clean(name: String): String = Normalizer.normalize(name, Normalizer.Form.NFC).trim()

    /** Why [name] (already cleaned) can't be used, or null when it can. */
    fun problem(name: String): PlaylistNameProblem? = when {
        name.isEmpty() -> PlaylistNameProblem.EMPTY
        name.codePointCount(0, name.length) > MAX -> PlaylistNameProblem.TOO_LONG
        hidden.containsMatchIn(name) -> PlaylistNameProblem.HIDDEN_CHARACTERS
        reserved.containsMatchIn(name) -> PlaylistNameProblem.RESERVED_CHARACTERS
        ".." in name -> PlaylistNameProblem.DOTS
        name.startsWith('.') -> PlaylistNameProblem.LEADING_DOT
        name.endsWith('.') -> PlaylistNameProblem.TRAILING_DOT
        device.matches(name.substringBefore('.').trimEnd()) -> PlaylistNameProblem.RESERVED_NAME
        else -> null
    }

    /** [s] made into an acceptable name, as far as possible ("" if nothing is left). */
    private fun tidy(s: String): String {
        var out = clean(
            s.replace(hidden, "").replace(reserved, " ").replace(Regex("""\.{2,}"""), ".").replace(Regex("""\s+"""), " "),
        )
        if (out.codePointCount(0, out.length) > MAX) out = out.substring(0, out.offsetByCodePoints(0, MAX))
        out = out.trim { it == '.' || it.isWhitespace() }
        return if (problem(out) == null) out else ""
    }

    /**
     * A name to start from: the album, when the queue is one album; otherwise "Queue" and
     * the date ([date], e.g. "14 October 2026").
     */
    fun suggest(albums: List<String?>, date: String): String {
        val distinct = albums.map { it?.trim().orEmpty() }.toSet()
        if (distinct.size == 1) tidy(distinct.first()).takeIf { it.isNotEmpty() }?.let { return it }
        return tidy("Queue $date").ifEmpty { "Queue" }
    }
}
