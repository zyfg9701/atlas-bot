package com.atlasbot.client.auth

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OidcAuthTest {
    @Test
    fun pkce_rfc7636_appendix_b() {
        val verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        val challenge = s256Challenge(verifier)
        assertEquals("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM", challenge)
        assertFalse(challenge.contains('='))
        assertFalse(challenge.contains('+'))
        assertFalse(challenge.contains('/'))
    }

    @Test
    fun generatePkce_shape() {
        val p = generatePkce()
        assertEquals("S256", p.method)
        assertTrue(p.verifier.length in 43..128)
        assertEquals(s256Challenge(p.verifier), p.challenge)
        assertEquals(43, p.challenge.length)
    }

    @Test
    fun parseCallback_code_and_state() {
        val p = parseCallbackUrl("atlasbot://auth/callback?code=abc&state=xyz")
        assertEquals("abc", p.code)
        assertEquals("xyz", p.state)
        assertNull(p.error)
    }

    @Test
    fun parseCallback_error() {
        val p = parseCallbackUrl("atlasbot://auth/callback?error=access_denied&state=s1")
        assertEquals("access_denied", p.error)
        assertEquals("s1", p.state)
        assertNull(p.code)
    }

    @Test
    fun buildAuthorizeUrl_includes_pkce_s256() {
        val pkce = PkcePair(verifier = "v", challenge = "chal", method = "S256")
        val url = buildAuthorizeUrl(
            OidcClientConfig(issuer = "http://127.0.0.1:8090", clientId = "atlas-bot-android"),
            redirectUri = MOBILE_REDIRECT_URI,
            state = "st",
            pkce = pkce,
        )
        assertTrue(url.startsWith("http://127.0.0.1:8090/authorize?"))
        assertTrue(url.contains("response_type=code"))
        assertTrue(url.contains("client_id=atlas-bot-android"))
        assertTrue(url.contains("code_challenge=chal"))
        assertTrue(url.contains("code_challenge_method=S256"))
        assertTrue(url.contains("redirect_uri="))
        assertTrue(url.contains("openid"))
    }

    @Test
    fun pickHubBearer_prefers_id_token() {
        val tr = TokenResponse(accessToken = "a.b.c", idToken = "id.tok.en")
        assertEquals("id.tok.en", pickHubBearer(tr))
    }

    @Test
    fun pickHubBearer_falls_back_to_jwt_access() {
        val tr = TokenResponse(accessToken = "aa.bb.cc")
        assertEquals("aa.bb.cc", pickHubBearer(tr))
    }

    @Test
    fun memoryTokenStore_save_load_clear() {
        val store = MemoryTokenStore()
        assertNull(store.loadBearer())
        store.saveBearer("tok")
        assertEquals("tok", store.loadBearer())
        store.clear()
        assertNull(store.loadBearer())
    }

    @Test
    fun redirect_uri_literal() {
        assertEquals("atlasbot://auth/callback", MOBILE_REDIRECT_URI)
        assertNotNull(DEFAULT_ANDROID_CLIENT_ID)
    }
}
