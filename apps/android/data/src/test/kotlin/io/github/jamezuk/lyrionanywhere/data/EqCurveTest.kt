package io.github.jamezuk.lyrionanywhere.data

import kotlin.math.abs
import kotlin.random.Random
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The profile editor's curve (Wave 2 §5.3): a bounded reader of untrusted text. */
class EqCurveTest {
    private val autoEq = """
        Preamp: -6.2 dB
        Filter 1: ON LSC Fc 105 Hz Gain 5.5 dB Q 0.70
        Filter 2: ON PK Fc 1000 Hz Gain -4.0 dB Q 1.41
        Filter 3: OFF PK Fc 3000 Hz Gain 9.0 dB Q 2.00
        Filter 4: ON HSC Fc 10000 Hz Gain -2.3 dB Q 0.70
    """.trimIndent()

    @Test
    fun readsAnAutoEqProfile() {
        val p = EqCurve.parse(autoEq)
        assertNotNull(p)
        p!!
        assertEquals(-6.2, p.preampDb, 1e-9)
        // The OFF filter is left out.
        assertEquals(listOf(EqCurve.Type.LOW_SHELF, EqCurve.Type.PEAK, EqCurve.Type.HIGH_SHELF), p.filters.map { it.type })
        assertEquals(1000.0, p.filters[1].freqHz, 0.0)
        assertEquals(-4.0, p.filters[1].gainDb, 0.0)
    }

    @Test
    fun theCurveFollowsTheFilters() {
        val peak = EqCurve.parse("Filter 1: ON PK Fc 1000 Hz Gain -6.0 dB Q 1.0")!!
        val freqs = EqCurve.frequencies(96)
        val curve = EqCurve.response(peak, 96)
        val at1k = freqs.indices.minByOrNull { abs(freqs[it] - 1000.0) }!!
        assertEquals(-6.0, curve[at1k], 0.6)
        // Far from the peak, flat.
        assertEquals(0.0, curve.first(), 0.2)
        assertEquals(0.0, curve.last(), 0.6)
        // The preamp shifts everything.
        val shifted = EqCurve.response(peak.copy(preampDb = -3.0), 96)
        assertEquals(-3.0, shifted.first(), 0.2)
        // A low shelf lifts the bass.
        val shelf = EqCurve.response(EqCurve.parse("Filter 1: ON LSC Fc 105 Hz Gain 6.0 dB Q 0.7")!!, 96)
        assertTrue(shelf.first() > 5.0)
        assertTrue(abs(shelf.last()) < 0.5)
        // Frequencies run 20 Hz to 20 kHz, rising.
        assertEquals(20.0, freqs.first(), 1e-6)
        assertEquals(20_000.0, freqs.last(), 1e-3)
        assertTrue(freqs.toList().zipWithNext().all { (a, b) -> b > a })
    }

    @Test
    fun theCurveIsClampedForDrawing() {
        val loud = (1..20).joinToString("\n") { "Filter $it: ON PK Fc 1000 Hz Gain 30 dB Q 1.0" }
        val curve = EqCurve.response(EqCurve.parse(loud)!!, 64)
        assertTrue(curve.all { it.isFinite() && it in -EqCurve.RANGE_DB..EqCurve.RANGE_DB })
    }

    @Test
    fun nonsenseIsRefusedOrSkipped() {
        for (bad in listOf(
            null,
            "",
            "Preamp: -6 dB",
            "hello",
            "Filter 1: ON XX Fc 1000 Hz Gain 1 dB Q 1",
            "Filter 1: ON PK Fc NaN Hz Gain 1 dB Q 1",
            "Filter 1: ON PK Fc 1000 Hz Gain Infinity dB Q 1",
            "Filter 1: ON PK Fc 5 Hz Gain 1 dB Q 1",
            "Filter 1: ON PK Fc 1000 Hz Gain 300 dB Q 1",
            "Filter 1: ON PK Fc 1000 Hz Gain 1 dB Q 0",
            "Filter 1: ON PK Fc 1000 Hz Gain 1 dB Q 1e9",
            "Filter 1: ON PK",
            "Filter 1: ON",
            "a".repeat(SoundSettings.MAX_EQ_CHARS + 1),
        )) {
            assertNull(bad?.take(40), EqCurve.parse(bad))
        }
        // A silly preamp is ignored, not trusted.
        assertEquals(0.0, EqCurve.parse("Preamp: 1e9 dB\nFilter 1: ON PK Fc 1000 Hz Gain 1 dB Q 1")!!.preampDb, 0.0)
    }

    @Test
    fun readingIsBounded() {
        val many = (1..1_000).joinToString("\n") { "Filter $it: ON PK Fc 1000 Hz Gain 1 dB Q 1" }
        assertEquals(EqCurve.MAX_FILTERS, EqCurve.parse(many)!!.filters.size)
        val late = "\n".repeat(EqCurve.MAX_LINES + 5) + "Filter 1: ON PK Fc 1000 Hz Gain 1 dB Q 1"
        assertNull(EqCurve.parse(late))
        assertEquals(2, EqCurve.frequencies(-5).size)
        assertEquals(512, EqCurve.frequencies(10_000).size)
    }

    /** Random text, and random edits of a real profile: never throws, always drawable. */
    @Test
    fun randomTextNeverThrows() {
        val rnd = Random(53)
        val alphabet = "Filter ONOFF PKLSCHSCLPHP Fc Hz Gain dB Q Preamp: 0123456789.-+eE\n\t:NaNInfinity"
        repeat(4_000) {
            val text = if (rnd.nextBoolean()) {
                String(CharArray(rnd.nextInt(0, 400)) { alphabet[rnd.nextInt(alphabet.length)] })
            } else {
                val chars = autoEq.toCharArray()
                repeat(rnd.nextInt(1, 12)) { chars[rnd.nextInt(chars.size)] = alphabet[rnd.nextInt(alphabet.length)] }
                String(chars)
            }
            EqCurve.parse(text)?.let { p ->
                assertTrue(p.filters.size <= EqCurve.MAX_FILTERS)
                val curve = EqCurve.response(p, 32)
                assertTrue(curve.all { it.isFinite() && abs(it) <= EqCurve.RANGE_DB })
            }
        }
    }
}
