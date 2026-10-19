package io.github.jamezuk.lyrionanywhere.ui

import android.util.Size
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.google.zxing.BarcodeFormat
import com.google.zxing.EncodeHintType
import com.google.zxing.qrcode.QRCodeWriter
import io.github.jamezuk.lyrionanywhere.ui.onboarding.ANALYSIS_SIZE
import io.github.jamezuk.lyrionanywhere.ui.onboarding.QrAnalyzer
import io.github.jamezuk.lyrionanywhere.ui.onboarding.qrAnalysis
import io.github.jamezuk.lyrionanywhere.ui.onboarding.scannerSide
import androidx.compose.ui.unit.dp
import java.util.concurrent.Executor
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * The QR analyser reads the web app's pairing link from a camera frame as CameraX delivers
 * it: a luminance plane with padded rows, in sensor orientation (rotated against the
 * preview), the code small and off-centre, and the frames at the size the scanner asks for.
 * A third-party scanner read the code when the app's own did not (1.15.6): the app decoded
 * CameraX's default 640 x 480 frames, too few pixels per module from a normal distance.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class QrAnalyzerTests {
    private val link = "lyrion-anywhere://pair?server=https%3A%2F%2Fmusic.example.com&code=ABCDEFGHJK"

    /** A luminance frame as CameraX hands it over: bytes, row stride, width and height. */
    private class Frame(val bytes: ByteArray, val stride: Int, val width: Int, val height: Int)

    /**
     * The web app's symbol (4-module quiet zone) drawn at [left], [top] of a [width] x
     * [height] picture, mid-grey elsewhere (a room). [rotate] gives the frame the sensor's
     * orientation, the picture turned a quarter turn, as the analyser sees it on a phone held
     * upright. [padding] is extra bytes per row (CameraX's row stride is often wider).
     */
    private fun frame(
        width: Int,
        height: Int,
        pxPerModule: Int,
        left: Int,
        top: Int,
        padding: Int = 0,
        rotate: Boolean = false,
        invert: Boolean = false,
    ): Frame {
        val matrix = QRCodeWriter().encode(
            link,
            BarcodeFormat.QR_CODE,
            0,
            0,
            mapOf(EncodeHintType.MARGIN to 4),
        )
        val dark: Byte = if (invert) 0xf0.toByte() else 0x10
        val light: Byte = if (invert) 0x10 else 0xf0.toByte()
        val frameWidth = if (rotate) height else width
        val frameHeight = if (rotate) width else height
        val stride = frameWidth + padding
        val bytes = ByteArray(stride * frameHeight) { 0x70 }
        val size = matrix.width * pxPerModule
        for (y in 0 until size) {
            for (x in 0 until size) {
                val v = if (matrix.get(x / pxPerModule, y / pxPerModule)) dark else light
                val px = left + x
                val py = top + y
                // A quarter turn clockwise: the picture's row becomes the frame's column.
                val (fx, fy) = if (rotate) (frameWidth - 1 - py) to px else px to py
                bytes[fy * stride + fx] = v
            }
        }
        return Frame(bytes, stride, frameWidth, frameHeight)
    }

    private fun decode(f: Frame): String? = QrAnalyzer {}.decode(f.bytes, f.stride, f.width, f.height)

    @Test
    fun readsTheCodeFromAPaddedRotatedFrameAtTheRequestedSize() {
        // The tablet at arm's length: the 232 px code is about an eighth of the frame.
        val w = ANALYSIS_SIZE.width
        val h = ANALYSIS_SIZE.height
        assertEquals(link, decode(frame(w, h, pxPerModule = 5, left = 300, top = 200, padding = 64)))
        assertEquals(link, decode(frame(w, h, pxPerModule = 5, left = 300, top = 200, padding = 64, rotate = true)))
        assertEquals(link, decode(frame(w, h, pxPerModule = 5, left = 300, top = 200, invert = true)))
        assertEquals(link, decode(frame(w, h, pxPerModule = 5, left = 1500, top = 1100, rotate = true, invert = true)))
    }

    @Test
    fun aFrameTooSmallForItsRowsIsRefusedNotRead() {
        val f = frame(640, 480, pxPerModule = 4, left = 10, top = 10)
        assertNull(QrAnalyzer {}.decode(f.bytes.copyOf(f.bytes.size / 2), f.stride, f.width, f.height))
    }

    @Test
    fun theAnalysisUseCaseAsksForTheLargerFrames() {
        val analysis = qrAnalysis(Executor { it.run() }, QrAnalyzer {})
        val strategy = analysis.resolutionSelector?.resolutionStrategy
        assertEquals(ANALYSIS_SIZE, strategy?.boundSize)
        assertEquals(ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER, strategy?.fallbackRule)
        assertEquals(Size(1920, 1440), ANALYSIS_SIZE)
    }
}

/** The scanner preview fits the window in both orientations. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class ScannerSideTests {
    @Test
    fun portraitTakesTheColumnsWidthAndLandscapeFitsTheHeight() {
        // A phone upright: 360 dp wide column, 800 dp tall window.
        assertEquals(360.dp, scannerSide(360.dp, 800))
        // The same phone on its side: a 480 dp column would overflow a 400 dp window.
        assertEquals(220.dp, scannerSide(480.dp, 400))
        // A tablet: the column's cap, never more.
        assertEquals(480.dp, scannerSide(480.dp, 1200))
    }
}
