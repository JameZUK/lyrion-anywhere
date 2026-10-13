package io.github.jamezuk.lyrionanywhere.data

import java.nio.ByteBuffer
import java.nio.ByteOrder
import kotlin.random.Random
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The visualiser's frames from the engine (Wave 2 §4.1): exact size only, values made safe. */
class VisualFrameTest {
    private fun packed(values: (Int) -> Float): ByteArray {
        val b = ByteBuffer.allocate(VisualFrame.BYTES).order(ByteOrder.LITTLE_ENDIAN)
        for (i in 0 until VisualFrame.FLOATS) b.putFloat(values(i))
        return b.array()
    }

    @Test
    fun theLayoutMatchesTheEngine() {
        // 32 bands, 128 scope points, RMS L/R, peak L/R, bass: 165 floats, 660 bytes.
        assertEquals(165, VisualFrame.FLOATS)
        assertEquals(660, VisualFrame.BYTES)
        val f = VisualFrame.parse(packed { i -> i / 1000f })!!
        assertEquals(0f, f.bands[0])
        assertEquals(0.031f, f.bands[31], 1e-6f)
        assertEquals(0.032f, f.scope[0], 1e-6f)
        assertEquals(0.159f, f.scope[127], 1e-6f)
        // Levels are dBFS (≤ 0): positive ones clamp to 0.
        assertEquals(0f, f.rmsDb[0])
        assertEquals(0.164f, f.bass, 1e-6f)
    }

    @Test
    fun anythingButTheExactSizeIsRefused() {
        assertNull(VisualFrame.parse(null))
        for (n in listOf(0, 1, 4, VisualFrame.BYTES - 1, VisualFrame.BYTES + 1, VisualFrame.BYTES * 2)) {
            assertNull("$n bytes", VisualFrame.parse(ByteArray(n)))
        }
        assertNotNull(VisualFrame.parse(ByteArray(VisualFrame.BYTES)))
    }

    @Test
    fun junkValuesAreMadeSafe() {
        val junk = listOf(Float.NaN, Float.POSITIVE_INFINITY, Float.NEGATIVE_INFINITY, 1e30f, -1e30f)
        for (j in junk) {
            val f = VisualFrame.parse(packed { j })!!
            assertTrue(f.bands.all { it in 0f..1f })
            assertTrue(f.scope.all { it in -1f..1f })
            assertTrue((f.rmsDb + f.peakDb).all { it in VisualFrame.FLOOR_DB..0f })
            assertTrue(f.bass in 0f..1f)
        }
        val nan = VisualFrame.parse(packed { Float.NaN })!!
        assertEquals(VisualFrame.FLOOR_DB, nan.rmsDb[0])
        assertEquals(0f, nan.scope[5])
    }

    @Test
    fun randomBytesNeverThrow() {
        val r = Random(7)
        repeat(2_000) {
            val n = if (r.nextInt(4) == 0) r.nextInt(0, 2_000) else VisualFrame.BYTES
            val f = VisualFrame.parse(r.nextBytes(n))
            if (f != null) {
                assertTrue((f.bands + f.scope + f.rmsDb + f.peakDb + f.bass).all { it.isFinite() })
            }
        }
    }
}
