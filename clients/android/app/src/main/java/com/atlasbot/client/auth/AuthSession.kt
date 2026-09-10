package com.atlasbot.client.auth

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference

/**
 * M1 Android login: Chrome Custom Tabs + PKCE S256 (AppAuth-equivalent shape).
 * Primary path is **system browser / Custom Tabs** — not an in-app WebView.
 *
 * Redirect: [MOBILE_REDIRECT_URI] (`atlasbot://auth/callback`).
 */
class AuthSession(
    private val tokenStore: TokenStore,
    private val ioExecutor: Executor = Executors.newSingleThreadExecutor(),
) {
    data class Pending(
        val cfg: OidcClientConfig,
        val redirectUri: String,
        val state: String,
        val pkce: PkcePair,
    )

    private val pending = AtomicReference<Pending?>(null)
    @Volatile private var callbackListener: ((Result<String>) -> Unit)? = null

    fun hasTicket(): Boolean = !tokenStore.loadBearer().isNullOrBlank()
    fun currentBearer(): String? = tokenStore.loadBearer()

    fun saveManualBearer(token: String) {
        tokenStore.saveBearer(token.trim())
    }

    fun logout() {
        pending.set(null)
        tokenStore.clear()
    }

    /**
     * Start OIDC authorize in a Custom Tab. Completion arrives via [handleCallbackUri]
     * after the IdP redirects to [MOBILE_REDIRECT_URI].
     */
    fun startLogin(
        context: Context,
        cfg: OidcClientConfig,
        redirectUri: String = MOBILE_REDIRECT_URI,
        onComplete: (Result<String>) -> Unit,
    ) {
        callbackListener = onComplete
        val pkce = generatePkce()
        val state = randomState()
        pending.set(Pending(cfg, redirectUri, state, pkce))
        val url = buildAuthorizeUrl(cfg, redirectUri, state, pkce)
        val intent = CustomTabsIntent.Builder().build()
        intent.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        intent.launchUrl(context, Uri.parse(url))
    }

    /** Called from MainActivity when `atlasbot://auth/callback` arrives. */
    fun handleCallbackUri(uri: Uri, onMain: (Result<String>) -> Unit) {
        val p = pending.getAndSet(null)
        if (p == null) {
            onMain(Result.failure(IllegalStateException("no pending login")))
            return
        }
        val parsed = parseCallbackUrl(uri.toString())
        if (!parsed.error.isNullOrEmpty()) {
            val err = Result.failure<String>(IllegalStateException(parsed.error))
            callbackListener?.invoke(err)
            callbackListener = null
            onMain(err)
            return
        }
        if (parsed.code.isNullOrEmpty()) {
            val err = Result.failure<String>(IllegalStateException("missing code"))
            callbackListener?.invoke(err)
            callbackListener = null
            onMain(err)
            return
        }
        if (parsed.state != null && parsed.state != p.state) {
            val err = Result.failure<String>(IllegalStateException("state mismatch"))
            callbackListener?.invoke(err)
            callbackListener = null
            onMain(err)
            return
        }
        ioExecutor.execute {
            try {
                val tr = exchangeCode(p.cfg, p.redirectUri, parsed.code, p.pkce.verifier)
                val bearer = pickHubBearer(tr)
                    ?: throw IllegalStateException("no id_token/access_token")
                tokenStore.saveBearer(bearer)
                val ok = Result.success(bearer)
                callbackListener?.invoke(ok)
                callbackListener = null
                onMain(ok)
            } catch (t: Throwable) {
                val err = Result.failure<String>(t)
                callbackListener?.invoke(err)
                callbackListener = null
                onMain(err)
            }
        }
    }

    companion object {
        /** Process-wide session used by MainActivity deep-link + Compose UI. */
        @Volatile var shared: AuthSession? = null

        fun obtain(context: Context): AuthSession {
            shared?.let { return it }
            return synchronized(this) {
                shared ?: AuthSession(EncryptedPrefsTokenStore(context.applicationContext)).also {
                    shared = it
                }
            }
        }
    }
}
