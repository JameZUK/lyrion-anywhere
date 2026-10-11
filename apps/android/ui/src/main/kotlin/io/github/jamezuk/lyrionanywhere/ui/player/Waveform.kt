package io.github.jamezuk.lyrionanywhere.ui.player

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import io.github.jamezuk.lyrionanywhere.data.Waveform
import kotlin.math.floor
import kotlin.math.pow

/** Drawing maths for the waveform seek bar (pure, so it is unit-tested). */
object WaveformMath {
    const val MIN_BARS = 16
    const val MAX_BARS = 600

    /** How many bars fit `widthPx` at `pitchPx` per bar (bar plus gap). */
    fun barCount(widthPx: Float, pitchPx: Float): Int {
        if (!widthPx.isFinite() || widthPx <= 0f || !pitchPx.isFinite() || pitchPx <= 0f) return MIN_BARS
        return floor(widthPx / pitchPx).toInt().coerceIn(MIN_BARS, MAX_BARS)
    }

    /** A bucket value (0–255) as a bar height (0–1): a gentle curve keeps quiet parts visible. */
    fun level(v: Int): Float = (v.coerceIn(0, 255) / 255f).pow(0.7f)

    /**
     * `size` buckets (read with `get`) reduced to `count` bar heights in 0–1. Each bar is its
     * span's loudest bucket, so short peaks never vanish.
     */
    fun bars(size: Int, count: Int, get: (Int) -> Int): FloatArray {
        if (size <= 0 || count <= 0) return FloatArray(0)
        return FloatArray(count) { i ->
            val a = (i.toLong() * size / count).toInt()
            val b = maxOf(a + 1, ((i + 1).toLong() * size / count).toInt()).coerceAtMost(size)
            var m = 0
            for (k in a until b) m = maxOf(m, get(k))
            level(m)
        }
    }

    fun bars(w: Waveform, count: Int): FloatArray = bars(w.size, count) { w[it] }

    /** The played fraction (0–1). */
    fun fraction(elapsed: Double, total: Double): Float {
        if (!elapsed.isFinite() || !total.isFinite() || total <= 0.0) return 0f
        return (elapsed / total).coerceIn(0.0, 1.0).toFloat()
    }
}

/** The unplayed bars' alpha over onSurfaceVariant: enough for 3:1 against the background. */
internal const val UNPLAYED_ALPHA = 0.65f

/** Bar pitch and gap, dp. */
private const val PITCH_DP = 3f
private const val GAP_DP = 1f

/**
 * The waveform drawn as the seek bar's track: the played part (up to `fraction`) in
 * `played`, the rest in `rest`. Pure decoration: the slider around it is the control.
 */
@Composable
fun WaveformTrack(
    waveform: Waveform,
    fraction: Float,
    modifier: Modifier = Modifier,
    played: Color = MaterialTheme.colorScheme.primary,
    rest: Color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = UNPLAYED_ALPHA),
) {
    val cache = remember(waveform) { HashMap<Int, FloatArray>() }
    Canvas(modifier.fillMaxWidth().height(36.dp).testTag("waveform")) {
        val pitch = PITCH_DP.dp.toPx()
        val count = WaveformMath.barCount(size.width, pitch)
        val levels = cache.getOrPut(count) { WaveformMath.bars(waveform, count) }
        val step = size.width / count
        val barW = maxOf(step * 0.4f, step - GAP_DP.dp.toPx())
        val minH = 2.dp.toPx()
        val split = fraction.coerceIn(0f, 1f) * size.width
        for (i in levels.indices) {
            val h = maxOf(minH, levels[i] * size.height).coerceAtMost(size.height)
            val x = i * step
            drawRect(
                color = if (x + barW / 2 <= split) played else rest,
                topLeft = Offset(x, (size.height - h) / 2),
                size = Size(barW, h),
            )
        }
    }
}
