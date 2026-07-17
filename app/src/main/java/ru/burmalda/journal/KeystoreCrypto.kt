package com.burmalda57.crypto

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

object KeystoreCrypto {
    private const val ALIAS = "burmalda57_master_key_v2"
    private const val TRANSFORM = "AES/GCM/NoPadding"
    private const val IV_LEN = 12
    private const val TAG_BITS = 128

    // Теперь метод принимает контекст, чтобы проверить наличие StrongBox
    private fun getOrCreateKey(context: Context? = null): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        
        if (ks.containsAlias(ALIAS)) {
            val entry = ks.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry
            if (entry != null) {
                return entry.secretKey
            }
        }

        val kg = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        val builder = KeyGenParameterSpec.Builder(
            ALIAS,
            KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT
        )
            .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
            .setKeySize(256)

        // Проверяем StrongBox через переданный контекст безопасным способом
        val hasStrongBox = context?.packageManager?.hasSystemFeature(
            android.content.pm.PackageManager.FEATURE_STRONGBOX_KEYSTORE
        ) ?: false

        if (hasStrongBox) {
            try {
                kg.init(builder.setIsStrongBoxBacked(true).build())
                return kg.generateKey()
            } catch (e: Exception) {
                android.util.Log.e("burmalda57", "StrongBox упал, уходим на TEE: ${e.message}")
            }
        }

        kg.init(builder.setIsStrongBoxBacked(false).build())
        return kg.generateKey()
    }

    // Метод для явной инициализации ключа при старте приложения
    fun init(context: Context) {
        getOrCreateKey(context)
    }

    @JvmStatic
    fun wrap(plain: ByteArray): ByteArray {
        val c = Cipher.getInstance(TRANSFORM)
        c.init(Cipher.ENCRYPT_MODE, getOrCreateKey(null))
        val iv = c.iv
        return iv + c.doFinal(plain)
    }

    @JvmStatic
    fun unwrap(blob: ByteArray): ByteArray {
        if (blob.size < IV_LEN + (TAG_BITS / 8)) throw IllegalArgumentException("Bad encrypted DEK blob")
        val iv = blob.copyOfRange(0, IV_LEN)
        val ct = blob.copyOfRange(IV_LEN, blob.size)
        val c = Cipher.getInstance(TRANSFORM)
        c.init(Cipher.DECRYPT_MODE, getOrCreateKey(null), GCMParameterSpec(TAG_BITS, iv))
        return c.doFinal(ct)
    }
}