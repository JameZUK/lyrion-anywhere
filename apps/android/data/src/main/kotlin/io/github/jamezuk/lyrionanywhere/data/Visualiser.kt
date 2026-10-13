package io.github.jamezuk.lyrionanywhere.data

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * One frame for the visualiser (Wave 2 §4.1), from the engine's analysis of what this phone
 * plays: 32 log-spaced bands (0–1), 128 oscilloscope points (−1–1), left and right RMS and
 * peak (dBFS) and a bass level (0–1).
 */
class VisualFrame(
    val bands: FloatArray,
    val scope: FloatArray,
    val rmsDb: FloatArray,
    val peakDb: FloatArray,
    val bass: Float,
) {
    companion object {
        const val BANDS = 32
        const val SCOPE = 128
        const val FLOATS = BANDS + SCOPE + 2 + 2 + 1
        const val BYTES = FLOATS * 4
        const val FLOOR_DB = -90f

        /** A silent frame (what the meters fall back to). */
        val SILENT = VisualFrame(FloatArray(BANDS), FloatArray(SCOPE), floatArrayOf(FLOOR_DB, FLOOR_DB), floatArrayOf(FLOOR_DB, FLOOR_DB), 0f)

        /**
         * Unpack the engine's bytes (`VISUAL_FRAME_BYTES` little-endian floats, in the order
         * above). Anything but exactly that size is refused (null); values out of range or
         * not finite are clamped or zeroed, so drawing never sees junk.
         */
        fun parse(bytes: ByteArray?): VisualFrame? {
            if (bytes == null || bytes.size != BYTES) return null
            val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
            fun next(lo: Float, hi: Float, fallback: Float): Float {
                val v = b.float
                return if (v.isFinite()) v.coerceIn(lo, hi) else fallback
            }
            val bands = FloatArray(BANDS) { next(0f, 1f, 0f) }
            val scope = FloatArray(SCOPE) { next(-1f, 1f, 0f) }
            val rms = FloatArray(2) { next(FLOOR_DB, 0f, FLOOR_DB) }
            val peak = FloatArray(2) { next(FLOOR_DB, 0f, FLOOR_DB) }
            val bass = next(0f, 1f, 0f)
            return VisualFrame(bands, scope, rms, peak, bass)
        }
    }
}

/** How the visualiser draws (Wave 2 §4.1). */
enum class VisualStyle { BARS, SCOPE, GLOW, VU }

/** Now playing's layouts (Wave 2 §4.2). */
enum class NowPlayingLayout { ART, LYRICS, INFO }

/** Now playing's choices, remembered on this device. */
data class NowPlayingPrefs(
    val layout: NowPlayingLayout = NowPlayingLayout.ART,
    /** The visualiser is off unless chosen. */
    val visualiser: Boolean = false,
    val visualStyle: VisualStyle = VisualStyle.BARS,
    /** Wide windows: the queue shows as a side panel (it can be hidden). */
    val queuePanel: Boolean = true,
)
