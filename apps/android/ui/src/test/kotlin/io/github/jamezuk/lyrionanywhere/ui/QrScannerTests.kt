package io.github.jamezuk.lyrionanywhere.ui

import android.os.Looper
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.google.common.util.concurrent.ListenableFuture
import io.github.jamezuk.lyrionanywhere.ui.onboarding.CameraLease
import io.github.jamezuk.lyrionanywhere.ui.onboarding.QrScanner
import io.github.jamezuk.lyrionanywhere.ui.theme.LyrionTheme
import java.util.concurrent.CancellationException
import java.util.concurrent.Executor
import java.util.concurrent.TimeUnit
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * M15: leaving the pairing screen before the camera provider is ready must not leave the
 * camera bound (running with no preview, and still reading QR codes) afterwards.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class QrScannerTests {
    @get:Rule val compose = createComposeRule()

    /** A camera provider the test hands over when it chooses. */
    private class PendingFuture<P : Any> : ListenableFuture<P> {
        private val listeners = mutableListOf<Pair<Runnable, Executor>>()
        private var value: P? = null
        var cancelled = false

        fun complete(v: P) {
            value = v
            listeners.forEach { (r, e) -> e.execute(r) }
            listeners.clear()
        }

        override fun addListener(listener: Runnable, executor: Executor) {
            if (value != null) executor.execute(listener) else listeners += listener to executor
        }

        override fun cancel(mayInterruptIfRunning: Boolean): Boolean {
            cancelled = true
            return true
        }

        override fun isCancelled() = cancelled

        override fun isDone() = value != null || cancelled

        override fun get(): P = value ?: throw CancellationException()

        override fun get(timeout: Long, unit: TimeUnit): P = get()
    }

    /** The fake camera: counts what is bound. */
    private class Camera {
        var binds = 0
        var unbinds = 0
        val bound get() = binds > unbinds
    }

    private val direct = Executor { it.run() }

    @Test
    fun aProviderThatArrivesAfterDisposalIsNeverBound() {
        val future = PendingFuture<Camera>()
        val lease = CameraLease(future, direct, bind = { it.binds++ }, unbind = { it.unbinds++ })
        lease.start()
        lease.dispose()
        val camera = Camera()
        future.complete(camera)
        assertEquals(0, camera.binds)
        assertTrue(!camera.bound)
    }

    @Test
    fun aBoundCameraIsReleasedOnDisposal() {
        val future = PendingFuture<Camera>()
        val lease = CameraLease(future, direct, bind = { it.binds++ }, unbind = { it.unbinds++ })
        lease.start()
        val camera = Camera()
        future.complete(camera)
        assertTrue(camera.bound)
        lease.dispose()
        assertTrue(!camera.bound)
        // Disposing twice is harmless.
        lease.dispose()
        assertEquals(1, camera.unbinds)
    }

    @Test
    fun leavingTheScreenBeforeTheCameraIsReadyLeavesItOff() {
        val future = PendingFuture<Camera>()
        val camera = Camera()
        var shown by mutableStateOf(true)
        compose.setContent {
            LyrionTheme(dynamicColour = false) {
                if (shown) {
                    QrScanner(
                        active = true,
                        onScanned = {},
                        modifier = Modifier,
                        openCamera = { future },
                        bind = { c, _, _, _ -> c.binds++ },
                        unbind = { it.unbinds++ },
                    )
                }
            }
        }
        compose.waitForIdle()
        shown = false
        compose.waitForIdle()
        // The provider turns up only now, after the screen has gone.
        future.complete(camera)
        shadowOf(Looper.getMainLooper()).idle()
        compose.waitForIdle()
        assertEquals("the camera was bound after the scanner left", 0, camera.binds)
        assertTrue(!camera.bound)
    }
}
