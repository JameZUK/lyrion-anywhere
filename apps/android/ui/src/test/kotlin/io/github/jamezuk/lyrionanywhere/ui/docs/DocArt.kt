package io.github.jamezuk.lyrionanywhere.ui.docs

import android.content.Context
import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.LinearGradient
import android.graphics.Paint
import android.graphics.Shader
import coil3.ImageLoader
import coil3.SingletonImageLoader
import coil3.asImage
import coil3.decode.DataSource
import coil3.fetch.FetchResult
import coil3.fetch.Fetcher
import coil3.fetch.ImageFetchResult
import coil3.request.Options
import io.github.jamezuk.lyrionanywhere.ui.art.Artwork
import io.github.jamezuk.lyrionanywhere.ui.art.ArtworkKeyer
import io.github.jamezuk.lyrionanywhere.ui.art.OFFLINE_PREFIX
import java.security.MessageDigest
import kotlin.coroutines.EmptyCoroutineContext
import kotlinx.coroutines.Dispatchers

/**
 * Generated cover art for the documentation screenshots: abstract gradients and rings, the
 * same for the same key, drawn as the web demo's stand-in server draws them
 * (`tools/demo/fake_lms.py`, `cover()`), so the Android and web screenshots match. No real
 * album art is ever used.
 */
object DocArt {
    /** Make every artwork request (the app's Coil loader) answer with generated covers. */
    fun install(context: Context) {
        val loader = ImageLoader.Builder(context)
            .components {
                add(ArtworkKeyer())
                add(Factory())
            }
            // Everything on the calling thread, so a frame or two later the covers are there.
            .coroutineContext(Dispatchers.Unconfined)
            .mainCoroutineContext(EmptyCoroutineContext)
            .build()
        SingletonImageLoader.setUnsafe(loader)
    }

    /** The cover id a key stands for: "1001" for album 1, as the demo server numbers them. */
    fun coverId(key: String): String {
        val k = key.removePrefix(OFFLINE_PREFIX)
        // Downloaded albums are referred to as "a<album id>".
        Regex("^a(\\d+)$").find(k)?.let { return (1000 + it.groupValues[1].toInt()).toString() }
        return k
    }

    fun cover(key: String, size: Int): Bitmap {
        val h = MessageDigest.getInstance("SHA-256").digest(coverId(key).toByteArray()).map { it.toInt() and 0xFF }
        val hue = h[0] / 255.0
        val c1 = hls(hue, 0.42, 0.65)
        val c2 = hls((hue + 0.12 + h[1] / 1200.0) % 1.0, 0.18, 0.55)
        val bmp = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bmp)
        val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            shader = LinearGradient(0f, 0f, 0f, size.toFloat(), c1, c2, Shader.TileMode.CLAMP)
        }
        canvas.drawRect(0f, 0f, size.toFloat(), size.toFloat(), fill)
        val rings = 3 + h[2] % 4
        for (i in 0 until rings) {
            val r = size * (0.12f + h[3 + i] / 255f * 0.35f)
            val x = size * h[8 + i] / 255f
            val y = size * h[12 + i] / 255f
            val ring = Paint(Paint.ANTI_ALIAS_FLAG).apply {
                style = Paint.Style.STROKE
                strokeWidth = maxOf(2, size / 60).toFloat()
                color = hls((hue + 0.5 + i * 0.07) % 1.0, 0.6 + i * 0.05, 0.5)
            }
            canvas.drawCircle(x, y, r, ring)
        }
        return bmp
    }

    /** Python's colorsys.hls_to_rgb, as an opaque ARGB colour. */
    private fun hls(h: Double, l: Double, s: Double): Int {
        fun v(m1: Double, m2: Double, hue0: Double): Double {
            val hue = ((hue0 % 1.0) + 1.0) % 1.0
            return when {
                hue < 1.0 / 6 -> m1 + (m2 - m1) * hue * 6.0
                hue < 0.5 -> m2
                hue < 2.0 / 3 -> m1 + (m2 - m1) * (2.0 / 3 - hue) * 6.0
                else -> m1
            }
        }
        val m2 = if (l <= 0.5) l * (1.0 + s) else l + s - l * s
        val m1 = 2.0 * l - m2
        val r = v(m1, m2, h + 1.0 / 3)
        val g = v(m1, m2, h)
        val b = v(m1, m2, h - 1.0 / 3)
        fun c(x: Double) = (x * 255).toInt().coerceIn(0, 255)
        return (0xFF shl 24) or (c(r) shl 16) or (c(g) shl 8) or c(b)
    }

    private class Factory : Fetcher.Factory<Artwork> {
        override fun create(data: Artwork, options: Options, imageLoader: ImageLoader): Fetcher = Fetcher {
            val size = data.sizePx.coerceIn(64, 800)
            ImageFetchResult(cover(data.key, size).asImage(), false, DataSource.MEMORY) as FetchResult
        }
    }
}
