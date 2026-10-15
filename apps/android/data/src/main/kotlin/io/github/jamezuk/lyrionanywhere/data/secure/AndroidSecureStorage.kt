package io.github.jamezuk.lyrionanywhere.data.secure

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.AtomicFile
import java.io.File
import java.io.FileNotFoundException
import java.security.KeyStore
import java.security.KeyStoreException
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey

/**
 * An AES-256-GCM key generated inside the AndroidKeyStore. It can't be exported, so the
 * sealed refresh token is useless off this device (and backups exclude it anyway).
 */
class KeystoreKeyProvider(private val alias: String = ALIAS) : () -> SecretKey {
    @Synchronized
    override fun invoke(): SecretKey {
        val ks = KeyStore.getInstance(ANDROID_KEYSTORE).apply { load(null) }
        return resolveKey(
            entry = { ks.getEntry(alias, null) },
            hasAlias = { ks.containsAlias(alias) },
            generate = ::generate,
        )
    }

    private fun generate(): SecretKey {
        val spec = KeyGenParameterSpec.Builder(
            alias,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)
            .setRandomizedEncryptionRequired(true)
            .build()
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, ANDROID_KEYSTORE)
            .apply { init(spec) }
            .generateKey()
    }

    internal companion object {
        /**
         * The key under the alias, or a new one only when there is genuinely none. A Keystore
         * that throws (its daemon restarting, say) is passed on, and an alias that exists but
         * won't yield its key is a [KeyStoreException]: a replacement key in either case would
         * leave the saved sign-in unreadable for good.
         */
        fun resolveKey(
            entry: () -> KeyStore.Entry?,
            hasAlias: () -> Boolean,
            generate: () -> SecretKey,
        ): SecretKey {
            val found = entry()
            (found as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
            if (found != null || hasAlias()) throw KeyStoreException("key entry unavailable")
            return generate()
        }

        const val ANDROID_KEYSTORE = "AndroidKeyStore"
        const val ALIAS = "lyrion-anywhere.refresh-token"
    }
}

/** A file in the app's no-backup directory, replaced atomically. */
class NoBackupFileStorage(context: Context, name: String = "credentials.bin") : BlobStorage {
    private val file = AtomicFile(File(context.noBackupFilesDir, name))

    override fun read(): ByteArray? = try {
        file.readFully()
    } catch (_: FileNotFoundException) {
        null
    }

    override fun write(bytes: ByteArray) {
        val out = file.startWrite()
        try {
            out.write(bytes)
            file.finishWrite(out)
        } catch (e: Exception) {
            file.failWrite(out)
            throw e
        }
    }

    override fun delete() {
        file.delete()
    }
}
