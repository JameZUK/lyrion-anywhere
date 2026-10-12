package io.github.jamezuk.lyrionanywhere.data

import io.github.jamezuk.lyrionanywhere.core.PairingInvite
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Pairing invites (scanned, typed or from a link) reach the app already checked by the
 * Rust layer, which allows plain HTTP to loopback for development. Release builds take
 * https alone: a link can't point a release build at a server on the phone itself.
 */
class PlainHttpPolicyTest {
    private fun invite(server: String?) = PairingInvite(server, "ABCDEFGHJK")

    @Test
    fun releaseBuildsRefuseLoopbackHttp() {
        for (s in listOf("http://127.0.0.1:9443/", "http://localhost/", "http://[::1]:8080/", "HTTP://127.0.0.1/")) {
            assertTrue(s, refusesPlainHttp(invite(s), allowPlainLoopback = false))
            assertFalse(s, refusesPlainHttp(invite(s), allowPlainLoopback = true))
        }
    }

    @Test
    fun httpsAndCodesWithoutAServerAreFine() {
        for (allow in listOf(true, false)) {
            assertFalse(refusesPlainHttp(invite("https://music.example.com/"), allow))
            assertFalse(refusesPlainHttp(invite(null), allow))
        }
    }

    @Test
    fun theRefusalSaysWhy() {
        assertTrue(PLAIN_HTTP_REFUSED.detail.startsWith("use https://"))
    }
}
