package com.atlasbot.client

import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class HubClientEncodingTest {
    @Test
    fun hello_frame_is_bot_client() {
        val h = RpcFrames.hello()
        assertEquals(PROTOCOL_VERSION, h.getString("protocol_version"))
        assertEquals("bot_client", h.getString("kind"))
        assertFalse(h.has("jsonrpc"))
    }

    @Test
    fun rpc_frame_jsonrpc_2() {
        val f = RpcFrames.rpc(7, "bot.status", JSONObject())
        assertEquals("2.0", f.getString("jsonrpc"))
        assertEquals(7, f.getInt("id"))
        assertEquals("bot.status", f.getString("method"))
    }

    @Test
    fun sendPrompt_args_keep_agentId_and_prompt() {
        val args = RpcFrames.sendPromptArgs("agt_1", "hi", immediate = true)
        assertEquals("agt_1", args.getString("agentId"))
        assertEquals("hi", args.getString("prompt"))
        assertTrue(args.getBoolean("immediate"))
        val cmd = RpcFrames.command("agt_1", "sendPrompt", args)
        assertEquals("agt_1", cmd.getString("agentId"))
        assertEquals("sendPrompt", cmd.getString("name"))
        assertEquals("agt_1", cmd.getJSONObject("args").getString("agentId"))
    }

    @Test
    fun transcriptTail_uses_id_field() {
        val args = RpcFrames.transcriptTailArgs("agt_2", 10)
        assertEquals("agt_2", args.getString("id"))
        assertEquals(10, args.getInt("limit"))
    }

    @Test
    fun subscribe_agentIds_camelCase() {
        val p = RpcFrames.subscribe(listOf("agt_1", "agt_2"))
        assertEquals(2, p.getJSONArray("agentIds").length())
        assertFalse(p.has("fullFidelity"))
    }

    @Test
    fun normalizeError_keeps_agent_id_mismatch() {
        val err = normalizeError(
            JSONObject()
                .put("code", -32000)
                .put("message", "command_rejected")
                .put("data", JSONObject().put("reason", "agent_id_mismatch").put("retryable", false)),
        )
        assertEquals("command_rejected", err.message)
        assertEquals("agent_id_mismatch", err.reason)
        assertEquals(false, err.retryable)
    }

    @Test
    fun normalizeError_unknown_code_is_failure() {
        val err = normalizeError(JSONObject().put("message", "totally_new_code"))
        assertEquals("upstream_error", err.code)
        assertTrue(err.message.contains("failure"))
    }

    @Test
    fun normalizeError_garbage_does_not_throw() {
        val err = normalizeError(null)
        assertEquals("upstream_error", err.code)
    }
}
