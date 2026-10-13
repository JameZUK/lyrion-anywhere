package io.github.jamezuk.lyrionanywhere

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element

/** Regression tests for the app's security settings (manifest and network policy). */
class SecurityConfigTest {
    private val android = "http://schemas.android.com/apk/res/android"

    private fun xml(path: String): Element {
        val f = DocumentBuilderFactory.newInstance().apply {
            isNamespaceAware = true
            // Our own files, but parse them safely all the same.
            setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
        }
        return f.newDocumentBuilder().parse(File(path)).documentElement
    }

    private fun Element.children(tag: String): List<Element> {
        val nodes = getElementsByTagName(tag)
        return (0 until nodes.length).map { nodes.item(it) as Element }
    }

    @Test
    fun noBackupsAndANetworkPolicy() {
        val app = xml("src/main/AndroidManifest.xml").children("application").single()
        assertEquals("false", app.getAttributeNS(android, "allowBackup"))
        assertEquals("false", app.getAttributeNS(android, "fullBackupContent"))
        assertEquals("@xml/data_extraction_rules", app.getAttributeNS(android, "dataExtractionRules"))
        assertEquals("@xml/network_security_config", app.getAttributeNS(android, "networkSecurityConfig"))
        assertFalse(app.hasAttributeNS(android, "usesCleartextTraffic"))
        assertFalse(app.hasAttributeNS(android, "debuggable"))
    }

    @Test
    fun onlyTheLauncherTheWidgetTheTileAndTheMediaLibraryAreExported() {
        val manifest = xml("src/main/AndroidManifest.xml")
        val exported = listOf("activity", "service", "receiver", "provider")
            .flatMap { manifest.children(it) }
            .filter { it.getAttributeNS(android, "exported") == "true" }
        assertEquals(
            setOf(
                ".MainActivity",
                ".widget.NowPlayingWidgetReceiver",
                ".widget.PlayPauseTileService",
                "io.github.jamezuk.lyrionanywhere.playback.PlaybackService",
            ),
            exported.map { it.getAttributeNS(android, "name") }.toSet(),
        )
        fun actions(e: Element) = e.children("action").map { it.getAttributeNS(android, "name") }.toSet()
        // The widget receiver takes only the widget update broadcast.
        val widget = exported.single { it.getAttributeNS(android, "name").endsWith("NowPlayingWidgetReceiver") }
        assertEquals(setOf("android.appwidget.action.APPWIDGET_UPDATE"), actions(widget))
        // Only the system can bind the tile.
        val tile = exported.single { it.getAttributeNS(android, "name").endsWith("PlayPauseTileService") }
        assertEquals("android.permission.BIND_QUICK_SETTINGS_TILE", tile.getAttributeNS(android, "permission"))
        assertEquals(setOf("android.service.quicksettings.action.QS_TILE"), actions(tile))
        // No periodic widget updates: nothing wakes the phone for it.
        val info = xml("src/main/res/xml/now_playing_widget.xml")
        assertEquals("0", info.getAttributeNS(android, "updatePeriodMillis"))
        // No content provider is exported (car artwork isn't served by one).
        assertTrue(exported.none { it.tagName == "provider" })
        // The launcher, voice "play …" (the query only) and pairing links (below).
        val main = exported.single { it.getAttributeNS(android, "name") == ".MainActivity" }
        assertEquals(
            setOf(
                "android.intent.action.MAIN",
                "android.media.action.MEDIA_PLAY_FROM_SEARCH",
                "android.intent.action.VIEW",
            ),
            actions(main),
        )
    }

    /**
     * Pairing links: VIEW of `lyrion-anywhere://pair` only. No other scheme (http, https,
     * content, file), host, path, MIME type or action reaches the app through it, and it
     * claims no web domain.
     */
    @Test
    fun pairingLinksAreTheOnlyDataTheAppOpens() {
        val main = xml("src/main/AndroidManifest.xml").children("activity")
            .single { it.getAttributeNS(android, "name") == ".MainActivity" }
        val withData = main.children("intent-filter").filter { it.children("data").isNotEmpty() }
        val filter = withData.single()
        assertEquals(
            listOf("android.intent.action.VIEW"),
            filter.children("action").map { it.getAttributeNS(android, "name") },
        )
        assertEquals(
            setOf("android.intent.category.DEFAULT", "android.intent.category.BROWSABLE"),
            filter.children("category").map { it.getAttributeNS(android, "name") }.toSet(),
        )
        assertFalse("no App Links verification", filter.hasAttributeNS(android, "autoVerify"))
        val data = filter.children("data").single()
        assertEquals("lyrion-anywhere", data.getAttributeNS(android, "scheme"))
        assertEquals("pair", data.getAttributeNS(android, "host"))
        for (attr in listOf("port", "path", "pathPrefix", "pathPattern", "pathAdvancedPattern", "pathSuffix", "mimeType")) {
            assertFalse("no $attr", data.hasAttributeNS(android, attr))
        }
        // Every other filter carries no data at all.
        assertTrue(
            main.children("intent-filter").filter { it !== filter }.all { it.children("data").isEmpty() },
        )
    }

    /**
     * Voice "play …": the MEDIA_PLAY_FROM_SEARCH filter carries no data and is not BROWSABLE,
     * so a web page can't start playback through it; only the query text is read (bounded,
     * plain text: LaunchIntentsTest).
     */
    @Test
    fun voiceSearchCanNotBeReachedFromTheBrowser() {
        val main = xml("src/main/AndroidManifest.xml").children("activity")
            .single { it.getAttributeNS(android, "name") == ".MainActivity" }
        val filter = main.children("intent-filter").single { f ->
            f.children("action").any { it.getAttributeNS(android, "name") == "android.media.action.MEDIA_PLAY_FROM_SEARCH" }
        }
        assertEquals(
            listOf("android.media.action.MEDIA_PLAY_FROM_SEARCH"),
            filter.children("action").map { it.getAttributeNS(android, "name") },
        )
        assertEquals(
            listOf("android.intent.category.DEFAULT"),
            filter.children("category").map { it.getAttributeNS(android, "name") },
        )
        assertTrue(filter.children("data").isEmpty())
    }

    /** The media library for cars (A6): exported only for the media browser actions; its
     *  callers are checked in code (CallerPolicy, tested in :playback). */
    @Test
    fun thePlaybackServiceIsAMediaLibraryForCarsAndAMediaForegroundService() {
        val services = xml("src/main/AndroidManifest.xml").children("service")
        val playback = services.single { it.getAttributeNS(android, "name").endsWith(".PlaybackService") }
        assertEquals("true", playback.getAttributeNS(android, "exported"))
        assertEquals("mediaPlayback", playback.getAttributeNS(android, "foregroundServiceType"))
        assertEquals(
            setOf("androidx.media3.session.MediaLibraryService", "android.media.browse.MediaBrowserService"),
            playback.children("action").map { it.getAttributeNS(android, "name") }.toSet(),
        )
        assertTrue(playback.children("data").isEmpty() && playback.children("category").isEmpty())
        // Declared as an Android Auto media app, nothing more.
        val app = xml("src/main/AndroidManifest.xml").children("application").single()
        val car = app.children("meta-data").single {
            it.getAttributeNS(android, "name") == "com.google.android.gms.car.application"
        }
        assertEquals("@xml/automotive_app_desc", car.getAttributeNS(android, "resource"))
        val desc = xml("src/main/res/xml/automotive_app_desc.xml")
        assertEquals(listOf("media"), desc.children("uses").map { it.getAttribute("name") })
    }

    @Test
    fun theDownloadServiceIsPrivateAndADataSyncForegroundService() {
        val services = xml("src/main/AndroidManifest.xml").children("service")
        val downloads = services.single { it.getAttributeNS(android, "name").endsWith(".DownloadService") }
        assertEquals("false", downloads.getAttributeNS(android, "exported"))
        assertEquals("dataSync", downloads.getAttributeNS(android, "foregroundServiceType"))
        assertTrue("no intent filter: only explicit intents reach it", downloads.children("intent-filter").isEmpty())
    }

    @Test
    fun noWorkManagerStartup() {
        // Its start-up initialiser crashed earlier test builds on launch; it stays removed.
        val manifest = xml("src/main/AndroidManifest.xml")
        val startup = manifest.children("provider").single {
            it.getAttributeNS(android, "name") == "androidx.startup.InitializationProvider"
        }
        val work = startup.children("meta-data").single {
            it.getAttributeNS(android, "name") == "androidx.work.WorkManagerInitializer"
        }
        assertEquals("remove", work.getAttributeNS("http://schemas.android.com/tools", "node"))
    }

    @Test
    fun onlyTheExpectedPermissions() {
        val perms = xml("src/main/AndroidManifest.xml").children("uses-permission")
            .map { it.getAttributeNS(android, "name") }
            .toSet()
        assertEquals(
            setOf(
                "android.permission.INTERNET",
                "android.permission.CAMERA",
                "android.permission.FOREGROUND_SERVICE",
                "android.permission.FOREGROUND_SERVICE_MEDIA_PLAYBACK",
                "android.permission.WAKE_LOCK",
                // Offline downloads (A5).
                "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
                "android.permission.POST_NOTIFICATIONS",
            ),
            perms,
        )
        // The data module adds only the network state (normal, install-time).
        val dataPerms = xml("../data/src/main/AndroidManifest.xml").children("uses-permission")
            .map { it.getAttributeNS(android, "name") }
            .toSet()
        assertEquals(setOf("android.permission.ACCESS_NETWORK_STATE"), dataPerms)
        // Nothing that reads shared storage: downloads live in app-private storage only.
        assertFalse(perms.any { it.contains("STORAGE") || it.contains("MEDIA_AUDIO") })
        // The playback module adds only what bit-perfect USB output needs (a normal,
        // install-time permission).
        val playbackPerms = xml("../playback/src/main/AndroidManifest.xml").children("uses-permission")
            .map { it.getAttributeNS(android, "name") }
            .toSet()
        assertEquals(setOf("android.permission.MODIFY_AUDIO_SETTINGS"), playbackPerms)
    }

    @Test
    fun releaseRefusesCleartextAndUserCertificates() {
        val cfg = xml("src/main/res/xml/network_security_config.xml")
        val base = cfg.children("base-config").single()
        assertEquals("false", base.getAttribute("cleartextTrafficPermitted"))
        val sources = base.children("certificates").map { it.getAttribute("src") }
        assertEquals(listOf("system"), sources)
        assertTrue(cfg.children("domain-config").isEmpty())
        assertTrue(cfg.children("debug-overrides").isEmpty())
    }

    @Test
    fun debugAllowsCleartextOnlyToLoopback() {
        val cfg = xml("src/debug/res/xml/network_security_config.xml")
        assertEquals("false", cfg.children("base-config").single().getAttribute("cleartextTrafficPermitted"))
        val domains = cfg.children("domain-config").flatMap { it.children("domain") }
        assertEquals(setOf("localhost", "127.0.0.1"), domains.map { it.textContent.trim() }.toSet())
        assertTrue(domains.all { it.getAttribute("includeSubdomains") == "false" })
    }

    @Test
    fun backupRulesExcludeEverything() {
        val rules = xml("src/main/res/xml/data_extraction_rules.xml")
        for (section in listOf("cloud-backup", "device-transfer")) {
            val s = rules.children(section).single()
            assertTrue(s.children("include").isEmpty())
            val excluded = s.children("exclude").map { it.getAttribute("domain") }.toSet()
            assertTrue(excluded.containsAll(setOf("root", "file", "database", "sharedpref", "external")))
        }
    }
}
