package io.github.jamezuk.lyrionanywhere

import android.app.ActivityManager
import android.app.Application
import android.app.ApplicationExitInfo
import android.content.Context
import androidx.test.core.app.ApplicationProvider
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config
import org.robolectric.shadows.ShadowActivityManager.ApplicationExitInfoBuilder

/**
 * How the app starts after Android reported it dying (review, 15 October): safe mode only
 * after repeated crashes; an ANR or a single crash shows its report once, then is forgotten.
 */
@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35], application = Application::class)
class CrashLogStartTest {
    private val context: Context = ApplicationProvider.getApplicationContext()
    private val now = System.currentTimeMillis()

    @Before
    fun clean() {
        CrashLog.clear(context)
        context.getSharedPreferences("crash_log", Context.MODE_PRIVATE).edit().clear().commit()
    }

    private fun died(reason: Int, at: Long) {
        val am = context.getSystemService(ActivityManager::class.java)
        shadowOf(am).addApplicationExitInfo(
            ApplicationExitInfoBuilder.newBuilder()
                .setProcessName(context.packageName)
                .setReason(reason)
                .setTimestamp(at)
                .setDescription("test")
                .build(),
        )
    }

    @Test
    fun aCleanStartIsNormal() {
        val start = CrashLog.start(context)
        assertEquals(CrashLog.StartMode.NORMAL, start.mode)
        assertNull(start.report)
    }

    @Test
    fun anAnrIsShownOnceButIsNeverSafeMode() {
        died(ApplicationExitInfo.REASON_ANR, now - 30_000)
        died(ApplicationExitInfo.REASON_ANR, now - 20_000)
        val start = CrashLog.start(context)
        assertEquals(CrashLog.StartMode.REPORT_ONCE, start.mode)
        assertTrue(start.report!!.contains("Not responding"))
        CrashLog.clear(context)
        assertEquals(CrashLog.StartMode.NORMAL, CrashLog.start(context).mode)
    }

    @Test
    fun oneCrashIsReportedOnceThenTheAppCarriesOn() {
        died(ApplicationExitInfo.REASON_CRASH, now - 30_000)
        val start = CrashLog.start(context)
        assertEquals(CrashLog.StartMode.REPORT_ONCE, start.mode)
        assertNotNull(start.report)
        CrashLog.clear(context)
        assertEquals(CrashLog.StartMode.NORMAL, CrashLog.start(context).mode)
    }

    @Test
    fun aCrashAtEveryStartIsSafeModeUntilRestart() {
        died(ApplicationExitInfo.REASON_CRASH, now - 60_000)
        // Shown once; the app carried on and crashed again.
        CrashLog.clear(context)
        died(ApplicationExitInfo.REASON_CRASH_NATIVE, now - 30_000)
        assertEquals(CrashLog.StartMode.SAFE, CrashLog.start(context).mode)
        // Restart (clear) starts normally again.
        CrashLog.clear(context)
        assertEquals(CrashLog.StartMode.NORMAL, CrashLog.start(context).mode)
    }

    @Test
    fun aFailureThatDidntEndTheAppIsReportedOnce() {
        CrashLog.record(context, "Background task failed", IllegalStateException("boom"))
        val start = CrashLog.start(context)
        assertEquals(CrashLog.StartMode.REPORT_ONCE, start.mode)
        assertTrue(start.report!!.contains("Background task failed"))
    }
}
