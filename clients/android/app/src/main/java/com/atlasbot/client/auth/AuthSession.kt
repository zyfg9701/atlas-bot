package com.atlasbot.client.auth

import android.content.Context
import android.content.Intent
import android.net.Uri
import androidx.browser.customtabs.CustomTabsIntent
import java.util.concurrent.Executor
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicReference

/**
 * M1 / WM1 Android login: Chrome Custom Tabs.
 * OIDC uses PKCE S256; WeCom uses state + one-time code (no PKCE) → Hub exchange.
 * Primary path is **system browser / Custom Tabs** — not an in-app WebView.
 *
 * Redirect: [MOBILE_REDIRECT_URI] (`atlasbot://auth/callback`).
 * Pending is typed separately ([OidcPending] vs [WeComPending]) so providers cannot mix.
 */
class AuthSession(
    private val tokenStore: TokenStore,
    private val ioExecutor: Executor = Executors.newSingleThreadExecutor(),
) {
    /** OIDC pending — holds PKCE verifier; must not be used for WeCom callback. */
    data class OidcPending(
        val cfg: OidcClientConfig,
        val redirectUri: String,
        val state: String,
        val pkce: PkcePair,
    )

    /** WeCom pending — no PKCE; Hub exchange uses code + optional state. */
    data class WeComPending(
        val cfg: WeComAuth.Config,
        val state: String,
    )

    sealed class Pending {
        data class Oidc(val value: OidcPending) : Pending()
        data class WeCom(val value: WeComPending) : Pending()
    }

    private val pending = AtomicReference<Pending?>(null)
    @Volatile private var callbackListener: ((Result<String>) -> Unit)? = null

    fun hasTicket(): Boolean = !tokenStore.loadBearer().isNullOrBlank()
    fun currentBearer(): String? = tokenStore.loadBearer()
    fun currentProvider(): String? = tokenStore.loadProvider()

    fun saveManualBearer(token: String, provider: String? = null) {
        tokenStore.saveBearer(token.trim(), provider)
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
        pending.set(Pending.Oidc(OidcPending(cfg, redirectUri, state, pkce)))
        val url = buildAuthorizeUrl(cfg, redirectUri, state, pkce)
        launchCustomTab(context, url)
    }

    /**
     * Start WeCom authorize in a Custom Tab (no PKCE).
     * Callback → Hub `POST /auth/wecom/exchange` → TokenStore (provider=wecom).
     */
    fun startWeComLogin(
        context: Context,
        cfg: WeComAuth.Config,
        onComplete: (Result<String>) -> Unit,
    ) {
        callbackListener = onComplete
        val state = randomState()
        pending.set(Pending.WeCom(WeComPending(cfg, state)))
        val url = WeComAuth.buildAuthorizeUrl(cfg, state)
        launchCustomTab(context, url)
    }

    /**
     * Thin dispatcher: `provider=wecom` → [startWeComLogin]; else OIDC [startLogin].
     * Avoids duplicating deep-link state machine in UI.
     */
    fun startLogin(
        context: Context,
        provider: String,
        oidc: OidcClientConfig? = null,
        wecom: WeComAuth.Config? = null,
        redirectUri: String = MOBILE_REDIRECT_URI,
        onComplete: (Result<String>) -> Unit,
    ) {
        when (ticketProviderFromEnv(provider)) {
            WeComAuth.PROVIDER -> {
                val cfg = wecom ?: throw IllegalArgumentException("wecom config required")
                startWeComLogin(context, cfg, onComplete)
            }
            else -> {
                val cfg = oidc ?: throw IllegalArgumentException("oidc config required")
                startLogin(context, cfg, redirectUri, onComplete)
            }
        }
    }

    /** Install WeCom pending without launching a browser (unit tests). */
    fun prepareWeComLogin(cfg: WeComAuth.Config, state: String = randomState()): String {
        pending.set(Pending.WeCom(WeComPending(cfg, state)))
        return state
    }

    /** Install OIDC pending without launching a browser (unit tests). */
    fun prepareOidcLogin(
        cfg: OidcClientConfig,
        redirectUri: String = MOBILE_REDIRECT_URI,
        state: String = randomState(),
        pkce: PkcePair = generatePkce(),
    ): String {
        pending.set(Pending.Oidc(OidcPending(cfg, redirectUri, state, pkce)))
        return state
    }

    private fun launchCustomTab(context: Context, url: String) {
        val intent = CustomTabsIntent.Builder().build()
        intent.intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        intent.launchUrl(context, Uri.parse(url))
    }

    /** Called from MainActivity when `atlasbot://auth/callback` arrives. */
    fun handleCallbackUri(uri: Uri, onMain: (Result<String>) -> Unit) {
        handleCallbackUrl(uri.toString(), onMain)
    }

    /** String entry for JVM unit tests (no android.net.Uri). */
    fun handleCallbackUrl(callbackUrl: String, onMain: (Result<String>) -> Unit) {
        val p = pending.getAndSet(null)
        if (p == null) {
            onMain(Result.failure(IllegalStateException("no pending login")))
            return
        }
        val parsed = parseCallbackUrl(callbackUrl)
        if (!parsed.error.isNullOrEmpty()) {
            fail(Result.failure(IllegalStateException(parsed.error)), onMain)
            return
        }
        if (parsed.code.isNullOrEmpty()) {
            fail(Result.failure(IllegalStateException("missing code")), onMain)
            return
        }
        when (p) {
            is Pending.Oidc -> handleOidcCallback(p.value, parsed, onMain)
            is Pending.WeCom -> handleWeComCallback(p.value, parsed, onMain)
        }
    }

    private fun handleOidcCallback(
        p: OidcPending,
        parsed: CallbackParse,
        onMain: (Result<String>) -> Unit,
    ) {
        if (parsed.state != null && parsed.state != p.state) {
            fail(Result.failure(IllegalStateException("state mismatch")), onMain)
            return
        }
        val code = parsed.code!!
        ioExecutor.execute {
            try {
                val tr = exchangeCode(p.cfg, p.redirectUri, code, p.pkce.verifier)
                val bearer = pickHubBearer(tr)
                    ?: throw IllegalStateException("no id_token/access_token")
                tokenStore.saveBearer(bearer, provider = "oidc")
                succeed(Result.success(bearer), onMain)
            } catch (t: Throwable) {
                fail(Result.failure(t), onMain)
            }
        }
    }

    private fun handleWeComCallback(
        p: WeComPending,
        parsed: CallbackParse,
        onMain: (Result<String>) -> Unit,
    ) {
        if (parsed.state != null && parsed.state != p.state) {
            fail(Result.failure(IllegalStateException("state mismatch")), onMain)
            return
        }
        val code = parsed.code!!
        ioExecutor.execute {
            try {
                val tr = WeComAuth.exchangeCode(
                    hubHttpBase = p.cfg.hubHttpBase,
                    code = code,
                    state = p.state,
                )
                tokenStore.saveBearer(tr.accessToken, provider = WeComAuth.PROVIDER)
                succeed(Result.success(tr.accessToken), onMain)
            } catch (t: Throwable) {
                fail(Result.failure(t), onMain)
            }
        }
    }

    private fun succeed(ok: Result<String>, onMain: (Result<String>) -> Unit) {
        callbackListener?.invoke(ok)
        callbackListener = null
        onMain(ok)
    }

    private fun fail(err: Result<String>, onMain: (Result<String>) -> Unit) {
        callbackListener?.invoke(err)
        callbackListener = null
        onMain(err)
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
