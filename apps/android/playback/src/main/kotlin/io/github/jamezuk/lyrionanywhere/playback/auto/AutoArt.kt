package io.github.jamezuk.lyrionanywhere.playback.auto

import android.content.ContentProvider
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.database.Cursor
import android.net.Uri
import android.os.Binder
import android.os.ParcelFileDescriptor
import android.os.Process
import io.github.jamezuk.lyrionanywhere.playback.PlaybackHost
import java.io.File
import java.io.FileNotFoundException
import java.security.SecureRandom
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withTimeoutOrNull

/*
 * Album art for Android Auto's browse lists (Wave 2 §5.2).
 *
 * Android Auto loads an item's artwork from a URI. Ours point at [AutoArtProvider], a
 * content provider that is NOT exported: Android Auto can read a URI only because the app
 * granted it that one URI, read-only ([AutoArtGrants]). The provider serves only covers the
 * app has already fetched through the gateway, cached on disk under random, opaque keys:
 * never a path, a server address or the artwork's own key, and nothing outside the cache.
 */

/** Covers as URIs for the car's items, one per artwork key (null where there is none). */
fun interface AutoArtUris {
    suspend fun urisFor(artKeys: List<String?>): List<Uri?>

    companion object {
        /** An artwork key naming a downloaded album's cached cover (by album reference). */
        const val OFFLINE_PREFIX = "offline:"
    }
}

/** Who may read a URI, and what a URI names: pure rules, tested on the JVM. */
object AutoArtAccess {
    /** The URI's one path prefix. */
    const val PATH = "art"

    private val KEY = Regex("^[0-9a-f]{32}$")

    /** An opaque cache key: 32 lower-case hex characters. */
    fun isKey(s: String?): Boolean = s != null && KEY.matches(s)

    /** The key a URI's path names: exactly `art/<key>`, else null (traversal, extra parts…). */
    fun keyOf(segments: List<String>?): String? {
        if (segments == null || segments.size != 2 || segments[0] != PATH) return null
        return segments[1].takeIf(::isKey)
    }

    /** Only reading: any mode that writes, truncates or appends is refused. */
    fun readOnly(mode: String?): Boolean = mode == "r"

    /**
     * Whether to serve: a well-formed key, opened read-only, by this app or by a caller
     * Android says holds a grant for this URI. Returns the key, or null to refuse.
     */
    fun decide(segments: List<String>?, mode: String?, callerIsSelf: Boolean, callerGranted: Boolean): String? {
        if (!readOnly(mode)) return null
        if (!callerIsSelf && !callerGranted) return null
        return keyOf(segments)
    }

    /** Image bytes we are willing to keep: JPEG, PNG, GIF or WebP, at most [AutoArtCache.MAX_BYTES]. */
    fun isImage(b: ByteArray?): Boolean {
        if (b == null || b.size < 12 || b.size > AutoArtCache.MAX_BYTES) return false
        fun at(i: Int) = b[i].toInt() and 0xff
        val jpeg = at(0) == 0xff && at(1) == 0xd8 && at(2) == 0xff
        val png = at(0) == 0x89 && at(1) == 0x50 && at(2) == 0x4e && at(3) == 0x47
        val gif = at(0) == 0x47 && at(1) == 0x49 && at(2) == 0x46 && at(3) == 0x38
        val webp = at(0) == 0x52 && at(1) == 0x49 && at(2) == 0x46 && at(3) == 0x46 &&
            at(8) == 0x57 && at(9) == 0x45 && at(10) == 0x42 && at(11) == 0x50
        return jpeg || png || gif || webp
    }
}

/**
 * The covers Android Auto may be shown, on disk in [dir] under opaque keys. A cover is put
 * here only after the app fetched it ([fetch], through the gateway or the downloads); the
 * key it is stored under is random and maps back to nothing outside this process. The cache
 * starts empty on each run and is emptied at sign-out.
 */
class AutoArtCache(
    private val dir: File,
    private val authority: String,
    /** A cover by its artwork key (the Art API's, or a download's), or null. */
    private val fetch: suspend (String) -> ByteArray?,
    private val random: SecureRandom = SecureRandom(),
) : AutoArtUris {
    /** Artwork key to opaque key, least recently used first. */
    private val keys = LinkedHashMap<String, String>(16, 0.75f, true)

    init {
        // A new run starts afresh: last run's URIs are dead anyway.
        runCatching { dir.listFiles()?.forEach { it.delete() } }
        runCatching { dir.mkdirs() }
    }

    /** The URI to show for [artKey], fetching and keeping the cover first (null: none). */
    suspend fun uriFor(artKey: String?): Uri? {
        if (artKey.isNullOrBlank() || artKey.length > MAX_ART_KEY) return null
        synchronized(keys) { keys[artKey] }?.let { k -> if (fileFor(k) != null) return uri(k) }
        val bytes = runCatching { fetch(artKey) }.getOrNull()
        if (!AutoArtAccess.isImage(bytes)) return null
        val key = newKey()
        val target = File(dir, key)
        val tmp = File(dir, "$key.tmp")
        val written = runCatching {
            tmp.writeBytes(bytes!!)
            tmp.renameTo(target)
        }.getOrDefault(false)
        if (!written) {
            tmp.delete()
            return null
        }
        val (kept, evicted) = synchronized(keys) {
            // Fetched twice at once: keep the first (its URI may already be out), drop ours.
            keys[artKey]?.let { return@synchronized it to listOf(key) }
            keys[artKey] = key
            val out = mutableListOf<String>()
            while (keys.size > MAX_FILES) {
                val oldest = keys.keys.first()
                keys.remove(oldest)?.let(out::add)
            }
            key to out
        }
        evicted.forEach { File(dir, it).delete() }
        return uri(kept)
    }

    /** The cached file for an opaque [key], only inside the cache (null: none). */
    fun fileFor(key: String?): File? {
        if (!AutoArtAccess.isKey(key)) return null
        val f = File(dir, key!!)
        val inside = runCatching { f.canonicalFile.parentFile == dir.canonicalFile }.getOrDefault(false)
        return f.takeIf { inside && it.isFile }
    }

    /** Every URI handed out (for revoking grants). */
    fun uris(): List<Uri> = synchronized(keys) { keys.values.toList() }.map(::uri)

    /** Forget everything (sign-out). */
    fun clear() {
        synchronized(keys) { keys.clear() }
        runCatching { dir.listFiles()?.forEach { it.delete() } }
    }

    fun uri(key: String): Uri = Uri.Builder().scheme("content").authority(authority).appendPath(AutoArtAccess.PATH).appendPath(key).build()

    private fun newKey(): String {
        val b = ByteArray(16)
        random.nextBytes(b)
        return b.joinToString("") { "%02x".format(it.toInt() and 0xff) }
    }

    /** URIs for several covers at once, a few at a time, each given [FETCH_TIMEOUT_MS]. */
    override suspend fun urisFor(artKeys: List<String?>): List<Uri?> = coroutineScope {
        val gate = Semaphore(PARALLEL)
        artKeys.map { k -> async { gate.withPermit { withTimeoutOrNull(FETCH_TIMEOUT_MS) { uriFor(k) } } } }.awaitAll()
    }

    companion object {
        /** Covers kept at once (each about 300 px, so a few megabytes). */
        const val MAX_FILES = 300

        /** The largest cover kept. */
        const val MAX_BYTES = 1024 * 1024

        /** The size covers are fetched at, pixels. */
        const val SIZE_PX = 320

        const val MAX_ART_KEY = 512
        const val PARALLEL = 4
        const val FETCH_TIMEOUT_MS = 3_000L

        /** The provider's authority for a package. */
        fun authorityFor(packageName: String) = "$packageName.autoart"
    }
}

/**
 * Grants Android Auto read access to the artwork URIs of the items it was just given: one
 * URI at a time, read-only, and only to the Android Auto host package once [CallerPolicy] has
 * verified its signature (a car browser connected with [CallerAccess.BROWSE]).
 */
class AutoArtGrants(private val grant: (packageName: String, uri: Uri, flags: Int) -> Unit) {
    fun grantFor(packageName: String, access: CallerAccess, uris: List<Uri>) {
        if (!mayGrant(packageName, access)) return
        uris.forEach { grant(packageName, it, FLAGS) }
    }

    companion object {
        /** Read, nothing else: never write, never persistable, never prefix. */
        const val FLAGS = Intent.FLAG_GRANT_READ_URI_PERMISSION

        /** The Android Auto host app. */
        const val ANDROID_AUTO = "com.google.android.projection.gearhead"

        fun mayGrant(packageName: String, access: CallerAccess): Boolean =
            access == CallerAccess.BROWSE && packageName == ANDROID_AUTO

        fun forContext(context: Context) = AutoArtGrants { pkg, uri, flags -> runCatching { context.grantUriPermission(pkg, uri, flags) } }
    }
}

/**
 * Serves cached covers to Android Auto (see the top of this file). Not exported; read-only;
 * a caller other than this app needs Android's grant for that exact URI. Nothing can be
 * written, listed or queried.
 */
open class AutoArtProvider : ContentProvider() {
    override fun onCreate(): Boolean = true

    /** The cache (the app's); null in safe mode. */
    protected open fun cache(): AutoArtCache? = (context?.applicationContext as? PlaybackHost)?.autoArt

    /** Whether the caller is this app itself. */
    protected open fun callerIsSelf(): Boolean = Binder.getCallingUid() == Process.myUid()

    /** Whether Android says the caller holds a read grant for [uri]. */
    protected open fun callerGranted(uri: Uri): Boolean =
        context?.checkCallingUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION) == PackageManager.PERMISSION_GRANTED

    override fun openFile(uri: Uri, mode: String): ParcelFileDescriptor {
        val key = AutoArtAccess.decide(uri.pathSegments, mode, callerIsSelf(), callerGranted(uri))
            ?: throw SecurityException("not readable")
        val file = cache()?.fileFor(key) ?: throw FileNotFoundException("no such cover")
        return ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)
    }

    override fun getType(uri: Uri): String? = if (AutoArtAccess.keyOf(uri.pathSegments) != null) "image/*" else null

    override fun query(uri: Uri, projection: Array<out String>?, selection: String?, selectionArgs: Array<out String>?, sortOrder: String?): Cursor? =
        null

    override fun insert(uri: Uri, values: ContentValues?): Uri? = throw UnsupportedOperationException("read-only")

    override fun update(uri: Uri, values: ContentValues?, selection: String?, selectionArgs: Array<out String>?): Int =
        throw UnsupportedOperationException("read-only")

    override fun delete(uri: Uri, selection: String?, selectionArgs: Array<out String>?): Int =
        throw UnsupportedOperationException("read-only")
}
