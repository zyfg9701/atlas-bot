package com.atlasbot.client.auth

import android.content.Context
import android.content.SharedPreferences
import androidx.security.crypto.EncryptedSharedPreferences
import androidx.security.crypto.MasterKeys

/** Secure local ticket store. Logout must [clear]. Optional [provider] is informational. */
interface TokenStore {
    fun saveBearer(token: String, provider: String? = null)
    fun loadBearer(): String?
    fun loadProvider(): String?
    fun clear()
}

/** In-memory store for JVM unit tests / previews. */
class MemoryTokenStore : TokenStore {
    @Volatile private var bearer: String? = null
    @Volatile private var provider: String? = null
    override fun saveBearer(token: String, provider: String?) {
        bearer = token
        this.provider = provider
    }
    override fun loadBearer(): String? = bearer
    override fun loadProvider(): String? = provider
    override fun clear() {
        bearer = null
        provider = null
    }
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

    override fun saveBearer(token: String, provider: String?) {
        prefs.edit()
            .putString(KEY_BEARER, token)
            .apply {
                if (provider != null) putString(KEY_PROVIDER, provider)
                else remove(KEY_PROVIDER)
            }
            .apply()
    }

    override fun loadBearer(): String? = prefs.getString(KEY_BEARER, null)

    override fun loadProvider(): String? = prefs.getString(KEY_PROVIDER, null)

    override fun clear() {
        prefs.edit().remove(KEY_BEARER).remove(KEY_PROVIDER).apply()
    }

    companion object {
        private const val KEY_BEARER = "hub_bearer"
        private const val KEY_PROVIDER = "ticket_provider"
    }
}
