package com.atlasbot.client.auth

import com.atlasbot.client.HubClient
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class HubBearerHeaderTest {
    @Test
    fun connect_request_omits_authorization_when_null() {
        val client = HubClient(url = "ws://127.0.0.1:9/ws")
        val req = client.buildConnectRequest()
        assertNull(req.header("Authorization"))
        client.shutdown()
    }

    @Test
    fun connect_request_sets_bearer_when_present() {
        val client = HubClient(url = "ws://127.0.0.1:9/ws")
        client.setAuthorization("id.tok.en")
        val req = client.buildConnectRequest()
        assertEquals("Bearer id.tok.en", req.header("Authorization"))
        client.setAuthorization(null)
        assertNull(client.buildConnectRequest().header("Authorization"))
        client.shutdown()
    }

    @Test
    fun connect_named_authorization_param_stores_token() {
        val client = HubClient(url = "ws://127.0.0.1:9/ws")
        // Do not actually open a socket — only verify set via optional param shape.
        client.setAuthorization("a.b.c")
        assertEquals("a.b.c", client.getAuthorization())
        client.shutdown()
    }
}
