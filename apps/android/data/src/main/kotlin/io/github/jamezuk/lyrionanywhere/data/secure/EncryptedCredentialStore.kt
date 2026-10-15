package io.github.jamezuk.lyrionanywhere.data.secure

import android.security.keystore.KeyPermanentlyInvalidatedException
import io.github.jamezuk.lyrionanywhere.core.SecureStore
import io.github.jamezuk.lyrionanywhere.core.StoreException
import io.github.jamezuk.lyrionanywhere.core.StoredCredentials
import java.nio.ByteBuffer
import java.security.GeneralSecurityException
import javax.crypto.BadPaddingException
import javax.crypto.Cipher
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/** Seals and opens small secrets. */
interface TokenCipher {
    fun seal(plain: ByteArray): ByteArray

    /**
     * Throws [javax.crypto.AEADBadTagException] or [BlobFormatException] if the blob was
     * tampered with or the key changed; anything else means the Keystore couldn't be used
     * just now (the blob is kept).
     */
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
            throw BlobFormatException()
        }
        val nonceLen = sealed[1].toInt()
        if (nonceLen !in 12..16 || sealed.size < 2 + nonceLen + TAG_BYTES) {
            throw BlobFormatException()
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

/** What [EncryptedCredentialStore.read] found. */
sealed interface SavedCredentials {
    data class Present(val credentials: StoredCredentials) : SavedCredentials

    /** Nothing saved (or what was saved could never be opened, and has been deleted). */
    data object Absent : SavedCredentials

    /**
     * Couldn't be read just now (the Keystore restarting, an I/O error). The blob is kept so
     * a later start can try again. [reason] is an exception class name, never a message.
     */
    data class Unavailable(val reason: String) : SavedCredentials
}

/** The sealed blob isn't in a format this app wrote. */
class BlobFormatException : GeneralSecurityException("unknown format")

/**
 * The app's [SecureStore]: the refresh token and device id, sealed with [cipher]. Nothing
 * else is stored, and nothing is ever logged.
 *
 * A blob that can never be opened (tampered with, or the Keystore key is gone after a restore
 * to another device) is deleted and reads as "not signed in", so the user simply signs in
 * again. A Keystore or file that fails for any other reason keeps the blob: see [read].
 */
class EncryptedCredentialStore(
    private val cipher: TokenCipher,
    private val storage: BlobStorage,
) : SecureStore {
    /**
     * Read the saved sign-in. Only a definite integrity failure (the tag doesn't match, so the
     * blob was tampered with or the key is a different one; a format this app never wrote; a
     * key Android has permanently invalidated; contents that don't decode) deletes the blob.
     * Every other failure leaves it alone and reads as [SavedCredentials.Unavailable].
     */
    fun read(): SavedCredentials {
        val blob = try {
            storage.read()
        } catch (e: Exception) {
            return SavedCredentials.Unavailable(e::class.java.simpleName)
        } ?: return SavedCredentials.Absent
        val plain = try {
            cipher.open(blob)
        } catch (e: Exception) {
            if (!isIntegrityFailure(e)) return SavedCredentials.Unavailable(e::class.java.simpleName)
            null
        }
        val credentials = plain?.let(::decode)
        if (credentials == null) {
            // Never readable: forget it so the user can sign in again. If even the delete
            // fails, the next start finds the same blob and tries again.
            runCatching { storage.delete() }
            return SavedCredentials.Absent
        }
        return SavedCredentials.Present(credentials)
    }

    /** As [read]; "unavailable" is a [StoreException.Failed], with the blob kept. */
    override fun load(): StoredCredentials? = when (val r = read()) {
        is SavedCredentials.Present -> r.credentials
        SavedCredentials.Absent -> null
        is SavedCredentials.Unavailable -> throw StoreException.Failed("$UNAVAILABLE (${r.reason})")
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
        const val UNAVAILABLE = "secure storage unavailable"

        /** The blob can never be opened (as opposed to the Keystore being busy). */
        fun isIntegrityFailure(e: Exception): Boolean =
            e is BadPaddingException || e is BlobFormatException || e is KeyPermanentlyInvalidatedException

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

/**
 * What this store holds, telling "nothing saved" from "can't be read just now" (a store whose
 * [SecureStore.load] throws).
 */
fun SecureStore.savedSignIn(): SavedCredentials = when (this) {
    is EncryptedCredentialStore -> read()
    else -> try {
        load()?.let { SavedCredentials.Present(it) } ?: SavedCredentials.Absent
    } catch (e: Exception) {
        SavedCredentials.Unavailable(e::class.java.simpleName)
    }
}
