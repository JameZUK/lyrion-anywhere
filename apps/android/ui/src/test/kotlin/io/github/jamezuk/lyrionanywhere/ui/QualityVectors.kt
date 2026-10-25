package io.github.jamezuk.lyrionanywhere.ui

import io.github.jamezuk.lyrionanywhere.core.DiagramBadge
import io.github.jamezuk.lyrionanywhere.core.DiagramBlock
import io.github.jamezuk.lyrionanywhere.core.DiagramBlockKind
import io.github.jamezuk.lyrionanywhere.core.DiagramSwitch
import io.github.jamezuk.lyrionanywhere.core.DiagramVerdict
import io.github.jamezuk.lyrionanywhere.core.QualityDiagram
import io.github.jamezuk.lyrionanywhere.core.StreamQuality
import org.json.JSONArray
import org.json.JSONObject
import java.io.File

/**
 * The shared quality vectors (core/tests/fixtures/quality-vectors.json), which the Rust core,
 * the mobile crate and the web app all pass: each vector's expected diagram as the UniFFI
 * records the screens draw. The rules are the core's; these tests check the drawing.
 */
object QualityVectors {
    private val all: JSONArray by lazy { JSONArray(File("../../../core/tests/fixtures/quality-vectors.json").readText()) }

    val names: List<String> get() = (0 until all.length()).map { all.getJSONObject(it).getString("name") }

    /** The expected diagram of the vector called [name]. */
    fun expected(name: String): QualityDiagram {
        val v = (0 until all.length()).map { all.getJSONObject(it) }.firstOrNull { it.getString("name") == name }
            ?: error("no vector $name")
        return diagram(v.getJSONObject("expect"))
    }

    /** The vector's player kind ("local", "remote", "offline"). */
    fun player(name: String): String =
        (0 until all.length()).map { all.getJSONObject(it) }.first { it.getString("name") == name }
            .getJSONObject("input").getJSONObject("player").getString("kind")

    private fun level(s: String) = when (s) {
        "original" -> StreamQuality.ORIGINAL
        "high" -> StreamQuality.HIGH
        "low" -> StreamQuality.LOW
        else -> error("level $s")
    }

    private fun verdict(s: String) = when (s) {
        "hi_res" -> DiagramVerdict.HI_RES
        "lossless" -> DiagramVerdict.LOSSLESS
        "transcoded" -> DiagramVerdict.TRANSCODED
        "lossy" -> DiagramVerdict.LOSSY
        else -> error("verdict $s")
    }

    private fun kind(s: String) = when (s) {
        "file" -> DiagramBlockKind.FILE
        "lms" -> DiagramBlockKind.LMS
        "gateway" -> DiagramBlockKind.GATEWAY
        "device" -> DiagramBlockKind.DEVICE
        "download" -> DiagramBlockKind.DOWNLOAD
        "player" -> DiagramBlockKind.PLAYER
        else -> error("block $s")
    }

    fun diagram(e: JSONObject): QualityDiagram {
        val badge = e.optJSONObject("badge")?.let { b ->
            val v = verdict(b.getString("verdict"))
            DiagramBadge(v, b.getString("stream"), b.getString("text"), v == DiagramVerdict.TRANSCODED || v == DiagramVerdict.LOSSY)
        }
        val blocks = e.getJSONArray("blocks").let { a ->
            (0 until a.length()).map { i ->
                val b = a.getJSONObject(i)
                DiagramBlock(kind(b.getString("kind")), b.getString("title"), b.getString("format"), b.getString("note"), b.getBoolean("lowers"))
            }
        }
        val switch = e.optJSONObject("switch")?.let { s ->
            val levels = s.getJSONArray("levels").let { a -> (0 until a.length()).map { level(a.getString(it)) } }
            DiagramSwitch(levels, level(s.getString("in_force")), s.getString("origin_text"), s.getBoolean("overridden"), s.getBoolean("seekable"))
        }
        return QualityDiagram(badge, blocks, switch)
    }

    /** The exact case reported: a FLAC 24/96 file re-encoded to Opus 160 for High on mobile data. */
    val REPORTED = "FLAC 24/96 re-encoded, High on mobile data"
    val PASSED_THROUGH = "FLAC 24/96 passed through on Wi-Fi"
    val OVERRIDDEN = "FLAC 24/96 re-encoded, overridden here"
    val REMOTE = "Remote player, FLAC 24/96"
    val OFFLINE_OPUS = "Offline, Opus download"
    val LMS_LOWERED = "FLAC 24/96 lowered by LMS to 16/44.1"
}
