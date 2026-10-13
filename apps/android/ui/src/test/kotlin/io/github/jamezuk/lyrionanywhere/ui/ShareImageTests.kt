package io.github.jamezuk.lyrionanywhere.ui

import android.content.Context
import android.content.Intent
import android.content.pm.ProviderInfo
import android.graphics.Bitmap
import android.net.Uri
import androidx.core.content.FileProvider
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import io.github.jamezuk.lyrionanywhere.ui.stats.ShareImageProvider
import io.github.jamezuk.lyrionanywhere.ui.stats.ShareImages
import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.annotation.Config
import org.w3c.dom.Element

/**
 * Sharing a story card: the image stays on the device in the cache's shared folder, is offered
 * through a provider that is not exported, with a read-only grant for that one URI, and is
 * deleted later. Nothing else can be reached through the provider.
 */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class ShareImageTests {
    private val context: Context get() = ApplicationProvider.getApplicationContext()
    private val android = "http://schemas.android.com/apk/res/android"
    private lateinit var provider: ShareImageProvider

    @Before
    fun attach() {
        val info = ProviderInfo().apply {
            authority = ShareImages.authority(context)
            exported = false
            grantUriPermissions = true
        }
        provider = Robolectric.buildContentProvider(ShareImageProvider::class.java).create(info).get()
    }

    private fun bitmap() = Bitmap.createBitmap(40, 60, Bitmap.Config.ARGB_8888)

    private fun uriFor(file: File): Uri = FileProvider.getUriForFile(context, ShareImages.authority(context), file)

    @Test
    fun theImageIsWrittenToTheCachesSharedFolder() {
        val file = ShareImages.write(context, bitmap(), now = 1_000)
        assertEquals(File(context.cacheDir, "shared").canonicalPath, file.parentFile!!.canonicalPath)
        assertTrue(file.length() > 0)
        val uri = uriFor(file)
        assertEquals("content", uri.scheme)
        assertEquals(ShareImages.authority(context), uri.authority)
        assertFalse("no file path in the URI", uri.toString().contains(context.cacheDir.path))
    }

    @Test
    fun theShareGrantsReadingThatOneUriOnly() {
        val uri = uriFor(ShareImages.write(context, bitmap()))
        val intent = ShareImages.intent(uri, "My year in sound 2026")
        assertEquals(Intent.ACTION_SEND, intent.action)
        assertEquals("image/png", intent.type)
        assertEquals(uri, intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java))
        assertTrue(intent.flags and Intent.FLAG_GRANT_READ_URI_PERMISSION != 0)
        assertEquals(0, intent.flags and Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        assertEquals(0, intent.flags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
        assertEquals(0, intent.flags and Intent.FLAG_GRANT_PREFIX_URI_PERMISSION)
        val clip = intent.clipData!!
        assertEquals(1, clip.itemCount)
        assertEquals(uri, clip.getItemAt(0).uri)
        // Through the system share sheet.
        val chooser = ShareImages.chooser(uri, "My year in sound 2026", "Share your card")
        assertEquals(Intent.ACTION_CHOOSER, chooser.action)
        assertEquals(intent.action, chooser.getParcelableExtra(Intent.EXTRA_INTENT, Intent::class.java)?.action)
    }

    @Test
    fun oldCardsAreDeletedAndNewOnesKept() {
        val dir = ShareImages.dir(context).apply { mkdirs() }
        val old = File(dir, "old.png").apply { writeText("x"); setLastModified(1_000) }
        val recent = File(dir, "recent.png").apply { writeText("x"); setLastModified(1_000 + ShareImages.KEEP_MILLIS - 1) }
        ShareImages.prune(dir, now = 1_000 + ShareImages.KEEP_MILLIS)
        assertFalse(old.exists())
        assertTrue(recent.exists())
        ShareImages.prune(dir, now = 1_000 + ShareImages.KEEP_MILLIS, keepMillis = 0)
        assertFalse(recent.exists())
    }

    @Test
    fun writingPrunesFirst() {
        val dir = ShareImages.dir(context).apply { mkdirs() }
        val stale = File(dir, "stale.png").apply { writeText("x"); setLastModified(0) }
        ShareImages.write(context, bitmap(), now = ShareImages.KEEP_MILLIS * 2)
        assertFalse(stale.exists())
    }

    @Test
    fun filesOutsideTheSharedFolderCannotBeOffered() {
        for (f in listOf(
            File(context.filesDir, "secret.txt"),
            File(context.cacheDir, "other.png"),
            File(context.dataDir, "shared_prefs/settings.xml"),
            File(ShareImages.dir(context), "../escape.png"),
        )) {
            try {
                uriFor(f)
                fail("offered $f")
            } catch (expected: IllegalArgumentException) {
            }
        }
    }

    @Test
    fun theProviderRefusesTraversalAndDeletesNothing() {
        val file = ShareImages.write(context, bitmap())
        val uri = uriFor(file)
        assertEquals(0, provider.delete(uri, null, null))
        assertTrue(file.exists())
        val traversal = Uri.parse("content://${ShareImages.authority(context)}/shared/..%2F..%2Ffiles%2Fsecret.txt")
        try {
            provider.openFile(traversal, "r")
            fail("opened a path outside the shared folder")
        } catch (expected: SecurityException) {
        } catch (expected: IllegalArgumentException) {
        } catch (expected: java.io.FileNotFoundException) {
        }
        for (mode in listOf("w", "rw", "wt", "wa", "rwt")) {
            try {
                provider.openFile(uri, mode)
                fail("opened for $mode")
            } catch (expected: SecurityException) {
            }
        }
        provider.openFile(uri, "r")!!.close()
        // A root it doesn't serve is refused.
        try {
            provider.openFile(Uri.parse("content://${ShareImages.authority(context)}/files/secret.txt"), "r")
            fail("opened another root")
        } catch (expected: IllegalArgumentException) {
        }
    }

    private fun xml(path: String): Element {
        val f = DocumentBuilderFactory.newInstance().apply {
            isNamespaceAware = true
            setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
        }
        return f.newDocumentBuilder().parse(File(path)).documentElement
    }

    @Test
    fun theProviderIsNotExportedAndOnlyGrants() {
        val providers = xml("src/main/AndroidManifest.xml").getElementsByTagName("provider")
        assertEquals(1, providers.length)
        val p = providers.item(0) as Element
        assertEquals("io.github.jamezuk.lyrionanywhere.ui.stats.ShareImageProvider", p.getAttributeNS(android, "name"))
        assertEquals("false", p.getAttributeNS(android, "exported"))
        assertEquals("true", p.getAttributeNS(android, "grantUriPermissions"))
        assertEquals("\${applicationId}.share", p.getAttributeNS(android, "authorities"))
        assertFalse(p.hasAttributeNS(android, "permission"))
        assertEquals(0, p.getElementsByTagName("intent-filter").length)
        // No other component comes with the UI library.
        for (tag in listOf("activity", "service", "receiver")) {
            assertEquals(tag, 0, xml("src/main/AndroidManifest.xml").getElementsByTagName(tag).length)
        }
    }

    @Test
    fun theProviderServesTheSharedCacheFolderOnly() {
        val paths = xml("src/main/res/xml/share_paths.xml")
        val children = (0 until paths.childNodes.length).map { paths.childNodes.item(it) }.filterIsInstance<Element>()
        assertEquals(listOf("cache-path"), children.map { it.tagName })
        assertEquals("shared/", children.single().getAttribute("path"))
    }
}
