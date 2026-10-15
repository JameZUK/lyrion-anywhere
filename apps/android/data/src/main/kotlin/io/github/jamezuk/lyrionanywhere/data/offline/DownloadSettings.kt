package io.github.jamezuk.lyrionanywhere.data.offline

import io.github.jamezuk.lyrionanywhere.core.DownloadFormat
import io.github.jamezuk.lyrionanywhere.core.DownloadPolicy

/**
 * This device's download settings (Settings → Downloads and storage). Nothing here is
 * secret; they are kept with the other settings.
 */
data class DownloadSettings(
    /** The format on Wi-Fi (or any unmetered network). */
    val wifiFormat: DownloadFormat = DownloadFormat.ORIGINAL,
    /** The format on mobile data. */
    val mobileFormat: DownloadFormat = DownloadFormat.OPUS,
    /** Download only on Wi-Fi (on by default, as on the web); on mobile data downloads wait. */
    val wifiOnly: Boolean = true,
    /** Most storage downloads may use, in bytes (0: no limit). */
    val capBytes: Long = 0,
    /** "Keep recently added albums offline": how many (0: off). */
    val smartRecentAlbums: Int = 0,
    /** "Keep favourites offline". */
    val smartFavourites: Boolean = false,
) {
    val smartOn: Boolean get() = smartRecentAlbums > 0 || smartFavourites

    /** Values outside the offered choices become the nearest sensible one. */
    fun sanitised(): DownloadSettings = copy(
        capBytes = capBytes.coerceIn(0, MAX_CAP),
        smartRecentAlbums = smartRecentAlbums.coerceIn(0, MAX_RECENT),
    )

    /** What the download manager enforces. */
    fun policy(): DownloadPolicy = DownloadPolicy(
        wifiFormat = wifiFormat,
        mobileFormat = mobileFormat,
        wifiOnly = wifiOnly,
        capBytes = capBytes.coerceAtLeast(0).toULong(),
    )

    companion object {
        const val GB = 1024L * 1024 * 1024
        const val MAX_CAP = 1024 * GB
        const val MAX_RECENT = 100

        /** The storage caps offered (0: no limit). */
        val CAP_CHOICES = listOf(0L, 1 * GB, 2 * GB, 5 * GB, 10 * GB, 20 * GB, 50 * GB, 100 * GB)

        /** How many recently added albums may be kept (0: off). */
        val RECENT_CHOICES = listOf(0, 5, 10, 20, 50)
    }
}
