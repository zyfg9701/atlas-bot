package com.atlasbot.client.auth


import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class WeComAuthTest {
    @Test
    fun buildAuthorizeUrl_has_corp_agent_state_no_pkce() {
        val cfg = WeComAuth.Config(
            corpId = "ww_mock_corp",
            agentId = "1000001",
            authorizeBase = "http://127.0.0.1:8091",
            hubHttpBase = "http://127.0.0.1:7700",
        )
        val url = WeComAuth.buildAuthorizeUrl(cfg, state = "st1")
        assertTrue(url.startsWith("http://127.0.0.1:8091/authorize?"))
        assertTrue(url.contains("appid=ww_mock_corp"))
        assertTrue(url.contains("agentid=1000001"))
        assertTrue(url.contains("response_type=code"))
        assertTrue(url.contains("scope=snsapi_base"))
        assertTrue(url.contains("state=st1"))
        assertTrue(url.contains("redirect_uri="))
        assertTrue(url.endsWith("#wechat_redirect") || url.contains("#wechat_redirect"))
        assertFalse(url.contains("code_challenge"))
        assertFalse(url.contains("code_challenge_method"))
    }

    @Test
    fun subject_shape() {
        assertEquals("wecom:ww_c:user1", WeComAuth.subject("ww_c", "user1"))
    }

    @Test
    fun ticketProvider_defaults_oidc() {
        assertEquals("oidc", ticketProviderFromEnv(null))
        assertEquals("oidc", ticketProviderFromEnv(""))
        assertEquals("oidc", ticketProviderFromEnv("OIDC"))
        assertEquals("wecom", ticketProviderFromEnv("wecom"))
        assertEquals("wecom", ticketProviderFromEnv(" WeCom "))
    }

    @Test
    fun wsToHttpBase_strips_ws_path() {
        assertEquals("http://127.0.0.1:7700", wsToHttpBase("ws://127.0.0.1:7700/ws"))
        assertEquals("https://hub.example", wsToHttpBase("wss://hub.example/ws"))
    }

    @Test
    fun parseExchangeJson_requires_access_token() {
        val ok = WeComAuth.parseExchangeJson(
            """{"access_token":"hub.jwt.here","token_type":"Bearer","subject":"wecom:c:u"}""",
        )
        assertEquals("hub.jwt.here", ok.accessToken)
        assertEquals("wecom:c:u", ok.subject)
        try {
            WeComAuth.parseExchangeJson("""{"token_type":"Bearer"}""")
            fail("expected missing access_token")
        } catch (e: IllegalStateException) {
            assertTrue(e.message!!.contains("access_token"))
        }
    }

    @Test
    fun exchangeCode_posts_json_and_parses_access_token() {
        val server = MockWebServer()
        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setHeader("Content-Type", "application/json")
                .setBody(
                    """{"access_token":"a.b.c","token_type":"Bearer","expires_in":3600,"subject":"wecom:ww_mock_corp:alice"}""",
                ),
        )
        server.start()
        try {
            val tr = WeComAuth.exchangeCode(
                hubHttpBase = server.url("/").toString().trimEnd('/'),
                code = "code-1",
                state = "st-1",
            )
            assertEquals("a.b.c", tr.accessToken)
            assertEquals("wecom:ww_mock_corp:alice", tr.subject)
            val req = server.takeRequest(2, TimeUnit.SECONDS)
            assertNotNull(req)
            assertEquals("POST", req!!.method)
            assertEquals("/auth/wecom/exchange", req.path)
            assertEquals("application/json", req.getHeader("Content-Type"))
            val body = req.body.readUtf8()
            assertTrue(body.contains("\"code\""))
            assertTrue(body.contains("code-1"))
            assertTrue(body.contains("\"state\""))
            assertFalse(body.contains("code_verifier"))
            assertFalse(body.contains("ATLAS_WECOM_SECRET"))
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun authSession_state_mismatch_fails_observably() {
        val store = MemoryTokenStore()
        val session = AuthSession(store, ioExecutor = Executors.newSingleThreadExecutor())
        val cfg = WeComAuth.Config(
            corpId = "ww",
            agentId = "1",
            authorizeBase = "http://127.0.0.1:9",
            hubHttpBase = "http://127.0.0.1:9",
        )
        session.prepareWeComLogin(cfg, state = "expected-state")
        val latch = CountDownLatch(1)
        var err: Throwable? = null
        session.handleCallbackUrl(
            "atlasbot://auth/callback?code=x&state=wrong-state",
        ) { result ->
            err = result.exceptionOrNull()
            latch.countDown()
        }
        assertTrue(latch.await(2, TimeUnit.SECONDS))
        assertNotNull(err)
        assertTrue(err!!.message!!.contains("state mismatch"))
        assertEquals(null, store.loadBearer())
    }

    @Test
    fun authSession_wecom_exchange_saves_bearer_and_provider() {
        val server = MockWebServer()
        server.enqueue(
            MockResponse()
                .setResponseCode(200)
                .setHeader("Content-Type", "application/json")
                .setBody("""{"access_token":"hub.tok.en","subject":"wecom:ww:u"}"""),
        )
        server.start()
        try {
            val store = MemoryTokenStore()
            val session = AuthSession(store, ioExecutor = Executors.newSingleThreadExecutor())
            val hub = server.url("/").toString().trimEnd('/')
            val cfg = WeComAuth.Config(
                corpId = "ww",
                agentId = "1",
                authorizeBase = "http://127.0.0.1:9",
                hubHttpBase = hub,
            )
            session.prepareWeComLogin(cfg, state = "st")
            val latch = CountDownLatch(1)
            var ok: String? = null
            var fail: Throwable? = null
            session.handleCallbackUrl(
                "atlasbot://auth/callback?code=c1&state=st",
            ) { result ->
                ok = result.getOrNull()
                fail = result.exceptionOrNull()
                latch.countDown()
            }
            assertTrue(latch.await(5, TimeUnit.SECONDS))
            if (fail != null) throw fail!!
            assertEquals("hub.tok.en", ok)
            assertEquals("hub.tok.en", store.loadBearer())
            assertEquals(WeComAuth.PROVIDER, store.loadProvider())
        } finally {
            server.shutdown()
        }
    }

    @Test
    fun logout_clears_provider() {
        val store = MemoryTokenStore()
        store.saveBearer("t", provider = WeComAuth.PROVIDER)
        assertEquals(WeComAuth.PROVIDER, store.loadProvider())
        val session = AuthSession(store)
        session.logout()
        assertEquals(null, store.loadBearer())
        assertEquals(null, store.loadProvider())
    }
}
