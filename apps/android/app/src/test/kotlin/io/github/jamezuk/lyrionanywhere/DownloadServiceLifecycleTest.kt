package io.github.jamezuk.lyrionanywhere

import android.app.Application
import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import androidx.test.core.app.ApplicationProvider
import io.github.jamezuk.lyrionanywhere.downloads.DownloadService
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

/**
 * The downloads service's basics (review, 15 October: untested). Without a started app
 * (safe mode, or Android starting it alone) it must still go to the foreground as Android
 * requires, then stop at once; its Cancel action reaches only this service.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class DownloadServiceLifecycleTest {
    private val context: Context = ApplicationProvider.getApplicationContext()

    @Test
    fun withoutAStartedAppItStopsAtOnce() {
        val controller = Robolectric.buildService(DownloadService::class.java, Intent(context, DownloadService::class.java)).create()
        controller.startCommand(0, 1)
        val service = shadowOf(controller.get())
        assertTrue("stopped itself", service.isStoppedBySelf)
        assertTrue("left the foreground", service.isForegroundStopped)
        // Its channel exists, low importance (no sound for a progress bar).
        val channel = context.getSystemService(NotificationManager::class.java).getNotificationChannel(DownloadService.CHANNEL)
        assertNotNull(channel)
        assertEquals(NotificationManager.IMPORTANCE_LOW, channel.importance)
    }

    @Test
    fun theCancelActionIsExplicitAndImmutable() {
        val pending = shadowOf(DownloadService.cancelIntent(context))
        assertTrue(pending.isServiceIntent)
        assertTrue("immutable: no one can fill it in", pending.isImmutable)
        val intent = pending.savedIntent
        assertEquals(DownloadService::class.java.name, intent.component?.className)
        assertEquals(context.packageName, intent.component?.packageName)
        assertEquals(DownloadService.ACTION_CANCEL, intent.action)
    }
}
