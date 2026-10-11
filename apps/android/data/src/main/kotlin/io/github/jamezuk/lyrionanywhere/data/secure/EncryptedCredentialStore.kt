package io.github.jamezuk.lyrionanywhere.data.secure

import io.github.jamezuk.lyrionanywhere.core.SecureStore
import io.github.jamezuk.lyrionanywhere.core.StoreException
import io.github.jamezuk.lyrionanywhere.core.StoredCredentials
import java.nio.ByteBuffer
import java.security.GeneralSecurityException
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Seals and opens small secrets. */
interface TokenCipher {
    fun seal(plain: ByteArray): ByteArray

    /** Throws [GeneralSecurityException] if the blob was tampered with or the key changed. */
    fun open(sealed: ByteArray): ByteArray
}

/**
 * AES-256-GCM with a random 96-bit nonce per message, bound to a fixed context string
 * (associated data) so a blob can't be reused for anything else.
 *
 * Blob format: `[version=1][nonce length][nonce][ciphertext + 128-bit tag]`.
 *
 * On a device the key lives in the AndroidKeyStore ([KeystoreKeyProvider]) and never leaves
 * it; in tests any AES key will do.
 */
class AesGcmCipher(private val key: () -> SecretKey) : TokenCipher {
    override fun seal(plain: ByteArray): ByteArray {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        // The provider picks the nonce (the AndroidKeyStore insists on it).
        cipher.init(Cipher.ENCRYPT_MODE, key())
        cipher.updateAAD(CONTEXT)
        val nonce = cipher.iv
        check(nonce.size in 12..16) { "unexpected nonce size" }
        val body = cipher.doFinal(plain)
        return ByteBuffer.allocate(2 + nonce.size + body.size)
            .put(VERSION)
            .put(nonce.size.toByte())
            .put(nonce)
            .put(body)
            .array()
    }

    override fun open(sealed: ByteArray): ByteArray {
        if (sealed.size < 2 + 12 + TAG_BYTES || sealed[0] != VERSION) {
            throw GeneralSecurityException("unknown format")
        }
        val nonceLen = sealed[1].toInt()
        if (nonceLen !in 12..16 || sealed.size < 2 + nonceLen + TAG_BYTES) {
            throw GeneralSecurityException("unknown format")
        }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(
            Cipher.DECRYPT_MODE,
            key(),
            GCMParameterSpec(TAG_BYTES * 8, sealed, 2, nonceLen),
        )
        cipher.updateAAD(CONTEXT)
        return cipher.doFinal(sealed, 2 + nonceLen, sealed.size - 2 - nonceLen)
    }

    private companion object {
        const val TRANSFORMATION = "AES/GCM/NoPadding"
        const val VERSION: Byte = 1
        const val TAG_BYTES = 16
        val CONTEXT = "lyrion-anywhere/refresh-token/v1".toByteArray()
    }
}

/** Where the sealed blob is kept (a file excluded from backups, on a device). */
interface BlobStorage {
    fun read(): ByteArray?

    fun write(bytes: ByteArray)

    fun delete()
}

/**
 * The app's [SecureStore]: the refresh token and device id, sealed with [cipher]. Nothing
 * else is stored, and nothing is ever logged.
 *
 * A blob that can't be opened (tampered with, or the Keystore key is gone after a restore to
 * another device) is deleted and reads as "not signed in", so the user simply signs in again.
 */
class EncryptedCredentialStore(
    private val cipher: TokenCipher,
    private val storage: BlobStorage,
) : SecureStore {
    override fun load(): StoredCredentials? {
        val blob = try {
            storage.read()
        } catch (_: Exception) {
            null
        } ?: return null
        val credentials = try {
            decode(cipher.open(blob))
        } catch (_: Exception) {
            null
        }
        if (credentials == null) clear()
        return credentials
    }

    override fun save(credentials: StoredCredentials) {
        try {
            storage.write(cipher.seal(encode(credentials)))
        } catch (e: Exception) {
            throw StoreException.Failed(e::class.java.simpleName)
        }
    }

    override fun clear() {
        try {
            storage.delete()
        } catch (e: Exception) {
            throw StoreException.Failed(e::class.java.simpleName)
        }
    }

    internal companion object {
        private const val MAX_TOKEN = 1024

        fun encode(c: StoredCredentials): ByteArray {
            require(validToken(c.refreshToken)) { "bad token" }
            return "${c.deviceId}\n${c.refreshToken}".toByteArray(Charsets.US_ASCII)
        }

        fun decode(bytes: ByteArray): StoredCredentials? {
            val text = bytes.toString(Charsets.US_ASCII)
            val nl = text.indexOf('\n')
            if (nl <= 0) return null
            val id = text.substring(0, nl).toLongOrNull()?.takeIf { it > 0 } ?: return null
            val token = text.substring(nl + 1).takeIf(::validToken) ?: return null
            return StoredCredentials(refreshToken = token, deviceId = id)
        }

        private fun validToken(t: String) =
            t.isNotEmpty() && t.length <= MAX_TOKEN && t.all { it in '!'..'~' }
    }
}
