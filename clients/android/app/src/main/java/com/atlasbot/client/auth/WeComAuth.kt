package com.atlasbot.client.auth

import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.nio.charset.StandardCharsets

/**
 * WM1 WeComProvider — system browser (Custom Tabs) → code → Hub exchange → TokenStore.
 *
 * Contract (aligned with PC `exchangeWeComCode` / W1):
 * - Authorize URL uses corpId/agentId (**no PKCE**; state + one-time code).
 * - Callback remains [MOBILE_REDIRECT_URI] (`atlasbot://auth/callback`).
 * - Exchange is **Hub** `POST /auth/wecom/exchange` (secret never in the APK).
 * - Store Hub JWT via [TokenStore]; [HubClient] sends `Authorization: Bearer`.
 * - **No WebView** primary path (Custom Tabs / system browser only).
 * - Switch with `ATLAS_TICKET_PROVIDER=wecom` (or UI advanced); default stays oidc.
 *
 * See docs/i2-login-runbook.md § WeCom / § Mobile.
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

    data class ExchangeResponse(
        val accessToken: String,
        val tokenType: String? = null,
        val expiresIn: Long? = null,
        val subject: String? = null,
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

    fun exchangeUrl(hubHttpBase: String): String =
        "${hubHttpBase.trimEnd('/')}/auth/wecom/exchange"

    /**
     * POST Hub `/auth/wecom/exchange` — JSON `{code, state?}` → `access_token` (Hub JWT).
     * Client never holds `ATLAS_WECOM_SECRET`. Blocking — call off main thread.
     */
    fun exchangeCode(
        hubHttpBase: String,
        code: String,
        state: String? = null,
    ): ExchangeResponse {
        val url = exchangeUrl(hubHttpBase)
        val bodyJson = JSONObject().apply {
            put("code", code)
            if (!state.isNullOrEmpty()) put("state", state)
        }
        val conn = (URL(url).openConnection() as HttpURLConnection).apply {
            requestMethod = "POST"
            doOutput = true
            setRequestProperty("Accept", "application/json")
            setRequestProperty("Content-Type", "application/json")
            connectTimeout = 30_000
            readTimeout = 30_000
        }
        conn.outputStream.use { it.write(bodyJson.toString().toByteArray(StandardCharsets.UTF_8)) }
        val status = conn.responseCode
        val body = (if (status in 200..299) conn.inputStream else conn.errorStream)
            ?.bufferedReader()?.readText().orEmpty()
        if (status !in 200..299) {
            throw IllegalStateException("wecom exchange $status: $body")
        }
        return parseExchangeJson(body)
    }

    fun parseExchangeJson(body: String): ExchangeResponse {
        val o = JSONObject(body)
        val token = o.optString("access_token").takeIf { it.isNotEmpty() }
            ?: throw IllegalStateException("exchange missing access_token")
        return ExchangeResponse(
            accessToken = token,
            tokenType = o.optString("token_type").takeIf { it.isNotEmpty() },
            expiresIn = if (o.has("expires_in") && !o.isNull("expires_in")) o.optLong("expires_in") else null,
            subject = o.optString("subject").takeIf { it.isNotEmpty() },
        )
    }
}

/** Derive Hub HTTP base from a Hub WS URL (`ws(s)://host/ws` → `http(s)://host`). */
fun wsToHttpBase(ws: String): String {
    var s = ws.trim()
    when {
        s.startsWith("ws://") -> s = "http://" + s.removePrefix("ws://")
        s.startsWith("wss://") -> s = "https://" + s.removePrefix("wss://")
    }
    if (s.endsWith("/ws")) s = s.dropLast(3)
    return s.trimEnd('/')
}

/** `ATLAS_TICKET_PROVIDER` → oidc|wecom (default oidc). */
fun ticketProviderFromEnv(raw: String?): String {
    val v = raw?.trim()?.lowercase().orEmpty()
    return if (v == WeComAuth.PROVIDER) WeComAuth.PROVIDER else "oidc"
}
