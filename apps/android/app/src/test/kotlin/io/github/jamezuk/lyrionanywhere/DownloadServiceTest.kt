package io.github.jamezuk.lyrionanywhere

import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.DownloadItem
import io.github.jamezuk.lyrionanywhere.core.DownloadOrigin
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.DownloadsSnapshot
import io.github.jamezuk.lyrionanywhere.core.NetworkKind
import io.github.jamezuk.lyrionanywhere.downloads.DownloadProgress
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** What the downloads notification shows (the service itself needs a device). */
class DownloadServiceTest {
    private fun item(id: Long, state: DownloadState, received: Long = 0, total: Long = 0) =
        DownloadItem(id, null, "Song $id", state, received.toULong(), total.toULong(), DownloadFormat.ORIGINAL, DownloadOrigin.MANUAL, null, "")

    private fun snap(vararg items: DownloadItem, waiting: Boolean = false) =
        DownloadsSnapshot(items.toList(), false, waiting, NetworkKind.WIFI, 0uL, 0uL, 0u)

    @Test
    fun countsWhatIsLeftAndTheRunningProgress() {
        val p = DownloadProgress.of(
            snap(
                item(1, DownloadState.ACTIVE, 50, 100),
                item(2, DownloadState.ACTIVE, 0, 300),
                item(3, DownloadState.QUEUED),
                item(4, DownloadState.DONE),
                item(5, DownloadState.PAUSED),
                item(6, DownloadState.FAILED),
                item(7, DownloadState.WAITING),
            ),
        )
        assertEquals(4, p.left)
        assertEquals(12, p.percent)
    }

    @Test
    fun nothingBusyMeansTheServiceStops() {
        assertEquals(DownloadProgress(0, null, false), DownloadProgress.of(null))
        assertEquals(0, DownloadProgress.of(snap(item(1, DownloadState.DONE), item(2, DownloadState.PAUSED))).left)
    }

    @Test
    fun waitingForWifiIsShownAndUnknownSizesAreIndeterminate() {
        val p = DownloadProgress.of(snap(item(1, DownloadState.QUEUED), waiting = true))
        assertTrue(p.waitingForNetwork)
        assertNull(p.percent)
        assertNull(DownloadProgress.of(snap(item(1, DownloadState.ACTIVE))).percent)
    }
}
