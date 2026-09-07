package com.atlasbot.client

import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okhttp3.mockwebserver.MockResponse
import okhttp3.mockwebserver.MockWebServer
import org.json.JSONArray
import org.json.JSONObject
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * P4 Android closed-loop smoke (JVM, no emulator).
 *
 * Drives [HubClient] against an in-process MockWebServer that speaks Bot-Relay
 * WS JSON-RPC and returns a **non-echo** transcript (same shape as P3.5 mock CLI).
 *
 * Expect stdout: SMOKE_OK p4 android mock-ws non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail
 */
class P4ClosedLoopSmokeTest {
    private lateinit var server: MockWebServer
    private var hub: HubClient? = null

    private val prompt = "hello-p4-android-smoke"
    private val nonEchoPreview =
        "atlas-mock-reply agent=agt_1 chars=${prompt.length} hash=p4android"

    @Before
    fun setUp() {
        server = MockWebServer()
        server.enqueue(
            MockResponse().withWebSocketUpgrade(
                object : WebSocketListener() {
                    private var seq = 0L

                    override fun onMessage(webSocket: WebSocket, text: String) {
                        val data = JSONObject(text)
                        if (!data.has("jsonrpc") && data.optString("kind") == "bot_client") {
                            val ack = JSONObject()
                                .put("connection_id", "c_p4_android")
                                .put("user_id", "u_p4")
                                .put("computer_hub_version", "0.4.0-p4")
                                .put(
                                    "supported_protocol_versions",
                                    JSONArray().put(PROTOCOL_VERSION),
                                )
                                .put(
                                    "capabilities",
                                    JSONArray()
                                        .put("bot.command")
                                        .put("bot.status")
                                        .put("bot.roster")
                                        .put("bot.subscribe")
                                        .put("bot.event"),
                                )
                            webSocket.send(ack.toString())
                            return
                        }
                        if (data.optString("jsonrpc") != "2.0" || !data.has("id")) return
                        val id = data.get("id")
                        val method = data.optString("method")
                        val params = data.optJSONObject("params") ?: JSONObject()
                        val result: JSONObject = when (method) {
                            "bot.status" -> JSONObject().put("runState", "idle")
                            "bot.roster" -> JSONObject().put(
                                "agents",
                                JSONArray().put(
                                    JSONObject()
                                        .put("agentId", "agt_1")
                                        .put("name", "Default")
                                        .put("status", "idle"),
                                ),
                            )
                            "bot.subscribe", "bot.unsubscribe" -> JSONObject()
                            "bot.command" -> handleCommand(webSocket, params)
                            else -> JSONObject().put("ok", true)
                        }
                        webSocket.send(
                            JSONObject()
                                .put("jsonrpc", "2.0")
                                .put("id", id)
                                .put("result", result)
                                .toString(),
                        )
                    }

                    private fun handleCommand(webSocket: WebSocket, params: JSONObject): JSONObject {
                        val name = params.optString("name")
                        val agentId = params.optString("agentId")
                        return when (name) {
                            "sendPrompt" -> {
                                seq += 1
                                val event = JSONObject()
                                    .put("v", 1)
                                    .put("agentId", agentId)
                                    .put("seq", seq)
                                    .put("channel", "hub:turn_finished")
                                    .put(
                                        "event",
                                        JSONObject().put("preview", nonEchoPreview),
                                    )
                                // Emit turn_finished after the RPC result is queued by caller.
                                // Schedule on a short delay so HubClient can register pending resolve first.
                                Thread {
                                    Thread.sleep(30)
                                    webSocket.send(
                                        JSONObject()
                                            .put("jsonrpc", "2.0")
                                            .put("method", "bot.event")
                                            .put("params", event)
                                            .toString(),
                                    )
                                }.start()
                                JSONObject()
                                    .put("accepted", true)
                                    .put("completed", true)
                                    .put("preview", nonEchoPreview)
                            }
                            "getAgentTranscriptTail" -> JSONObject().put(
                                "entries",
                                JSONArray()
                                    .put(
                                        JSONObject()
                                            .put("id", "e_user")
                                            .put("role", "user")
                                            .put("text", prompt),
                                    )
                                    .put(
                                        JSONObject()
                                            .put("id", "e_assistant")
                                            .put("role", "assistant")
                                            .put("text", nonEchoPreview),
                                    ),
                            )
                            else -> JSONObject().put("ok", true)
                        }
                    }
                },
            ),
        )
        server.start()
    }

    @After
    fun tearDown() {
        runCatching { hub?.shutdown() }
        hub = null
        runCatching { server.shutdown() }
    }

    @Test
    fun closed_loop_non_echo_prints_smoke_ok() {
        val wsUrl = server.url("/ws").toString()
            .replace("http://", "ws://")
            .replace("https://", "wss://")

        val ready = CountDownLatch(1)
        val turnFinished = CountDownLatch(1)
        val fail = AtomicReference<String?>(null)
        val transcriptText = AtomicReference<String?>(null)

        val listener = object : HubClientListener {
            override fun onState(state: ConnState, detail: String?) {}
            override fun onLog(level: String, msg: String, data: Any?) {}
            override fun onHelloAck(ack: HelloAck) {}
            override fun onEvent(ev: BotEventEnvelope) {
                if (ev.channel == "hub:turn_finished") {
                    turnFinished.countDown()
                }
            }
            override fun onRpcError(err: DisplayError) {
                fail.compareAndSet(null, "rpc error: ${err.message} reason=${err.reason}")
            }
        }

        val client = HubClient(url = wsUrl, listener = listener)
        hub = client
        client.connect(
            onReady = { ready.countDown() },
            onFail = { err ->
                fail.compareAndSet(null, "connect failed: ${err.message}")
                ready.countDown()
            },
        )
        assertTrue("hello_ack timeout", ready.await(5, TimeUnit.SECONDS))
        fail.get()?.let { fail(it) }
        assertTrue(client.connectionState == ConnState.READY)

        val statusOk = CountDownLatch(1)
        client.status(
            onOk = { statusOk.countDown() },
            onErr = { e -> fail.compareAndSet(null, "status: ${e.message}"); statusOk.countDown() },
        )
        assertTrue(statusOk.await(5, TimeUnit.SECONDS))
        fail.get()?.let { fail(it) }

        val rosterOk = CountDownLatch(1)
        client.roster(
            onOk = { agents ->
                assertTrue(agents.any { it.agentId == "agt_1" })
                rosterOk.countDown()
            },
            onErr = { e -> fail.compareAndSet(null, "roster: ${e.message}"); rosterOk.countDown() },
        )
        assertTrue(rosterOk.await(5, TimeUnit.SECONDS))
        fail.get()?.let { fail(it) }

        val subOk = CountDownLatch(1)
        client.subscribe(
            listOf("agt_1"),
            onOk = { subOk.countDown() },
            onErr = { e -> fail.compareAndSet(null, "subscribe: ${e.message}"); subOk.countDown() },
        )
        assertTrue(subOk.await(5, TimeUnit.SECONDS))
        fail.get()?.let { fail(it) }

        val sendOk = CountDownLatch(1)
        val previewRef = AtomicReference<String?>(null)
        client.sendPrompt(
            agentId = "agt_1",
            prompt = prompt,
            onOk = { result ->
                val preview = (result as? JSONObject)?.optString("preview")
                previewRef.set(preview)
                sendOk.countDown()
            },
            onErr = { e -> fail.compareAndSet(null, "sendPrompt: ${e.message}"); sendOk.countDown() },
        )
        assertTrue(sendOk.await(5, TimeUnit.SECONDS))
        fail.get()?.let { fail(it) }

        val preview = previewRef.get()
        assertNotNull(preview)
        assertTrue("expected mock non-echo reply, got $preview", preview!!.contains("atlas-mock-reply"))
        assertFalse("must not be stub echo: $preview", preview.startsWith("echo:"))
        assertFalse("must not be bare prompt echo: $preview", preview == prompt)

        assertTrue("hub:turn_finished timeout", turnFinished.await(5, TimeUnit.SECONDS))

        val tailOk = CountDownLatch(1)
        client.getAgentTranscriptTail(
            agentId = "agt_1",
            limit = 20,
            onOk = { result ->
                val entries = (result as? JSONObject)?.optJSONArray("entries")
                val joined = buildString {
                    if (entries != null) {
                        for (i in 0 until entries.length()) {
                            append(entries.getJSONObject(i).optString("text")).append('\n')
                        }
                    }
                }
                transcriptText.set(joined)
                tailOk.countDown()
            },
            onErr = { e -> fail.compareAndSet(null, "transcriptTail: ${e.message}"); tailOk.countDown() },
        )
        assertTrue(tailOk.await(5, TimeUnit.SECONDS))
        fail.get()?.let { fail(it) }

        val transcript = transcriptText.get()
        assertNotNull(transcript)
        assertTrue(
            "transcript must contain non-echo mock reply: $transcript",
            transcript!!.contains("atlas-mock-reply"),
        )
        assertFalse(
            "transcript must not be echo-only: $transcript",
            transcript.trim() == "echo: $prompt" || transcript.contains("echo: $prompt"),
        )

        client.shutdown()

        // Recorded evidence line for QA / runbook / CI logs.
        println(
            "SMOKE_OK p4 android mock-ws non-echo+hello+cold+subscribe+sendPrompt+turn_finished+transcriptTail",
        )
    }
}
