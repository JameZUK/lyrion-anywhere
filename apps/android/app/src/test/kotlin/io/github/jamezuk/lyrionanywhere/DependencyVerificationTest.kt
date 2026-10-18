package io.github.jamezuk.lyrionanywhere

import java.io.File
import javax.xml.parsers.DocumentBuilderFactory
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.w3c.dom.Element

/**
 * Gradle checks every dependency against `gradle/verification-metadata.xml` (review, 15
 * October): above all the rustls-platform-verifier AAR, the JNI bridge every certificate
 * check goes through, which is fetched from a GitHub branch that can change under us.
 * Before the fix there was no metadata, so a changed AAR would have been built in silently.
 */
class DependencyVerificationTest {
    private val ns = "https://schema.gradle.org/dependency-verification"

    private fun metadata(): Element {
        val f = DocumentBuilderFactory.newInstance().apply {
            isNamespaceAware = true
            setFeature("http://apache.org/xml/features/disallow-doctype-decl", true)
        }
        return f.newDocumentBuilder().parse(File("../gradle/verification-metadata.xml")).documentElement
    }

    private fun Element.children(tag: String): List<Element> {
        val nodes = getElementsByTagNameNS(ns, tag)
        return (0 until nodes.length).map { nodes.item(it) as Element }
    }

    private fun components(): List<Element> = metadata().children("component")

    @Test
    fun metadataIsVerified() {
        val config = metadata().children("configuration").single()
        assertEquals("true", config.children("verify-metadata").single().textContent.trim())
    }

    @Test
    fun theCertificateVerifierAarIsPinnedBySha256() {
        val verifier = components().filter {
            it.getAttribute("group") == "org.rustls" && it.getAttribute("name") == "rustls-platform-verifier"
        }
        assertEquals("one pinned version of the verifier", 1, verifier.size)
        val aar = verifier.single().children("artifact").single { it.getAttribute("name").endsWith(".aar") }
        val sha = aar.children("sha256").single().getAttribute("value")
        assertTrue(sha, Regex("[0-9a-f]{64}").matches(sha))
    }

    @Test
    fun theVerifierVersionPinnedIsTheOneBuilt() {
        val catalogue = File("../gradle/libs.versions.toml").readText()
        val version = Regex("""(?m)^rustls-platform-verifier\s*=\s*"([^"]+)"""").find(catalogue)!!.groupValues[1]
        val pinned = components().single { it.getAttribute("group") == "org.rustls" }.getAttribute("version")
        assertEquals(version, pinned)
    }

    @Test
    fun everyComponentHasAChecksumAndOnlySourcesAndJavadocAreTrusted() {
        val all = components()
        assertTrue("the whole build is covered, not only the verifier", all.size > 100)
        for (c in all) {
            val artifacts = c.children("artifact")
            assertTrue(c.getAttribute("name"), artifacts.isNotEmpty())
            for (a in artifacts) {
                assertTrue("${a.getAttribute("name")} has no sha256", a.children("sha256").isNotEmpty())
            }
        }
        val trusted = metadata().children("trust").map { it.getAttribute("file") }.toSet()
        assertEquals(setOf(".*-javadoc[.]jar", ".*-sources[.]jar"), trusted)
        // No component or group is trusted wholesale.
        assertTrue(metadata().children("trust").none { it.hasAttribute("group") || it.hasAttribute("name") })
    }
}
