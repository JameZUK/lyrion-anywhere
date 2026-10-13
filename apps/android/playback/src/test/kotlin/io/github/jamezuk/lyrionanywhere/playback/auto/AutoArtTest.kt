package io.github.jamezuk.lyrionanywhere.playback.auto

import android.content.ContentValues
import android.content.Intent
import android.net.Uri
import android.os.Looper
import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import androidx.media3.session.MediaLibraryService.MediaLibrarySession
import androidx.media3.session.MediaSession
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.core.BrowseNode
import io.github.jamezuk.lyrionanywhere.playback.LmsSessionPlayer
import io.github.jamezuk.lyrionanywhere.playback.RecordingActions
import io.github.jamezuk.lyrionanywhere.playback.SessionSnapshot
import java.io.File
import java.io.FileNotFoundException
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.runTest
import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val AUTHORITY = "io.github.jamezuk.lyrionanywhere.autoart"

/** A tiny JPEG-looking cover (only its header is checked). */
private fun jpeg(size: Int = 64) = ByteArray(size).also {
    it[0] = 0xff.toByte()
    it[1] = 0xd8.toByte()
    it[2] = 0xff.toByte()
}

/** The rules: which URIs name a cover, and who may read one. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class AutoArtAccessTest {
    private val key = "0123456789abcdef0123456789abcdef"

    @Test
    fun onlyArtSlashAnOpaqueKeyNamesACover() {
        assertEquals(key, AutoArtAccess.keyOf(listOf("art", key)))
        for (bad in listOf(
            null,
            emptyList(),
            listOf("art"),
            listOf(key),
            listOf("art", key, "x"),
            listOf("art", "..", key),
            listOf("..", "art", key),
            listOf("art", "../$key"),
            listOf("art", "$key/.."),
            listOf("art", key.uppercase()),
            listOf("art", key.dropLast(1)),
            listOf("art", key + "0"),
            listOf("art", "music/7/cover"),
            listOf("art", "/data/data/io.github.jamezuk.lyrionanywhere/files/x"),
            listOf("art", "%2e%2e%2f$key".take(32)),
            listOf("ART", key),
        )) {
            assertNull(bad.toString(), AutoArtAccess.keyOf(bad))
        }
    }

    @Test
    fun onlyReadingIsAllowed() {
        assertTrue(AutoArtAccess.readOnly("r"))
        for (mode in listOf("w", "wt", "wa", "rw", "rwt", "R", "", " r", null)) assertFalse("$mode", AutoArtAccess.readOnly(mode))
    }

    @Test
    fun aCallerNeedsToBeThisAppOrHoldAGrant() {
        val path = listOf("art", key)
        assertEquals(key, AutoArtAccess.decide(path, "r", callerIsSelf = true, callerGranted = false))
        assertEquals(key, AutoArtAccess.decide(path, "r", callerIsSelf = false, callerGranted = true))
        assertNull("wrong caller", AutoArtAccess.decide(path, "r", callerIsSelf = false, callerGranted = false))
        assertNull("write mode", AutoArtAccess.decide(path, "rw", callerIsSelf = true, callerGranted = true))
        assertNull("traversal", AutoArtAccess.decide(listOf("art", "..", key), "r", callerIsSelf = true, callerGranted = true))
    }

    @Test
    fun onlyImagesOfABoundedSizeAreKept() {
        assertTrue(AutoArtAccess.isImage(jpeg()))
        val png = byteArrayOf(0x89.toByte(), 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0)
        assertTrue(AutoArtAccess.isImage(png))
        val webp = "RIFF\u0000\u0000\u0000\u0000WEBP".toByteArray(Charsets.ISO_8859_1)
        assertTrue(AutoArtAccess.isImage(webp))
        assertTrue(AutoArtAccess.isImage("GIF89a......".toByteArray()))
        for (bad in listOf(null, ByteArray(0), jpeg().copyOf(5), "<html>hello</html>".toByteArray(), "#!/bin/sh\necho".toByteArray())) {
            assertFalse(AutoArtAccess.isImage(bad))
        }
        assertFalse("too big", AutoArtAccess.isImage(jpeg(AutoArtCache.MAX_BYTES + 1)))
    }

    @Test
    fun grantsGoOnlyToAndroidAutoAndOnlyForReading() {
        val granted = mutableListOf<Triple<String, Uri?, Int>>()
        val g = AutoArtGrants { pkg, uri, flags -> granted += Triple(pkg, uri, flags) }
        g.grantFor("com.example.other", CallerAccess.BROWSE, listOf(Uri.EMPTY))
        g.grantFor(AutoArtGrants.ANDROID_AUTO, CallerAccess.CONTROL, listOf(Uri.EMPTY))
        g.grantFor(AutoArtGrants.ANDROID_AUTO, CallerAccess.REJECT, listOf(Uri.EMPTY))
        assertTrue(granted.isEmpty())
        g.grantFor(AutoArtGrants.ANDROID_AUTO, CallerAccess.BROWSE, listOf(Uri.EMPTY, Uri.EMPTY))
        assertEquals(2, granted.size)
        assertTrue(granted.all { it.first == AutoArtGrants.ANDROID_AUTO && it.third == Intent.FLAG_GRANT_READ_URI_PERMISSION })
        // Never write, persistable or prefix grants.
        assertEquals(0, AutoArtGrants.FLAGS and Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        assertEquals(0, AutoArtGrants.FLAGS and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        assertEquals(0, AutoArtGrants.FLAGS and Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
    }
}

/** The cache of covers on disk, and the provider that serves them to Android Auto. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class AutoArtCacheTest {
    @get:Rule val tmp = TemporaryFolder()

    private val fetched = mutableListOf<String>()
    private var answer: (String) -> ByteArray? = { jpeg() }

    private fun cache(dir: File = tmp.newFolder()) = AutoArtCache(dir, AUTHORITY, fetch = { k ->
        fetched += k
        answer(k)
    })

    @Test
    fun aCoverIsFetchedOnceAndServedByAnOpaqueKey() = runTest {
        val c = cache()
        val uri = c.uriFor("music/7/cover")!!
        assertEquals("content", uri.scheme)
        assertEquals(AUTHORITY, uri.authority)
        val key = AutoArtAccess.keyOf(uri.pathSegments)!!
        // Nothing of the artwork's own key shows in the URI.
        assertFalse(uri.toString().contains("music") || uri.toString().contains("cover"))
        assertEquals(uri, c.uriFor("music/7/cover"))
        assertEquals(listOf("music/7/cover"), fetched)
        assertArrayEquals(jpeg(), c.fileFor(key)!!.readBytes())
        // Another cover, another random key.
        assertTrue(c.uriFor("music/8/cover") != uri)
    }

    @Test
    fun onlyImagesAreKept() = runTest {
        val c = cache()
        for (bytes in listOf(null, ByteArray(0), "<html/>".toByteArray(), jpeg(AutoArtCache.MAX_BYTES + 1))) {
            answer = { bytes }
            assertNull(c.uriFor("music/1/cover"))
        }
        answer = { throw IllegalStateException("network") }
        assertNull(c.uriFor("music/1/cover"))
        // Keys that are blank or absurdly long are never fetched.
        fetched.clear()
        assertNull(c.uriFor(null))
        assertNull(c.uriFor(" "))
        assertNull(c.uriFor("x".repeat(AutoArtCache.MAX_ART_KEY + 1)))
        assertTrue(fetched.isEmpty())
    }

    @Test
    fun nothingOutsideTheCacheIsEverNamed() = runTest {
        val dir = tmp.newFolder()
        val c = cache(dir)
        File(dir.parentFile, "secret").writeText("not for the car")
        for (bad in listOf(null, "", "..", "../secret", "secret", "0123456789abcdef0123456789abcde/", "0123456789ABCDEF0123456789ABCDEF")) {
            assertNull(bad, c.fileFor(bad))
        }
        // A well-formed key with no file: nothing.
        assertNull(c.fileFor("0123456789abcdef0123456789abcdef"))
    }

    @Test
    fun theCacheIsBoundedAndCleared() = runTest {
        val dir = tmp.newFolder()
        val c = cache(dir)
        val first = c.uriFor("cover/0")!!
        repeat(AutoArtCache.MAX_FILES) { c.uriFor("cover/${it + 1}") }
        assertNull("the oldest went", c.fileFor(AutoArtAccess.keyOf(first.pathSegments)))
        assertEquals(AutoArtCache.MAX_FILES, dir.listFiles()!!.size)
        assertEquals(AutoArtCache.MAX_FILES, c.uris().size)
        c.clear()
        assertTrue(dir.listFiles()!!.isEmpty())
        assertTrue(c.uris().isEmpty())
        // A new run starts empty.
        c.uriFor("cover/x")
        cache(dir)
        assertTrue(dir.listFiles()!!.isEmpty())
    }

    @Test
    fun severalAtOnceWithATimeoutEach() = runBlocking {
        val c = AutoArtCache(tmp.newFolder(), AUTHORITY, fetch = { k ->
            if (k == "slow") delay(60_000)
            if (k == "none") null else jpeg()
        })
        val uris = c.urisFor(listOf("a", null, "none", "slow", "b"))
        assertEquals(5, uris.size)
        assertNotNull(uris[0])
        assertNull(uris[1])
        assertNull(uris[2])
        assertNull("timed out", uris[3])
        assertNotNull(uris[4])
    }

    /** A provider whose caller can be set, over a cache in the test's folder. */
    class TestProvider : AutoArtProvider() {
        override fun cache() = testCache

        override fun callerIsSelf() = self

        override fun callerGranted(uri: Uri) = granted

        companion object {
            var testCache: AutoArtCache? = null
            var self = false
            var granted = false
        }
    }

    private fun provider(): AutoArtProvider =
        Robolectric.buildContentProvider(TestProvider::class.java).create(AUTHORITY).get()

    @After
    fun reset() {
        TestProvider.testCache = null
        TestProvider.self = false
        TestProvider.granted = false
    }

    @Test
    fun theProviderServesAGrantedCoverReadOnly() = runTest {
        val c = cache()
        TestProvider.testCache = c
        val uri = c.uriFor("music/7/cover")!!
        val p = provider()
        TestProvider.granted = true
        p.openFile(uri, "r").use { fd ->
            assertArrayEquals(jpeg(), java.io.FileInputStream(fd.fileDescriptor).readBytes())
        }
        assertEquals("image/*", p.getType(uri))
    }

    @Test
    fun theProviderRefusesEverythingElse() = runTest {
        val c = cache()
        TestProvider.testCache = c
        val uri = c.uriFor("music/7/cover")!!
        val p = provider()
        fun refused(u: Uri, mode: String, expect: Class<out Exception>) {
            try {
                p.openFile(u, mode).close()
                fail("served $u ($mode)")
            } catch (e: Exception) {
                assertTrue("$u $mode: $e", expect.isInstance(e))
            }
        }
        // A caller without a grant.
        refused(uri, "r", SecurityException::class.java)
        TestProvider.granted = true
        // Write modes.
        for (mode in listOf("w", "rw", "wt", "wa", "rwt")) refused(uri, mode, SecurityException::class.java)
        // Traversal and other paths.
        val base = "content://$AUTHORITY"
        for (bad in listOf("$base/art/../art/x", "$base/art/%2e%2e%2fsecret", "$base/secret", "$base/art", "$base/art/${"a".repeat(32)}/x", "$base/")) {
            refused(Uri.parse(bad), "r", SecurityException::class.java)
        }
        // A key it never handed out.
        refused(Uri.parse("$base/art/${"a".repeat(32)}"), "r", FileNotFoundException::class.java)
        // Safe mode (no cache): nothing.
        TestProvider.testCache = null
        refused(uri, "r", FileNotFoundException::class.java)
        // Nothing can be written, listed or queried.
        assertNull(p.query(uri, null, null, null, null))
        assertNull(p.getType(Uri.parse("$base/secret")))
        for (write in listOf<() -> Unit>(
            { p.insert(uri, ContentValues()) },
            { p.update(uri, ContentValues(), null, null) },
            { p.delete(uri, null, null) },
        )) {
            try {
                write()
                fail("written")
            } catch (_: UnsupportedOperationException) {
            }
        }
    }
}

/** Album items get their covers' URIs, and the car (alone) is granted them. */
@OptIn(UnstableApi::class)
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class AutoArtCatalogueTest {
    private val source = FakeAutoSource()
    private val codec = FakeCodec()
    private val asked = mutableListOf<List<String?>>()
    private val art = AutoArtUris { keys ->
        asked += keys
        keys.map { k -> k?.takeIf { !it.contains("none") }?.let { Uri.parse("content://$AUTHORITY/art/${"%032x".format(it.hashCode())}") } }
    }
    private val tree = AutoCatalogue(source, { it::class.simpleName ?: "?" }, codec, art)

    @Test
    fun albumsCarryTheirCoverFromTheAppsProviderOnly() = runTest {
        val albums = tree.children(codec.id(BrowseNode.Albums)!!, 0, 3)!!
        assertEquals(listOf("music/1/cover", "music/2/cover", "music/3/cover"), asked.single())
        assertTrue(albums.all { it.mediaMetadata.artworkUri?.authority == AUTHORITY })
        // Downloaded albums ask for their cached cover.
        tree.children(codec.id(BrowseNode.Downloads)!!, 0, 10)
        assertEquals(listOf("offline:a7", "offline:a8"), asked.last())
        // Tracks and folders carry none.
        val tracks = tree.children(codec.id(BrowseNode.Album(7))!!, 0, 3)!!
        assertTrue(tracks.all { it.mediaMetadata.artworkUri == null })
    }

    @Test
    fun withoutArtNothingChanges() = runTest {
        val failing = AutoCatalogue(source, { it::class.simpleName ?: "?" }, codec) { throw IllegalStateException("no") }
        val albums = failing.children(codec.id(BrowseNode.Albums)!!, 0, 3)!!
        assertEquals(3, albums.size)
        assertTrue(albums.all { it.mediaMetadata.artworkUri == null })
    }

    @Test
    fun onlyTheVerifiedCarIsGrantedTheCovers() {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val granted = mutableListOf<Pair<String, Uri>>()
        val grants = AutoArtGrants { pkg, uri, _ -> granted += pkg to uri }
        fun controller(pkg: String, uid: Int) =
            MediaSession.ControllerInfo.createTestOnlyControllerInfo(pkg, 0, uid, 0, 0, false, android.os.Bundle.EMPTY, true)
        val car = controller(AutoArtGrants.ANDROID_AUTO, 10_001)
        val system = controller("com.android.systemui", 10_004)
        val decisions = mapOf(car.packageName to CallerAccess.BROWSE, system.packageName to CallerAccess.BROWSE)
        val callback = LibraryCallback({ decisions.getValue(it.packageName) }, { tree }, scope, grants)
        val player = LmsSessionPlayer(Looper.getMainLooper(), MutableStateFlow(SessionSnapshot()), RecordingActions(), scope)
        val session = MediaLibrarySession.Builder(ApplicationProvider.getApplicationContext<android.content.Context>(), player, callback).build()
        try {
            val f = callback.onGetChildren(session, car, codec.id(BrowseNode.Albums)!!, 0, 3, null)
            shadowOf(Looper.getMainLooper()).idle()
            val items = f.get(5, TimeUnit.SECONDS).value!!
            assertEquals(items.map { it.mediaMetadata.artworkUri }, granted.map { it.second })
            assertTrue(granted.all { it.first == AutoArtGrants.ANDROID_AUTO })
            // Another browser (the system's own) gets the same items but no grant.
            granted.clear()
            val g = callback.onGetChildren(session, system, codec.id(BrowseNode.Albums)!!, 0, 3, null)
            shadowOf(Looper.getMainLooper()).idle()
            g.get(5, TimeUnit.SECONDS)
            assertTrue(granted.isEmpty())
        } finally {
            session.release()
            player.release()
            scope.cancel()
        }
    }
}
