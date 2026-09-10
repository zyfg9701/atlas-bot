package com.atlasbot.client.auth

import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URI
import java.net.URL
import java.net.URLEncoder
import java.nio.charset.StandardCharsets
import java.security.MessageDigest
import java.security.SecureRandom
import java.util.Base64

/** Custom scheme redirect (IdP must register this alongside PC loopback). */
const val MOBILE_REDIRECT_URI = "atlasbot://auth/callback"
const val MOBILE_CALLBACK_SCHEME = "atlasbot"
const val MOBILE_CALLBACK_HOST = "auth"
const val MOBILE_CALLBACK_PATH = "/callback"
const val DEFAULT_ANDROID_CLIENT_ID = "atlas-bot-android"

data class PkcePair(
    val verifier: String,
    val challenge: String,
    val method: String = "S256",
)

data class OidcClientConfig(
    val issuer: String,
    val clientId: String = DEFAULT_ANDROID_CLIENT_ID,
    val scopes: List<String> = listOf("openid", "profile"),
    val audience: String? = null,
) {
    val authorizeUrl: String get() = "${issuer.trimEnd('/')}/authorize"
    val tokenUrl: String get() = "${issuer.trimEnd('/')}/token"
}

data class TokenResponse(
    val accessToken: String? = null,
    val idToken: String? = null,
    val refreshToken: String? = null,
    val tokenType: String? = null,
    val expiresIn: Long? = null,
)

data class CallbackParse(
    val code: String? = null,
    val state: String? = null,
    val error: String? = null,
)

private val VERIFIER_ALPHABET =
    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~"

fun randomVerifier(len: Int = 64): String {
    val n = len.coerceIn(43, 128)
    val rng = SecureRandom()
    return buildString(n) {
        repeat(n) { append(VERIFIER_ALPHABET[rng.nextInt(VERIFIER_ALPHABET.length)]) }
    }
}

fun s256Challenge(verifier: String): String {
    val digest = MessageDigest.getInstance("SHA-256").digest(verifier.toByteArray(StandardCharsets.US_ASCII))
    return Base64.getUrlEncoder().withoutPadding().encodeToString(digest)
}

fun generatePkce(): PkcePair {
    val verifier = randomVerifier(64)
    return PkcePair(verifier = verifier, challenge = s256Challenge(verifier))
}

fun randomState(): String = randomVerifier(32)

fun buildAuthorizeUrl(cfg: OidcClientConfig, redirectUri: String, state: String, pkce: PkcePair): String {
    val q = linkedMapOf(
        "response_type" to "code",
        "client_id" to cfg.clientId,
        "redirect_uri" to redirectUri,
        "scope" to cfg.scopes.joinToString(" "),
        "state" to state,
        "code_challenge" to pkce.challenge,
        "code_challenge_method" to pkce.method,
    )
    cfg.audience?.takeIf { it.isNotBlank() }?.let { q["audience"] = it }
    val query = q.entries.joinToString("&") { (k, v) ->
        "${URLEncoder.encode(k, "UTF-8")}=${URLEncoder.encode(v, "UTF-8")}"
    }
    return "${cfg.authorizeUrl}?$query"
}

/** Parse query `code`/`state`/`error` from a callback URL (custom scheme or https). */
fun parseCallbackUrl(callbackUrl: String): CallbackParse {
    val uri = URI(callbackUrl)
    val q = uri.rawQuery.orEmpty()
    val params = linkedMapOf<String, String>()
    if (q.isNotEmpty()) {
        for (part in q.split('&')) {
            val eq = part.indexOf('=')
            if (eq <= 0) continue
            val k = java.net.URLDecoder.decode(part.substring(0, eq), "UTF-8")
            val v = java.net.URLDecoder.decode(part.substring(eq + 1), "UTF-8")
            params[k] = v
        }
    }
    val err = params["error"]
    if (!err.isNullOrEmpty()) {
        return CallbackParse(error = err, state = params["state"])
    }
    return CallbackParse(code = params["code"], state = params["state"])
}

fun looksLikeJwt(token: String): Boolean {
    val parts = token.split('.')
    return parts.size == 3 && parts.all { it.isNotEmpty() }
}

/** Prefer id_token; else JWT access_token (same shape as pick_hub_bearer / PC). */
fun pickHubBearer(tr: TokenResponse): String? {
    tr.idToken?.takeIf { it.isNotEmpty() }?.let { return it }
    tr.accessToken?.takeIf { looksLikeJwt(it) }?.let { return it }
    return tr.accessToken?.takeIf { it.isNotEmpty() } ?: tr.idToken?.takeIf { it.isNotEmpty() }
}

private fun JSONObject.optNullableString(key: String): String? {
    if (!has(key) || isNull(key)) return null
    val s = optString(key)
    return s.takeIf { it.isNotEmpty() }
}

fun parseTokenJson(body: String): TokenResponse {
    val o = JSONObject(body)
    return TokenResponse(
        accessToken = o.optNullableString("access_token"),
        idToken = o.optNullableString("id_token"),
        refreshToken = o.optNullableString("refresh_token"),
        tokenType = o.optNullableString("token_type"),
        expiresIn = if (o.has("expires_in") && !o.isNull("expires_in")) o.optLong("expires_in") else null,
    )
}

/**
 * Exchange authorization code at the token endpoint (PKCE).
 * Blocking HTTP — call from a background thread / coroutine.
 */
fun exchangeCode(
    cfg: OidcClientConfig,
    redirectUri: String,
    code: String,
    codeVerifier: String,
): TokenResponse {
    val form = listOf(
        "grant_type" to "authorization_code",
        "code" to code,
        "redirect_uri" to redirectUri,
        "client_id" to cfg.clientId,
        "code_verifier" to codeVerifier,
    ).joinToString("&") { (k, v) ->
        "${URLEncoder.encode(k, "UTF-8")}=${URLEncoder.encode(v, "UTF-8")}"
    }
    val conn = (URL(cfg.tokenUrl).openConnection() as HttpURLConnection).apply {
        requestMethod = "POST"
        doOutput = true
        setRequestProperty("Accept", "application/json")
        setRequestProperty("Content-Type", "application/x-www-form-urlencoded")
        connectTimeout = 30_000
        readTimeout = 30_000
    }
    conn.outputStream.use { it.write(form.toByteArray(StandardCharsets.UTF_8)) }
    val status = conn.responseCode
    val body = (if (status in 200..299) conn.inputStream else conn.errorStream)
        ?.bufferedReader()?.readText().orEmpty()
    if (status !in 200..299) {
        throw IllegalStateException("token endpoint $status: $body")
    }
    return parseTokenJson(body)
}
