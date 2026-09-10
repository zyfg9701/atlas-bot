package com.atlasbot.client.auth

/**
 * W1 WeComProvider stub — same deep-link shape as I2.2 OIDC (M1).
 *
 * Full mobile wiring is a follow-up; this PR nails the contract:
 * - Authorize URL uses corpId/agentId (no PKCE; state + one-time code).
 * - Callback remains [MOBILE_REDIRECT_URI] (`atlasbot://auth/callback`).
 * - Exchange is **Hub** `POST /auth/wecom/exchange` (secret never in the APK).
 * - Store Hub JWT via [TokenStore]; [HubClient] sends `Authorization: Bearer`.
 * - **No WebView** primary path (Custom Tabs / system browser only).
 * - Switch with `ATLAS_TICKET_PROVIDER=wecom` (or UI advanced); default stays oidc.
 *
 * See docs/i2-login-runbook.md § WeCom.
 */
object WeComAuth {
    const val PROVIDER = "wecom"

    data class Config(
        val corpId: String,
        val agentId: String,
        /** mock-wecom base or live open.weixin authorize host path */
        val authorizeBase: String,
        val hubHttpBase: String,
        val redirectUri: String = MOBILE_REDIRECT_URI,
    )

    fun buildAuthorizeUrl(cfg: Config, state: String): String {
        val base = cfg.authorizeBase.trimEnd('/')
        val authorize = if (base.endsWith("/authorize")) base else "$base/authorize"
        val q = listOf(
            "appid" to cfg.corpId,
            "redirect_uri" to cfg.redirectUri,
            "response_type" to "code",
            "scope" to "snsapi_base",
            "state" to state,
            "agentid" to cfg.agentId,
        ).joinToString("&") { (k, v) ->
            "$k=${java.net.URLEncoder.encode(v, Charsets.UTF_8.name())}"
        }
        return "$authorize?$q#wechat_redirect"
    }

    fun subject(corpId: String, userid: String): String = "wecom:$corpId:$userid"

    /**
     * POST Hub exchange. Returns Hub-verifiable JWT (not opaque WeCom token).
     * Stub: callers should prefer full implementation in a follow-up PR;
     * shape is locked here for compile/docs.
     */
    fun exchangeUrl(hubHttpBase: String): String =
        "${hubHttpBase.trimEnd('/')}/auth/wecom/exchange"
}
