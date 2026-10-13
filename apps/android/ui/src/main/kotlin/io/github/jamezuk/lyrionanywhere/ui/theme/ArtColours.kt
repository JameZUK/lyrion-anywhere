package io.github.jamezuk.lyrionanywhere.ui.theme

import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow

/**
 * Album-art colour, in plain Kotlin (no Android types, so it is tested on the JVM):
 * - [ArtPalette.seed] picks a cover's characteristic colour with a small quantiser;
 * - [ArtScheme.from] turns it into the colours a page is tinted with, each foreground
 *   adjusted until it meets WCAG contrast on its background (4.5:1 for text, 3:1 for
 *   accents), whatever the cover.
 *
 * Colours are ARGB [Int]s.
 */
object ArtPalette {
    /** Pixels sampled at most (covers are scaled down before this anyway). */
    private const val MAX_PIXELS = 64 * 64 * 4

    /**
     * The cover's characteristic colour: the most prominent fairly colourful one, or the
     * most common colour if the cover is grey. Null for an empty or fully transparent image.
     */
    fun seed(pixels: IntArray): Int? {
        // 4 bits a channel: 4096 buckets, each keeping its pixels' sums for the average.
        val count = IntArray(4096)
        val sumR = LongArray(4096)
        val sumG = LongArray(4096)
        val sumB = LongArray(4096)
        val step = max(1, pixels.size / MAX_PIXELS)
        var seen = 0
        var i = 0
        while (i < pixels.size) {
            val p = pixels[i]
            i += step
            if ((p ushr 24) < 128) continue // transparent
            val r = (p shr 16) and 0xff
            val g = (p shr 8) and 0xff
            val b = p and 0xff
            val k = ((r shr 4) shl 8) or ((g shr 4) shl 4) or (b shr 4)
            count[k]++
            sumR[k] += r.toLong()
            sumG[k] += g.toLong()
            sumB[k] += b.toLong()
            seen++
        }
        if (seen == 0) return null
        var best = -1
        var bestScore = 0.0
        var common = -1
        for (k in 0 until 4096) {
            val n = count[k]
            if (n == 0) continue
            if (common < 0 || n > count[common]) common = k
            val c = average(k, count, sumR, sumG, sumB)
            val hsl = Hsl.of(c)
            // Skip near-black and near-white; prefer colour, weighted by how much there is.
            if (hsl.l < 0.12 || hsl.l > 0.92 || hsl.s < 0.2) continue
            val score = n * (0.25 + hsl.s) * (1.0 - abs(hsl.l - 0.5))
            if (score > bestScore) {
                bestScore = score
                best = k
            }
        }
        // A colourful accent must be more than a speck to count.
        val k = if (best >= 0 && count[best] * 200 >= seen) best else common
        return average(k, count, sumR, sumG, sumB)
    }

    /** A colour counts for Glow's halo only this saturated, and within this lightness. */
    const val HALO_MIN_SATURATION = 0.25
    const val HALO_MIN_LIGHTNESS = 0.12
    const val HALO_MAX_LIGHTNESS = 0.9

    /**
     * A halo colour must cover at least this share of the colourful pixels; and unless this
     * share of the whole image is colourful, there are none (the theme's accent is used).
     */
    const val HALO_MIN_SHARE = 0.05

    /**
     * Up to [k] dominant, sufficiently saturated colours of a cover (Glow's halo), most
     * prominent first: a small k-means over the colourful pixels (grey, near-black and
     * near-white ones are left out). Empty for a grey, empty or transparent cover (the caller
     * falls back to the theme's accent). Deterministic: the same pixels give the same colours.
     */
    fun dominant(pixels: IntArray, k: Int = 3): List<Int> {
        val want = k.coerceIn(1, 8)
        val step = max(1, pixels.size / MAX_PIXELS)
        // The colourful pixels, as RGB floats.
        val pts = ArrayList<FloatArray>()
        var i = 0
        var sampled = 0
        while (i < pixels.size) {
            val p = pixels[i]
            i += step
            sampled++
            if ((p ushr 24) < 128) continue
            val hsl = Hsl.of(p)
            if (hsl.s < HALO_MIN_SATURATION || hsl.l < HALO_MIN_LIGHTNESS || hsl.l > HALO_MAX_LIGHTNESS) continue
            pts += floatArrayOf(((p shr 16) and 0xff).toFloat(), ((p shr 8) and 0xff).toFloat(), (p and 0xff).toFloat())
        }
        if (pts.isEmpty() || pts.size < sampled * HALO_MIN_SHARE) return emptyList()
        // Seeds: the first colourful pixel, then each time the pixel farthest from every seed.
        val centres = ArrayList<FloatArray>()
        centres += pts[0].copyOf()
        while (centres.size < want) {
            var far = -1
            var farD = 0f
            for ((j, q) in pts.withIndex()) {
                val d = centres.minOf { dist2(it, q) }
                if (d > farD) {
                    farD = d
                    far = j
                }
            }
            // Nothing left that differs: fewer colours.
            if (far < 0 || farD < MIN_SEED_DIST2) break
            centres += pts[far].copyOf()
        }
        val assign = IntArray(pts.size)
        repeat(KMEANS_ROUNDS) {
            for ((j, q) in pts.withIndex()) {
                var best = 0
                var bestD = Float.MAX_VALUE
                for ((c, centre) in centres.withIndex()) {
                    val d = dist2(centre, q)
                    if (d < bestD) {
                        bestD = d
                        best = c
                    }
                }
                assign[j] = best
            }
            val sums = Array(centres.size) { FloatArray(3) }
            val counts = IntArray(centres.size)
            for ((j, q) in pts.withIndex()) {
                val c = assign[j]
                sums[c][0] += q[0]
                sums[c][1] += q[1]
                sums[c][2] += q[2]
                counts[c]++
            }
            for (c in centres.indices) {
                if (counts[c] > 0) centres[c] = floatArrayOf(sums[c][0] / counts[c], sums[c][1] / counts[c], sums[c][2] / counts[c])
            }
        }
        val counts = IntArray(centres.size)
        assign.forEach { counts[it]++ }
        return centres.indices
            .filter { counts[it] >= pts.size * HALO_MIN_SHARE }
            .sortedByDescending { counts[it] }
            .map { c -> argb(kotlin.math.round(centres[c][0]).toInt(), kotlin.math.round(centres[c][1]).toInt(), kotlin.math.round(centres[c][2]).toInt()) }
            .filter { Hsl.of(it).s >= HALO_MIN_SATURATION }
    }

    private const val KMEANS_ROUNDS = 10

    /** Seeds closer than this (squared RGB distance, about 40 apart) are one colour. */
    private const val MIN_SEED_DIST2 = 1_600f

    private fun dist2(a: FloatArray, b: FloatArray): Float {
        val dr = a[0] - b[0]
        val dg = a[1] - b[1]
        val db = a[2] - b[2]
        return dr * dr + dg * dg + db * db
    }

    private fun average(k: Int, count: IntArray, r: LongArray, g: LongArray, b: LongArray): Int {
        val n = count[k].toLong()
        return argb((r[k] / n).toInt(), (g[k] / n).toInt(), (b[k] / n).toInt())
    }
}

/** The colours a page is tinted with (ARGB). Every pair meets the contrast it is used for. */
data class ArtScheme(
    val primary: Int,
    val onPrimary: Int,
    val primaryContainer: Int,
    val onPrimaryContainer: Int,
    val background: Int,
    val onBackground: Int,
    val surfaceVariant: Int,
    val onSurfaceVariant: Int,
    /** Raised surfaces (cards, sheets), lowest to highest. */
    val containerLow: Int,
    val container: Int,
    val containerHigh: Int,
) {
    companion object {
        /** Contrast for text (WCAG AA, normal text). */
        const val TEXT_CONTRAST = 4.5

        /** Contrast for accents, icons and large text (WCAG AA, non-text). */
        const val ACCENT_CONTRAST = 3.0

        fun from(seed: Int, dark: Boolean): ArtScheme {
            val c = Hsl.of(seed)
            // A grey cover tints barely; a vivid one is calmed so text stays comfortable.
            val s = if (c.s < 0.08) 0.06 else c.s.coerceIn(0.3, 0.75)
            val h = c.h
            val muted = min(s, 0.3)
            // The accent is text (links, section headings) on the page, on raised surfaces and
            // on the header's gradient (primaryContainer into the background): it must read
            // on all of them, and the text on it must still read once it has moved.
            return if (dark) {
                val bg = Hsl(h, muted, 0.08).argb()
                val pc = Hsl(h, s * 0.8, 0.26).argb()
                val low = Hsl(h, muted, 0.11).argb()
                val mid = Hsl(h, muted, 0.14).argb()
                val high = Hsl(h, muted, 0.18).argb()
                val sv = Hsl(h, muted, 0.22).argb()
                val surfaces = listOf(bg, low, mid, high, sv)
                val primary = ensureOnAll(Hsl(h, s, 0.72).argb(), surfaces + pc, TEXT_CONTRAST)
                val onPrimary = ensure(Hsl(h, s, 0.14).argb(), primary, TEXT_CONTRAST)
                ArtScheme(
                    primary = primary,
                    onPrimary = onPrimary,
                    primaryContainer = pc,
                    onPrimaryContainer = ensure(Hsl(h, s, 0.9).argb(), pc, TEXT_CONTRAST),
                    background = bg,
                    onBackground = ensureOnAll(Hsl(h, 0.1, 0.92).argb(), surfaces, TEXT_CONTRAST),
                    surfaceVariant = sv,
                    onSurfaceVariant = ensureOnAll(Hsl(h, 0.12, 0.8).argb(), surfaces, TEXT_CONTRAST),
                    containerLow = low,
                    container = mid,
                    containerHigh = high,
                )
            } else {
                val bg = Hsl(h, muted, 0.97).argb()
                val pc = Hsl(h, s * 0.8, 0.86).argb()
                val low = Hsl(h, muted, 0.95).argb()
                val mid = Hsl(h, muted, 0.93).argb()
                val high = Hsl(h, muted, 0.9).argb()
                val sv = Hsl(h, muted, 0.88).argb()
                val surfaces = listOf(bg, low, mid, high, sv)
                val primary = ensureOnAll(Hsl(h, s, 0.36).argb(), surfaces + pc, TEXT_CONTRAST)
                val onPrimary = ensure(0xFFFFFFFF.toInt(), primary, TEXT_CONTRAST)
                ArtScheme(
                    primary = primary,
                    onPrimary = onPrimary,
                    primaryContainer = pc,
                    onPrimaryContainer = ensure(Hsl(h, s, 0.14).argb(), pc, TEXT_CONTRAST),
                    background = bg,
                    onBackground = ensureOnAll(Hsl(h, 0.1, 0.1).argb(), surfaces, TEXT_CONTRAST),
                    surfaceVariant = sv,
                    onSurfaceVariant = ensureOnAll(Hsl(h, 0.12, 0.28).argb(), surfaces, TEXT_CONTRAST),
                    containerLow = low,
                    container = mid,
                    containerHigh = high,
                )
            }
        }

        /**
         * [fg], moved (as [ensure] does) until it meets [min] on every one of [bgs]. The
         * backgrounds are all light or all dark, so each step away from the worst one also
         * moves away from the rest; a few rounds at most.
         */
        fun ensureOnAll(fg: Int, bgs: List<Int>, min: Double): Int {
            var c = fg
            repeat(bgs.size + 1) {
                val worst = bgs.minByOrNull { contrast(c, it) } ?: return c
                if (contrast(c, worst) >= min) return c
                c = ensure(c, worst, min)
            }
            return c
        }

        /**
         * [fg], made lighter or darker (whichever moves away from [bg]) until its contrast
         * with [bg] is at least [min]; black or white if nothing in between will do.
         */
        fun ensure(fg: Int, bg: Int, min: Double): Int {
            if (contrast(fg, bg) >= min) return fg
            val c = Hsl.of(fg)
            val lighter = luminance(bg) < 0.18
            var l = c.l
            repeat(100) {
                l = if (lighter) (l + 0.01).coerceAtMost(1.0) else (l - 0.01).coerceAtLeast(0.0)
                val x = Hsl(c.h, c.s, l).argb()
                if (contrast(x, bg) >= min) return x
            }
            val black = 0xFF000000.toInt()
            val white = 0xFFFFFFFF.toInt()
            return if (contrast(white, bg) >= contrast(black, bg)) white else black
        }
    }
}

fun argb(r: Int, g: Int, b: Int): Int =
    (0xFF shl 24) or (r.coerceIn(0, 255) shl 16) or (g.coerceIn(0, 255) shl 8) or b.coerceIn(0, 255)

/** WCAG relative luminance, 0 (black) to 1 (white). */
fun luminance(c: Int): Double {
    fun ch(v: Int): Double {
        val s = v / 255.0
        return if (s <= 0.03928) s / 12.92 else ((s + 0.055) / 1.055).pow(2.4)
    }
    return 0.2126 * ch((c shr 16) and 0xff) + 0.7152 * ch((c shr 8) and 0xff) + 0.0722 * ch(c and 0xff)
}

/** WCAG contrast ratio, 1 to 21. */
fun contrast(a: Int, b: Int): Double {
    val la = luminance(a)
    val lb = luminance(b)
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)
}

/** Hue (degrees), saturation and lightness (0–1). */
data class Hsl(val h: Double, val s: Double, val l: Double) {
    fun argb(): Int {
        val ss = s.coerceIn(0.0, 1.0)
        val ll = l.coerceIn(0.0, 1.0)
        val c = (1 - abs(2 * ll - 1)) * ss
        val hh = ((h % 360) + 360) % 360 / 60.0
        val x = c * (1 - abs(hh % 2 - 1))
        val (r, g, b) = when {
            hh < 1 -> Triple(c, x, 0.0)
            hh < 2 -> Triple(x, c, 0.0)
            hh < 3 -> Triple(0.0, c, x)
            hh < 4 -> Triple(0.0, x, c)
            hh < 5 -> Triple(x, 0.0, c)
            else -> Triple(c, 0.0, x)
        }
        val m = ll - c / 2
        fun v(t: Double) = ((t + m) * 255).let { kotlin.math.round(it).toInt() }
        return argb(v(r), v(g), v(b))
    }

    companion object {
        fun of(c: Int): Hsl {
            val r = ((c shr 16) and 0xff) / 255.0
            val g = ((c shr 8) and 0xff) / 255.0
            val b = (c and 0xff) / 255.0
            val mx = max(r, max(g, b))
            val mn = min(r, min(g, b))
            val l = (mx + mn) / 2
            val d = mx - mn
            if (d == 0.0) return Hsl(0.0, 0.0, l)
            val s = d / (1 - abs(2 * l - 1))
            val h = when (mx) {
                r -> 60 * (((g - b) / d) % 6)
                g -> 60 * ((b - r) / d + 2)
                else -> 60 * ((r - g) / d + 4)
            }
            return Hsl((h + 360) % 360, s.coerceIn(0.0, 1.0), l)
        }
    }
}
