package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Plain `http://` servers (docs/plans/trusted-networks.md). Addresses reach the app already
 * checked by the Rust layer (https, or http to loopback or to a literal private address).
 * This build then takes loopback only in debug builds, a private address only after the user
 * confirms, and nothing else over http, typed, scanned or from a link.
 */
class PlainHttpPolicyTest {
    private fun invite(server: String?) = PairingInvite(server, "ABCDEFGHJK")

    private val loopback = listOf("http://127.0.0.1:9443/", "http://localhost/", "http://[::1]:8080/", "HTTP://127.0.0.1/", "http://127.8.9.10/")
    private val privateLiterals = listOf(
        "http://10.8.0.1:9443/",
        "http://10.0.0.10/",
        "http://172.16.0.1:9000/",
        "http://172.31.255.254/",
        "http://100.64.0.7:9443/",
        "http://169.254.1.1/",
        "http://[fd12:3456:789a::1]:9443/",
        "http://[fe80::1]/",
        "HTTP://10.8.0.1/",
    )
    private val neverOverHttp = listOf(
        // Names: they could resolve anywhere.
        "http://music.example.com/",
        "http://music.example.com:9443/",
        "http://10.8.0.1.nip.io/",
        "http://localhost.example.com/",
        // Public and special addresses.
        "http://203.0.113.10/",
        "http://8.8.8.8:9443/",
        "http://0.0.0.0/",
        "http://172.32.0.1/",
        "http://100.128.0.1/",
        "http://[2001:db8::1]/",
        "http://[::ffff:a00:1]/",
        "http://[::ffff:10.0.0.1]/",
        "http://[::]/",
        // Not an address the Rust layer would ever return.
        "http://user@10.8.0.1/",
        "http://10.8.0.1@music.example.com/",
        "http://10.8.0.1:99999999/",
        "http://10.8.0.1:x/",
        "http://010.8.0.1/",
        "http://10.8.0/",
        "http://10.8.0.1.5/",
        "http://[fd00::1/",
        "http://[fd00::1::2]/",
        "http://[fd00::1%25eth0]/",
        "http://",
        "http:///",
        "http:10.8.0.1",
        "ws://10.8.0.1/",
        "ftp://10.8.0.1/",
        "",
    )

    @Test
    fun loopbackOverHttpOnlyInDebugBuilds() {
        for (s in loopback) {
            assertEquals(s, ServerTransport.LOOPBACK, serverTransport(s))
            assertTrue(s, refusesPlainHttp(invite(s), allowPlainLoopback = false))
            assertFalse(s, refusesPlainHttp(invite(s), allowPlainLoopback = true))
            // Confirming doesn't make loopback acceptable in a release build.
            assertEquals(s, PLAIN_HTTP_REFUSED, plainHttpRefusal(s, allowPlainLoopback = false, confirmedUnencrypted = true))
            assertNull(s, plainHttpRefusal(s, allowPlainLoopback = true, confirmedUnencrypted = false))
        }
    }

    @Test
    fun aPrivateAddressOverHttpNeedsTheUsersConfirmation() {
        for (s in privateLiterals) {
            assertEquals(s, ServerTransport.PRIVATE_NETWORK, serverTransport(s))
            for (debug in listOf(true, false)) {
                // An invite is read (pairing asks first)...
                assertFalse(s, refusesPlainHttp(invite(s), debug))
                // ...and refused until confirmed, then allowed.
                val unconfirmed = plainHttpRefusal(s, debug, confirmedUnencrypted = false)
                assertEquals(s, UNENCRYPTED_UNCONFIRMED, unconfirmed)
                assertTrue(s, unconfirmed!!.needsUnencryptedConfirmation())
                assertNull(s, plainHttpRefusal(s, debug, confirmedUnencrypted = true))
            }
        }
    }

    @Test
    fun namesAndPublicAddressesAreNeverTakenOverHttp() {
        for (s in neverOverHttp) {
            assertEquals(s, ServerTransport.REFUSED, serverTransport(s))
            for (debug in listOf(true, false)) {
                for (confirmed in listOf(true, false)) {
                    assertEquals(s, PLAIN_HTTP_REFUSED, plainHttpRefusal(s, debug, confirmed))
                    assertFalse(s, PLAIN_HTTP_REFUSED.needsUnencryptedConfirmation())
                }
                assertTrue(s, s.isEmpty() || refusesPlainHttp(invite(s), debug))
            }
        }
    }

    @Test
    fun httpsAndCodesWithoutAServerAreFine() {
        for (allow in listOf(true, false)) {
            for (s in listOf("https://music.example.com/", "https://10.0.0.10/", "HTTPS://music.example.com:8443/")) {
                assertEquals(s, ServerTransport.ENCRYPTED, serverTransport(s))
                assertFalse(refusesPlainHttp(invite(s), allow))
                assertNull(plainHttpRefusal(s, allow, confirmedUnencrypted = false))
            }
            assertFalse(refusesPlainHttp(invite(null), allow))
        }
    }

    @Test
    fun theRefusalsSayWhy() {
        // Worded by the app (its resources), not this text.
        assertEquals(AppError.Reason.PLAIN_HTTP, PLAIN_HTTP_REFUSED.reason)
        assertEquals(AppError.Reason.CONFIRM_UNENCRYPTED, UNENCRYPTED_UNCONFIRMED.reason)
        assertFalse(AppError.Offline().needsUnencryptedConfirmation())
        assertFalse(AppError.InvalidInput("confirm unencrypted").needsUnencryptedConfirmation())
    }

    /**
     * The shared vectors (web/app/tests/fixtures/private-networks.json), also run against
     * `core::privnet`, the plugin's Perl check and the web app's JavaScript check.
     */
    @Test
    fun theSharedPrivateAddressVectors() {
        val json = File("../../../web/app/tests/fixtures/private-networks.json").readText()
        fun list(key: String): List<String> {
            val body = Regex("\"$key\"\\s*:\\s*\\[([^\\]]*)]").find(json)?.groupValues?.get(1)
                ?: error("no $key in the vectors")
            return Regex("\"([^\"]*)\"").findAll(body).map { it.groupValues[1] }.toList()
        }
        fun url(a: String) = if (':' in a) "http://[$a]:9443/" else "http://$a:9443/"
        val private = list("private")
        val notPrivate = list("not_private")
        assertTrue(private.size >= 10 && notPrivate.size >= 10)
        for (a in private) assertEquals(a, ServerTransport.PRIVATE_NETWORK, serverTransport(url(a)))
        for (a in notPrivate) {
            val t = serverTransport(url(a))
            val loopbackAddress = a == "127.0.0.1" || a == "::1"
            assertEquals(a, if (loopbackAddress) ServerTransport.LOOPBACK else ServerTransport.REFUSED, t)
        }
    }

    @Test
    fun arbitraryInputNeverThrowsAndNeverTrustsANameOverHttp() {
        val alphabet = "ab:/@?#%[]\\. 0123456789fdeFE-xX\u0000‮hHtTpPsS:é".toList()
        var seed = 0x2545F4914F6CDD1DL
        repeat(20_000) {
            val len = ((seed ushr 1) % 40).toInt()
            val sb = StringBuilder(if (it % 2 == 0) "http://" else "")
            repeat(len) {
                seed = seed xor (seed shl 13)
                seed = seed xor (seed ushr 7)
                seed = seed xor (seed shl 17)
                sb.append(alphabet[((seed ushr 1) % alphabet.size).toInt()])
            }
            val s = sb.toString()
            val t = serverTransport(s)
            if (t == ServerTransport.PRIVATE_NETWORK || t == ServerTransport.LOOPBACK) {
                val host = s.substringAfter("://").substringBefore('/').let { a ->
                    if (a.startsWith('[')) a.substring(1, a.indexOf(']')) else a.substringBefore(':')
                }
                // Only an address literal (or "localhost") is ever let through over http.
                assertTrue(s, host.equals("localhost", true) || host.all { c -> c == '.' || c == ':' || c.isDigit() || c.lowercaseChar() in 'a'..'f' })
                assertFalse(s, s.contains('@'))
            }
            plainHttpRefusal(s, allowPlainLoopback = false, confirmedUnencrypted = true)
            refusesPlainHttp(invite(s), allowPlainLoopback = false)
        }
    }
}
