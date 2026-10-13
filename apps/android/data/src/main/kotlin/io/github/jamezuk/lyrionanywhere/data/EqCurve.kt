package io.github.jamezuk.lyrionanywhere.data

import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.log10
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sqrt

/**
 * The curve of an AutoEq `ParametricEQ.txt` profile, for the profile editor's preview (Wave 2
 * §5.3). Display only: the engine's own (fuzzed) parser decides what is applied, so anything
 * this reader doesn't follow is simply left out of the picture.
 *
 * The text is untrusted (pasted or picked from a file): reading is bounded (size, lines,
 * filters), every number is checked, and nothing here throws.
 */
object EqCurve {
    enum class Type { PEAK, LOW_SHELF, HIGH_SHELF, LOW_PASS, HIGH_PASS }

    data class Filter(val type: Type, val freqHz: Double, val gainDb: Double, val q: Double)

    data class Profile(val preampDb: Double, val filters: List<Filter>)

    /** The most filters drawn, and the most lines read. */
    const val MAX_FILTERS = 64
    const val MAX_LINES = 512

    /** The plotted range, Hz, and the gain range shown, dB. */
    const val LOW_HZ = 20.0
    const val HIGH_HZ = 20_000.0
    const val RANGE_DB = 24.0

    private const val SAMPLE_RATE = 48_000.0

    /** The profile's preamp and filters (null: no usable filter, or not a profile). */
    fun parse(text: String?): Profile? {
        if (text.isNullOrEmpty() || text.length > SoundSettings.MAX_EQ_CHARS) return null
        var preamp = 0.0
        val filters = mutableListOf<Filter>()
        var lines = 0
        for (raw in text.lineSequence()) {
            if (++lines > MAX_LINES || filters.size >= MAX_FILTERS) break
            val line = raw.trim()
            if (line.length > 200) continue
            val words = line.split(' ', '\t').filter { it.isNotEmpty() }
            if (words.isEmpty()) continue
            when {
                words[0].equals("Preamp:", ignoreCase = true) ->
                    words.getOrNull(1)?.toDoubleOrNull()?.takeIf { it.isFinite() && it in -30.0..30.0 }?.let { preamp = it }
                words[0].equals("Filter", ignoreCase = true) || words[0].startsWith("Filter", ignoreCase = true) ->
                    filter(words)?.let(filters::add)
            }
        }
        if (filters.isEmpty()) return null
        return Profile(preamp, filters)
    }

    /** One `Filter n: ON PK Fc 1000 Hz Gain -3.0 dB Q 1.41` line (null: off, or unreadable). */
    private fun filter(words: List<String>): Filter? {
        val on = words.indexOfFirst { it.equals("ON", ignoreCase = true) }
        if (on < 0) return null
        val type = when (words.getOrNull(on + 1)?.uppercase()) {
            "PK", "PEQ", "PEAK" -> Type.PEAK
            "LS", "LSC", "LSQ" -> Type.LOW_SHELF
            "HS", "HSC", "HSQ" -> Type.HIGH_SHELF
            "LP", "LPQ" -> Type.LOW_PASS
            "HP", "HPQ" -> Type.HIGH_PASS
            else -> return null
        }
        fun after(key: String): Double? {
            val i = words.indexOfFirst { it.equals(key, ignoreCase = true) }
            return if (i < 0) null else words.getOrNull(i + 1)?.toDoubleOrNull()?.takeIf { it.isFinite() }
        }
        fun has(key: String) = words.any { it.equals(key, ignoreCase = true) }
        val fc = after("Fc")?.takeIf { it in 10.0..24_000.0 } ?: return null
        // A value named but unreadable refuses the filter; one left out has its default.
        val gain = if (has("Gain")) after("Gain") ?: return null else 0.0
        if (gain !in -30.0..30.0) return null
        val q = (if (has("Q")) after("Q") ?: return null else 0.707).takeIf { it in 0.05..20.0 } ?: return null
        return Filter(type, fc, gain, q)
    }

    /** [points] log-spaced frequencies over the plotted range. */
    fun frequencies(points: Int): DoubleArray {
        val n = points.coerceIn(2, 512)
        val lo = log10(LOW_HZ)
        val hi = log10(HIGH_HZ)
        return DoubleArray(n) { i -> 10.0.pow(lo + (hi - lo) * i / (n - 1)) }
    }

    /** The profile's gain (dB, clamped to ±[RANGE_DB]) at each of [frequencies] [points]. */
    fun response(profile: Profile, points: Int = 64): DoubleArray {
        val freqs = frequencies(points)
        val sum = DoubleArray(freqs.size) { profile.preampDb }
        for (f in profile.filters) {
            val c = coefficients(f) ?: continue
            for (i in freqs.indices) sum[i] += c.gainDb(freqs[i])
        }
        return DoubleArray(sum.size) { i -> if (sum[i].isFinite()) sum[i].coerceIn(-RANGE_DB, RANGE_DB) else 0.0 }
    }

    private class Biquad(val b0: Double, val b1: Double, val b2: Double, val a0: Double, val a1: Double, val a2: Double) {
        fun gainDb(freq: Double): Double {
            val w = 2 * PI * freq / SAMPLE_RATE
            val c1 = cos(w)
            val c2 = cos(2 * w)
            val num = b0 * b0 + b1 * b1 + b2 * b2 + 2 * (b0 * b1 + b1 * b2) * c1 + 2 * b0 * b2 * c2
            val den = a0 * a0 + a1 * a1 + a2 * a2 + 2 * (a0 * a1 + a1 * a2) * c1 + 2 * a0 * a2 * c2
            if (num <= 0 || den <= 0) return 0.0
            return 10 * log10(num / den)
        }
    }

    /** The filter as a biquad (Robert Bristow-Johnson's audio EQ cookbook). */
    private fun coefficients(f: Filter): Biquad? {
        val w0 = 2 * PI * f.freqHz.coerceAtMost(SAMPLE_RATE / 2 - 1) / SAMPLE_RATE
        val cw = cos(w0)
        val alpha = sin(w0) / (2 * f.q)
        val a = 10.0.pow(f.gainDb / 40)
        val b = when (f.type) {
            Type.PEAK -> Biquad(1 + alpha * a, -2 * cw, 1 - alpha * a, 1 + alpha / a, -2 * cw, 1 - alpha / a)
            Type.LOW_SHELF -> {
                val s = 2 * sqrt(a) * alpha
                Biquad(
                    a * ((a + 1) - (a - 1) * cw + s),
                    2 * a * ((a - 1) - (a + 1) * cw),
                    a * ((a + 1) - (a - 1) * cw - s),
                    (a + 1) + (a - 1) * cw + s,
                    -2 * ((a - 1) + (a + 1) * cw),
                    (a + 1) + (a - 1) * cw - s,
                )
            }
            Type.HIGH_SHELF -> {
                val s = 2 * sqrt(a) * alpha
                Biquad(
                    a * ((a + 1) + (a - 1) * cw + s),
                    -2 * a * ((a - 1) + (a + 1) * cw),
                    a * ((a + 1) + (a - 1) * cw - s),
                    (a + 1) - (a - 1) * cw + s,
                    2 * ((a - 1) - (a + 1) * cw),
                    (a + 1) - (a - 1) * cw - s,
                )
            }
            Type.LOW_PASS -> Biquad((1 - cw) / 2, 1 - cw, (1 - cw) / 2, 1 + alpha, -2 * cw, 1 - alpha)
            Type.HIGH_PASS -> Biquad((1 + cw) / 2, -(1 + cw), (1 + cw) / 2, 1 + alpha, -2 * cw, 1 - alpha)
        }
        val all = listOf(b.b0, b.b1, b.b2, b.a0, b.a1, b.a2)
        return b.takeIf { all.all { it.isFinite() } && b.a0 != 0.0 }
    }
}
