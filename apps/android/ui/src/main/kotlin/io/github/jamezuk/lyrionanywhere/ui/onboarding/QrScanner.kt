package io.github.jamezuk.lyrionanywhere.ui.onboarding

import android.content.Context
import android.util.Size
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.common.util.concurrent.ListenableFuture
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.MultiFormatReader
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import io.github.jamezuk.lyrionanywhere.ui.R
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

/**
 * The analysis frames' size. CameraX's default is 640 x 480: the pairing QR code is 232 px
 * on a tablet's screen, so from a normal distance it covered under two pixels per module
 * there and ZXing never read it. At this size (or the nearest the camera offers) it has four
 * or more, and a 2 MP frame still decodes in well under a frame interval.
 */
internal val ANALYSIS_SIZE = Size(1920, 1440)

/** The analysis use case the scanner binds: frames of [ANALYSIS_SIZE], latest only. */
internal fun qrAnalysis(executor: Executor, analyzer: ImageAnalysis.Analyzer): ImageAnalysis =
    ImageAnalysis.Builder()
        .setResolutionSelector(
            ResolutionSelector.Builder()
                .setResolutionStrategy(
                    ResolutionStrategy(ANALYSIS_SIZE, ResolutionStrategy.FALLBACK_RULE_CLOSEST_HIGHER_THEN_LOWER),
                )
                .build(),
        )
        .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
        .build()
        .also { it.setAnalyzer(executor, analyzer) }

/**
 * Reads QR codes from camera frames (the luminance plane only) with ZXing. The multi-format
 * reader, limited to QR: it is the one that honours ALSO_INVERTED (the plain QR reader
 * ignores the hint).
 */
internal class QrAnalyzer(private val onText: (String) -> Unit) : ImageAnalysis.Analyzer {
    private val reader = MultiFormatReader().apply {
        setHints(
            mapOf(
                DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE),
                DecodeHintType.TRY_HARDER to true,
                // A light-on-dark code (a page in dark mode) reads too.
                DecodeHintType.ALSO_INVERTED to true,
            ),
        )
    }
    val enabled = AtomicBoolean(true)

    override fun analyze(image: ImageProxy) {
        image.use {
            if (!enabled.get()) return
            val plane = it.planes[0]
            val buffer = plane.buffer
            val rowStride = plane.rowStride
            val bytes = ByteArray(buffer.remaining()).also(buffer::get)
            val text = decode(bytes, rowStride, it.width, it.height) ?: return
            onText(text)
        }
    }

    fun decode(luminance: ByteArray, rowStride: Int, width: Int, height: Int): String? {
        if (luminance.size < rowStride * (height - 1) + width) return null
        val source = PlanarYUVLuminanceSource(luminance, rowStride, height, 0, 0, width, height, false)
        return try {
            reader.decodeWithState(BinaryBitmap(HybridBinarizer(source))).text
        } catch (_: NotFoundException) {
            null
        } catch (_: ReaderException) {
            null
        } finally {
            reader.reset()
        }
    }
}

/**
 * The camera, once its provider is ready: [bind] when the future resolves, [unbind] on
 * [dispose]. A provider that turns up after [dispose] (the screen left within the first few
 * hundred milliseconds) is never bound, so the camera can't run on with no preview.
 *
 * The future is not cancelled: CameraX hands out one shared future for its process-wide
 * provider, and cancelling it could break the next scanner.
 */
internal class CameraLease<P : Any>(
    private val future: ListenableFuture<P>,
    private val executor: Executor,
    private val bind: (P) -> Unit,
    private val unbind: (P) -> Unit,
) {
    // Set and read on [executor] and the thread that disposes (both the main thread), but
    // atomic in case a caller passes another executor.
    private val disposed = AtomicBoolean(false)
    private var provider: P? = null

    fun start() {
        future.addListener({
            if (disposed.get()) return@addListener
            val p = runCatching { future.get() }.getOrNull() ?: return@addListener
            provider = p
            runCatching { bind(p) }
            // Disposed while binding: release it again at once.
            if (disposed.get()) release()
        }, executor)
    }

    fun dispose() {
        disposed.set(true)
        release()
    }

    private fun release() {
        provider?.let { runCatching { unbind(it) } }
        provider = null
    }
}

/** A camera preview that reports QR codes it reads while [active]. */
@Composable
fun QrScanner(active: Boolean, onScanned: (String) -> Unit, modifier: Modifier = Modifier) {
    QrScanner(
        active = active,
        onScanned = onScanned,
        modifier = modifier,
        openCamera = { context -> ProcessCameraProvider.getInstance(context) },
        bind = { provider, owner, preview, analysis ->
            provider.unbindAll()
            provider.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
        },
        unbind = { provider -> provider.unbindAll() },
    )
}

/** [QrScanner] over any camera provider (a fake in tests). */
@Composable
internal fun <P : Any> QrScanner(
    active: Boolean,
    onScanned: (String) -> Unit,
    modifier: Modifier,
    openCamera: (Context) -> ListenableFuture<P>,
    bind: (P, LifecycleOwner, Preview, ImageAnalysis) -> Unit,
    unbind: (P) -> Unit,
) {
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    val latestOnScanned by rememberUpdatedState(onScanned)
    val mainExecutor = remember(context) { ContextCompat.getMainExecutor(context) }
    val analyzer = remember {
        QrAnalyzer { text -> mainExecutor.execute { latestOnScanned(text) } }
    }
    // Not during composition (which may be thrown away): once this composition is applied.
    SideEffect { analyzer.enabled.set(active) }
    val previewView = remember(context) { PreviewView(context) }
    val description = stringResource(R.string.camera_preview)

    DisposableEffect(lifecycleOwner) {
        val analysisExecutor = Executors.newSingleThreadExecutor()
        val lease = CameraLease(
            future = openCamera(context),
            executor = mainExecutor,
            bind = { p ->
                val preview = Preview.Builder().build().also { it.surfaceProvider = previewView.surfaceProvider }
                bind(p, lifecycleOwner, preview, qrAnalysis(analysisExecutor, analyzer))
            },
            unbind = unbind,
        )
        lease.start()
        onDispose {
            // No more codes from frames already in flight, then let the camera go.
            analyzer.enabled.set(false)
            lease.dispose()
            analysisExecutor.shutdown()
        }
    }

    AndroidView(
        factory = { previewView },
        modifier = modifier
            .aspectRatio(1f)
            .clip(RoundedCornerShape(16.dp))
            .semantics { contentDescription = description },
    )
}
