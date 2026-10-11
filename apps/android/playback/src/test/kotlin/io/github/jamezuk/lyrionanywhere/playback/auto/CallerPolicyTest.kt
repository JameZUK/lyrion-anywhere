package io.github.jamezuk.lyrionanywhere.playback.auto

import android.content.pm.PackageInfo
import android.content.pm.Signature
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val AUTO = "com.google.android.projection.gearhead"
private val AUTO_CERT = CallerPolicy.KNOWN.first { it.packageName == AUTO }.certificates.first()

private fun facts(
    pkg: String = AUTO,
    matches: Boolean = true,
    self: Boolean = false,
    systemUid: Boolean = false,
    platform: Boolean = false,
    mcc: Boolean = false,
    certs: Set<String> = emptySet(),
    trusted: Boolean = false,
) = CallerFacts(pkg, 10_123, matches, self, systemUid, platform, mcc, certs, trusted)

/** Who may connect to the exported media library: the decision table. */
class CallerPolicyTest {
    @Test
    fun androidAutoWithItsReleaseCertificateMayBrowse() {
        assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(facts(certs = setOf(AUTO_CERT))))
        // Either of its release keys (the original and the rotated one).
        for (c in CallerPolicy.KNOWN.first { it.packageName == AUTO }.certificates) {
            assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(facts(certs = setOf("aa:bb", c))))
        }
    }

    @Test
    fun anImpostorWithAKnownPackageNameIsRejected() {
        // The right name, the wrong certificate: nothing, not even transport controls.
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(facts(certs = setOf("00:11:22"))))
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(facts(certs = emptySet(), trusted = true)))
        // Another known app's certificate doesn't do either.
        val assistant = CallerPolicy.KNOWN.first { it.packageName != AUTO }.certificates.first()
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(facts(certs = setOf(assistant))))
    }

    @Test
    fun aPackageThatIsNotTheCallersIsRejected() {
        // Whatever else is claimed, a package name not belonging to the calling uid is refused.
        assertEquals(
            CallerAccess.REJECT,
            CallerPolicy.decide(facts(matches = false, certs = setOf(AUTO_CERT), platform = true, self = true)),
        )
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(facts(pkg = "", certs = setOf(AUTO_CERT))))
    }

    @Test
    fun theSystemsOwnControllersAndThisAppMayBrowse() {
        assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(facts(pkg = "com.android.systemui", platform = true)))
        assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(facts(pkg = "com.android.bluetooth", systemUid = true)))
        assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(facts(pkg = "com.android.car.media", mcc = true)))
        assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(facts(pkg = "io.github.jamezuk.lyrionanywhere", self = true)))
    }

    @Test
    fun otherTrustedControllersGetTransportControlsOnly() {
        assertEquals(CallerAccess.CONTROL, CallerPolicy.decide(facts(pkg = "com.example.watch", trusted = true)))
    }

    @Test
    fun unknownCallersAreRejected() {
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(facts(pkg = "com.example.anything")))
        assertEquals(
            "Android Auto's certificate on another package proves nothing",
            CallerAccess.REJECT,
            CallerPolicy.decide(facts(pkg = "com.example.anything", certs = setOf(AUTO_CERT))),
        )
    }

    @Test
    fun theKnownListHoldsReleaseKeysOnly() {
        // SHA-256: 32 bytes, lower-case hex pairs.
        val shape = Regex("^([0-9a-f]{2}:){31}[0-9a-f]{2}$")
        for (k in CallerPolicy.KNOWN) {
            assertTrue(k.certificates.isNotEmpty())
            k.certificates.forEach { assertTrue(it, shape.matches(it)) }
        }
        // The widely shared debug key is never trusted.
        val debug = "19:75:b2:f1:71:77:bc:89:a5:df:f3:1f:9e:64:a6:ca:e2:81:a5:3d:c1:d1:d5:9b:1d:14:7f:e1:c8:2a:fa:00"
        assertFalse(CallerPolicy.KNOWN.any { debug in it.certificates })
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(facts(certs = setOf(debug))))
    }

    @Test
    fun digestsAreFormattedAsListed() {
        assertEquals(
            "ba:78:16:bf:8f:01:cf:ea:41:41:40:de:5d:ae:22:23:b0:03:61:a3:96:17:7a:9c:b4:10:ff:61:f2:00:15:ad",
            sha256Hex("abc".toByteArray()),
        )
    }
}

/** The facts come from Android's package manager: the uid's packages and the certificates. */
@RunWith(AndroidJUnit4::class)
@Config(sdk = [35])
class CallerFactsReaderTest {
    private val context = ApplicationProvider.getApplicationContext<android.app.Application>()
    private val pm = shadowOf(context.packageManager)

    private fun install(pkg: String, uid: Int, cert: ByteArray) {
        @Suppress("DEPRECATION")
        val info = PackageInfo().apply {
            packageName = pkg
            signatures = arrayOf(Signature(cert))
            applicationInfo = android.content.pm.ApplicationInfo().apply {
                packageName = pkg
                this.uid = uid
            }
        }
        pm.installPackage(info)
        pm.setPackagesForUid(uid, pkg)
    }

    @Test
    fun aCallerSignedWithAKnownCertificateIsRecognised() {
        val cert = byteArrayOf(1, 2, 3, 4, 5)
        install(AUTO, 10_200, cert)
        val f = CallerFactsReader(context).read(AUTO, 10_200, media3Trusted = false)
        assertTrue(f.packageMatchesUid)
        assertTrue(sha256Hex(cert) in f.certificates)
        val known = listOf(KnownCaller(AUTO, "Android Auto", setOf(sha256Hex(cert))))
        assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(f, known))
        // The real list doesn't contain this made-up certificate.
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(f))
    }

    @Test
    fun aPackageNameClaimedByAnotherUidIsRefused() {
        install(AUTO, 10_201, byteArrayOf(9))
        val f = CallerFactsReader(context).read(AUTO, 10_999, media3Trusted = true)
        assertFalse(f.packageMatchesUid)
        assertTrue("nothing is read for a mismatched caller", f.certificates.isEmpty())
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(f))
    }

    @Test
    fun anUnknownPackageIsRefused() {
        val f = CallerFactsReader(context).read("com.example.nothing", 10_300, media3Trusted = false)
        assertEquals(CallerAccess.REJECT, CallerPolicy.decide(f))
    }

    @Test
    fun thisAppIsItself() {
        val f = CallerFactsReader(context).read(context.packageName, android.os.Process.myUid(), media3Trusted = true)
        assertTrue(f.packageMatchesUid)
        assertTrue(f.self)
        assertEquals(CallerAccess.BROWSE, CallerPolicy.decide(f))
    }
}
