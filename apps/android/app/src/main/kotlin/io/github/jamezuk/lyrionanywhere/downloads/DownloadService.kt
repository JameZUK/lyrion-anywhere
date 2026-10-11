package io.github.jamezuk.lyrionanywhere.downloads

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import io.github.jamezuk.lyrionanywhere.LyrionApp
import io.github.jamezuk.lyrionanywhere.core.DownloadState
import io.github.jamezuk.lyrionanywhere.core.DownloadsSnapshot
import io.github.jamezuk.lyrionanywhere.ui.R
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.distinctUntilChanged
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.launch

/** What the downloads notification shows. Pure: tested on the JVM. */
data class DownloadProgress(
    /** Tracks still to download (queued, running or waiting). */
    val left: Int,
    /** Progress of what is downloading now, 0…100 (null: unknown). */
    val percent: Int?,
    val waitingForNetwork: Boolean,
) {
    companion object {
        fun of(s: DownloadsSnapshot?): DownloadProgress {
            val busy = s?.items.orEmpty().filter {
                it.state == DownloadState.QUEUED || it.state == DownloadState.ACTIVE || it.state == DownloadState.WAITING
            }
            val active = busy.filter { it.state == DownloadState.ACTIVE && it.total > 0uL }
            val percent = if (active.isEmpty()) {
                null
            } else {
                val got = active.sumOf { it.received.toDouble() }
                val total = active.sumOf { it.total.toDouble() }
                (got * 100 / total).toInt().coerceIn(0, 100)
            }
            return DownloadProgress(busy.size, percent, s?.waitingForNetwork == true)
        }
    }
}

/**
 * Keeps downloads going when the app is in the background: a foreground service of type
 * `dataSync` (no WorkManager, no start-up initialiser), with a progress notification and a
 * Cancel action. It runs only while downloads are busy, and stops itself when they are done.
 *
 * Android 15 limits `dataSync` services to six hours a day: on [onTimeout] the downloads are
 * paused (what they have is kept) and the service stops; they carry on when the app is next
 * opened. Not exported: only this app starts it (the Cancel action is an explicit,
 * immutable PendingIntent to it).
 */
class DownloadService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var started = false

    override fun onBind(intent: Intent?): IBinder? = null

    private val app: LyrionApp? get() = application as? LyrionApp

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val a = app?.takeIf { it.started }
        // Always go to the foreground first: a service started as a foreground one must, even
        // if it then stops at once.
        ensureChannel(this)
        val first = DownloadProgress.of(a?.container?.offline?.state?.value?.downloads)
        ServiceCompat.startForeground(
            this,
            NOTIFICATION_ID,
            notification(first),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
        if (a == null) {
            ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
            stopSelf()
            return START_NOT_STICKY
        }
        if (intent?.action == ACTION_CANCEL) {
            a.container.offline.cancel(null)
        }
        if (!started) {
            started = true
            scope.launch {
                a.container.offline.state.map { DownloadProgress.of(it.downloads) }.distinctUntilChanged().collect { p ->
                    if (p.left == 0) {
                        ServiceCompat.stopForeground(this@DownloadService, ServiceCompat.STOP_FOREGROUND_REMOVE)
                        stopSelf()
                    } else {
                        getSystemService(NotificationManager::class.java)?.notify(NOTIFICATION_ID, notification(p))
                    }
                }
            }
        }
        return START_NOT_STICKY
    }

    /** Android 15+: this service type's daily time is used up. Pause, keep what we have. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        app?.takeIf { it.started }?.container?.offline?.pause(null)
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun notification(p: DownloadProgress) = NotificationCompat.Builder(this, CHANNEL)
        .setSmallIcon(android.R.drawable.stat_sys_download)
        .setContentTitle(getString(R.string.downloads_notification_title))
        .setContentText(
            if (p.waitingForNetwork) {
                getString(R.string.downloads_notification_waiting)
            } else {
                resources.getQuantityString(R.plurals.downloads_notification_left, p.left, p.left)
            },
        )
        .setProgress(100, p.percent ?: 0, p.percent == null && !p.waitingForNetwork)
        .setOngoing(true)
        .setSilent(true)
        .setOnlyAlertOnce(true)
        .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
        .setCategory(NotificationCompat.CATEGORY_PROGRESS)
        .apply { openApp()?.let { setContentIntent(it) } }
        .addAction(0, getString(R.string.action_cancel), cancelIntent(this))
        .build()

    private fun openApp(): PendingIntent? = packageManager.getLaunchIntentForPackage(packageName)?.let {
        PendingIntent.getActivity(this, 0, it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    companion object {
        const val CHANNEL = "downloads"
        const val NOTIFICATION_ID = 0x0D0A
        const val ACTION_CANCEL = "io.github.jamezuk.lyrionanywhere.action.CANCEL_DOWNLOADS"

        fun cancelIntent(context: Context): PendingIntent = PendingIntent.getService(
            context,
            1,
            Intent(context, DownloadService::class.java).setAction(ACTION_CANCEL),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )

        fun ensureChannel(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java) ?: return
            if (nm.getNotificationChannel(CHANNEL) != null) return
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL, context.getString(R.string.downloads_notification_channel), NotificationManager.IMPORTANCE_LOW),
            )
        }

        /**
         * Start the service (downloads became busy). Allowed while the app is in the
         * foreground; from the background Android refuses, and the downloads carry on as
         * long as the process lives (and resume when the app next opens).
         */
        fun start(context: Context) {
            try {
                ContextCompat.startForegroundService(context, Intent(context, DownloadService::class.java))
            } catch (_: IllegalStateException) {
                // Not allowed from the background (ForegroundServiceStartNotAllowedException).
            } catch (_: SecurityException) {
                // The foreground-service permission is missing: nothing to show.
            }
        }
    }
}
