package io.github.jamezuk.lyrionanywhere.data.secure

import io.github.jamezuk.lyrionanywhere.core.StoreException
import io.github.jamezuk.lyrionanywhere.core.StoredCredentials
import java.security.GeneralSecurityException
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertThrows
import org.junit.Assert.assertTrue
import org.junit.Test

class EncryptedCredentialStoreTest {
    private fun newKey(): SecretKey = KeyGenerator.getInstance("AES").apply { init(256) }.generateKey()

    private class MemoryBlob : BlobStorage {
        var bytes: ByteArray? = null
        var failWrites = false

        override fun read() = bytes?.copyOf()

        override fun write(bytes: ByteArray) {
            if (failWrites) throw java.io.IOException("disk full")
            this.bytes = bytes.copyOf()
        }

        override fun delete() {
            bytes = null
        }
    }

    private val creds = StoredCredentials(refreshToken = "r3fr3sh-T0ken_abc.~", deviceId = 42)

    @Test
    fun roundTripsAndStoresNoPlaintext() {
        val blob = MemoryBlob()
        val key = newKey()
        val store = EncryptedCredentialStore(AesGcmCipher { key }, blob)
        assertNull(store.load())
        store.save(creds)
        val raw = blob.bytes!!
        assertFalse(
            "the token must not be stored in clear",
            String(raw, Charsets.ISO_8859_1).contains(creds.refreshToken),
        )
        assertEquals(creds, store.load())
        // A fresh nonce every time: two seals of the same data differ.
        store.save(creds)
        assertFalse(raw.contentEquals(blob.bytes!!))
        assertEquals(creds, store.load())
    }

    @Test
    fun tamperingIsDetectedAndTheBlobDiscarded() {
        val blob = MemoryBlob()
        val key = newKey()
        val store = EncryptedCredentialStore(AesGcmCipher { key }, blob)
        store.save(creds)
        val original = blob.bytes!!
        for (i in original.indices) {
            val bad = original.copyOf()
            bad[i] = (bad[i].toInt() xor 0x01).toByte()
            blob.bytes = bad
            assertNull("flipped byte $i was accepted", store.load())
            assertNull("tampered blob not deleted", blob.bytes)
        }
        // Truncated or empty blobs too.
        for (n in listOf(0, 1, 2, 13, original.size - 1)) {
            blob.bytes = original.copyOf(n)
            assertNull(store.load())
        }
    }

    @Test
    fun anotherKeyCantOpenIt() {
        val blob = MemoryBlob()
        val mine = newKey()
        EncryptedCredentialStore(AesGcmCipher { mine }, blob).save(creds)
        val other = newKey()
        assertNull(EncryptedCredentialStore(AesGcmCipher { other }, blob).load())
        assertNull(blob.bytes)
    }

    @Test
    fun clearRemovesIt() {
        val blob = MemoryBlob()
        val key = newKey()
        val store = EncryptedCredentialStore(AesGcmCipher { key }, blob)
        store.save(creds)
        store.clear()
        assertNull(blob.bytes)
        assertNull(store.load())
    }

    @Test
    fun writeFailuresAreReportedWithoutDetail() {
        val blob = MemoryBlob().apply { failWrites = true }
        val key = newKey()
        val store = EncryptedCredentialStore(AesGcmCipher { key }, blob)
        val e = assertThrows(StoreException.Failed::class.java) { store.save(creds) }
        assertFalse(e.detail.contains(creds.refreshToken))
    }

    @Test
    fun theEncodingIsStrict() {
        val good = EncryptedCredentialStore.encode(creds)
        assertEquals(creds, EncryptedCredentialStore.decode(good))
        for (bad in listOf("", "\n", "42", "42\n", "x\nabc", "0\nabc", "-1\nabc", "42\na b", "42\nab\u0000c", "42\né")) {
            assertNull(bad, EncryptedCredentialStore.decode(bad.toByteArray()))
        }
        assertThrows(IllegalArgumentException::class.java) {
            EncryptedCredentialStore.encode(StoredCredentials("a\nb", 1))
        }
    }

    @Test
    fun cipherRejectsUnknownFormats() {
        val key = newKey()
        val cipher = AesGcmCipher { key }
        val sealed = cipher.seal("hello".toByteArray())
        assertArrayEquals("hello".toByteArray(), cipher.open(sealed))
        val wrongVersion = sealed.copyOf().also { it[0] = 2 }
        assertThrows(GeneralSecurityException::class.java) { cipher.open(wrongVersion) }
        val wrongNonce = sealed.copyOf().also { it[1] = 99 }
        assertThrows(GeneralSecurityException::class.java) { cipher.open(wrongNonce) }
        assertTrue(sealed.size >= 2 + 12 + 16 + 5)
    }
}
