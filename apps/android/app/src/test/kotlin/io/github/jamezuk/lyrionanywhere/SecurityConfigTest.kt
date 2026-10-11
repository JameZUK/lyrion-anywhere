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
    fun onlyTheLauncherActivityIsExported() {
        val manifest = xml("src/main/AndroidManifest.xml")
        val exported = listOf("activity", "service", "receiver", "provider")
            .flatMap { manifest.children(it) }
            .filter { it.getAttributeNS(android, "exported") == "true" }
        assertEquals(listOf(".MainActivity"), exported.map { it.getAttributeNS(android, "name") })
    }

    @Test
    fun thePlaybackServiceIsPrivateAndAMediaForegroundService() {
        val services = xml("src/main/AndroidManifest.xml").children("service")
        val playback = services.single { it.getAttributeNS(android, "name").endsWith(".PlaybackService") }
        assertEquals("false", playback.getAttributeNS(android, "exported"))
        assertEquals("mediaPlayback", playback.getAttributeNS(android, "foregroundServiceType"))
        assertFalse(playback.hasAttributeNS(android, "permission"))
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
            ),
            perms,
        )
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
