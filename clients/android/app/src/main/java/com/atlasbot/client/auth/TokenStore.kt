package com.atlasbot.client.auth

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKeys

/** Secure local ticket store. Logout must [clear]. */
interface TokenStore {
    fun saveBearer(token: String)
    fun loadBearer(): String?
    fun clear()
}

/** In-memory store for JVM unit tests / previews. */
class MemoryTokenStore : TokenStore {
    @Volatile private var bearer: String? = null
    override fun saveBearer(token: String) { bearer = token }
    override fun loadBearer(): String? = bearer
    override fun clear() { bearer = null }
}

/**
 * EncryptedSharedPreferences (Jetpack Security / Keystore-backed).
 * Falls back to private SharedPreferences if crypto init fails (emulator edge cases).
 */
class EncryptedPrefsTokenStore(context: Context) : TokenStore {
    private val prefs: SharedPreferences = try {
        val masterKeyAlias = MasterKeys.getOrCreate(MasterKeys.AES256_GCM_SPEC)
        EncryptedSharedPreferences.create(
            "atlas_bot_auth_enc",
            masterKeyAlias,
            context,
            EncryptedSharedPreferences.PrefKeyEncryptionScheme.AES256_SIV,
            EncryptedSharedPreferences.PrefValueEncryptionScheme.AES256_GCM,
        )
    } catch (_: Exception) {
        context.getSharedPreferences("atlas_bot_auth_fallback", Context.MODE_PRIVATE)
    }

    override fun saveBearer(token: String) {
        prefs.edit().putString(KEY_BEARER, token).apply()
    }

    override fun loadBearer(): String? = prefs.getString(KEY_BEARER, null)

    override fun clear() {
        prefs.edit().remove(KEY_BEARER).apply()
    }

    companion object {
        private const val KEY_BEARER = "hub_bearer"
    }
}
