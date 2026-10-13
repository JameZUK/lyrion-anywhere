package io.github.jamezuk.lyrionanywhere.ui.stats

import android.content.ClipData
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.net.Uri
import android.os.ParcelFileDescriptor
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.core.content.FileProvider
import io.github.jamezuk.lyrionanywhere.ui.R
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * Serves the story card images being shared, and nothing else: only `cache/shared/` (see
 * res/xml/share_paths.xml). Not exported; the app that receives a card reads it through a
 * read-only grant for that one URI, given with the share. Nothing can be written or deleted.
 */
class ShareImageProvider : FileProvider(R.xml.share_paths) {
    /** Read only, whoever asks. */
    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor? {
        if (mode != "r") throw SecurityException("read only")
        return super.openFile(uri, mode)
    }

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int = 0
}

/** Shares a rendered card. A seam, so tests can see what would be shared. */
fun interface CardSharer {
    suspend fun share(image: ImageBitmap, year: Int)
}

/**
 * Card images on the device: written to a cache file, offered through [ShareImageProvider]
 * to the system share sheet, and deleted after an hour (on the next share, or when the stats
 * open). Nothing is uploaded.
 */
object ShareImages {
    /** The folder under the cache directory (as in share_paths.xml). */
    const val DIR = "shared"

    /** How long a shared card is kept for the receiving app to read it. */
    const val KEEP_MILLIS = 60 * 60 * 1000L

    const val MIME = "image/png"

    fun authority(context: Context): String = context.packageName + ".share"

    fun dir(context: Context): File = File(context.cacheDir, DIR)

    /** Delete the cards older than [KEEP_MILLIS] (all of them with [keepMillis] 0). */
    fun prune(dir: File, now: Long, keepMillis: Long = KEEP_MILLIS) {
        dir.listFiles()?.forEach { f -> if (f.isFile && now - f.lastModified() >= keepMillis) f.delete() }
    }

    /** Write [bitmap] as a PNG under the cache's shared folder, after pruning old cards. */
    fun write(context: Context, bitmap: Bitmap, now: Long = System.currentTimeMillis()): File {
        val dir = dir(context)
        dir.mkdirs()
        prune(dir, now)
        val file = File(dir, "year-in-sound-$now.png")
        // A hardware bitmap (from a graphics layer) is copied to one that can be compressed.
        val soft = if (bitmap.config == Bitmap.Config.HARDWARE) bitmap.copy(Bitmap.Config.ARGB_8888, false) else bitmap
        file.outputStream().use { soft.compress(Bitmap.CompressFormat.PNG, 100, it) }
        return file
    }

    /**
     * The share: one image, read-only, granted for [uri] alone (the clip holds only it, so
     * the grant reaches the receiving app through the chooser).
     */
    fun intent(uri: Uri, subject: String): Intent = Intent(Intent.ACTION_SEND).apply {
        type = MIME
        putExtra(Intent.EXTRA_STREAM, uri)
        putExtra(Intent.EXTRA_SUBJECT, subject)
        clipData = ClipData.newRawUri(subject, uri)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }

    /** The system share sheet for a card. */
    fun chooser(uri: Uri, subject: String, title: String): Intent = Intent.createChooser(intent(uri, subject), title)
}

/** The real sharer: a cache file, a content URI and the system share sheet. */
class AndroidCardSharer(private val context: Context) : CardSharer {
    override suspend fun share(image: ImageBitmap, year: Int) {
        val file = withContext(Dispatchers.IO) { ShareImages.write(context, image.asAndroidBitmap()) }
        val uri = FileProvider.getUriForFile(context, ShareImages.authority(context), file)
        val res = context.resources
        val chooser = ShareImages.chooser(
            uri,
            subject = res.getString(R.string.story_share_subject, year),
            title = res.getString(R.string.story_share_chooser),
        )
        if (context !is android.app.Activity) chooser.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        context.startActivity(chooser)
    }
}
