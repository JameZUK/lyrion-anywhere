package io.github.jamezuk.lyrionanywhere.data.secure

import java.security.KeyStore
import java.security.KeyStoreException
import java.security.ProviderException
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertThrows
import org.junit.Test

/**
 * M6: a new key is made only when there is genuinely none. A Keystore that throws, or that
 * says the alias exists but won't hand the key over, must not get a replacement key: that
 * would make the saved sign-in unreadable for good.
 */
class KeystoreKeyProviderTest {
    private fun newKey(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    @Test
    fun anExistingKeyIsUsed() {
        val key = newKey()
        var made = 0
        val got = KeystoreKeyProvider.resolveKey({ KeyStore.SecretKeyEntry(key) }, { true }, { made++; newKey() })
        assertSame(key, got)
        assertEquals(0, made)
    }

    @Test
    fun noKeyAtAllMakesOne() {
        val fresh = newKey()
        assertSame(fresh, KeystoreKeyProvider.resolveKey({ null }, { false }, { fresh }))
    }

    @Test
    fun aKeystoreErrorNeverMakesAReplacement() {
        var made = 0
        for (failure in listOf(KeyStoreException("busy"), ProviderException("restarting"))) {
            val e = assertThrows(Exception::class.java) {
                KeystoreKeyProvider.resolveKey({ throw failure }, { true }, { made++; newKey() })
            }
            assertSame(failure, e)
        }
        assertEquals(0, made)
    }

    @Test
    fun anAliasThatWontYieldItsKeyIsTransient() {
        var made = 0
        assertThrows(KeyStoreException::class.java) {
            KeystoreKeyProvider.resolveKey({ null }, { true }, { made++; newKey() })
        }
        assertEquals(0, made)
    }
}
