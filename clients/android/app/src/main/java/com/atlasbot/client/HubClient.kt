package com.atlasbot.client

import ai.x.grok.botrelay.protocol.BOT_EVENT_ENVELOPE_V
import ai.x.grok.botrelay.protocol.COMMAND_REJECTED_AGENT_ID_MISMATCH
import ai.x.grok.botrelay.protocol.HubChannel
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import org.json.JSONArray
import org.json.JSONObject
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicInteger

/** Bot-Relay hub client (P4 Android). Mirrors clients/pc Hub WS loop. */
const val PROTOCOL_VERSION = "1.0.0"
const val DEFAULT_HUB_WS = "ws://10.0.2.2:7700/ws" // emulator → host loopback

enum class ConnState { DISCONNECTED, CONNECTING, HELLO, READY, ERROR }

data class HelloAck(
    val connectionId: String,
    val userId: String,
    val computerHubVersion: String,
    val supportedProtocolVersions: List<String>,
    val capabilities: List<String>,
)

data class RosterEntry(
    val agentId: String,
    val name: String,
    val status: String,
    val lastTurnAt: Long? = null,
)

data class BotEventEnvelope(
    val v: Int,
    val agentId: String,
    val seq: Long,
    val channel: String,
    val event: Any?,
)

data class DisplayError(
    val message: String,
    val code: Any? = null,
    val reason: String? = null,
    val retryable: Boolean? = null,
    val raw: Any? = null,
)

private val KNOWN_CODES = setOf(
    "command_rejected",
    "identity_unavailable",
    "link_state_unavailable",
    "upstream_error",
    "forbidden",
    "link_required",
    "link_removed",
    "consent_required",
    "box_unavailable",
    "computer_unavailable",
)

fun normalizeError(data: Any?): DisplayError {
    if (data !is JSONObject && data !is Map<*, *>) {
        return DisplayError(message = "unknown failure", code = "upstream_error", raw = data)
    }
    val o = when (data) {
        is JSONObject -> data
        is Map<*, *> -> JSONObject(data)
        else -> return DisplayError(message = "unknown failure", code = "upstream_error", raw = data)
    }
    if (!o.has("message") && !o.has("code")) {
        return DisplayError(message = "unknown failure", code = "upstream_error", raw = data)
    }
    val msg = o.optString("message", "error")
    val payload: JSONObject = when {
        o.opt("data") is JSONObject -> o.getJSONObject("data")
        else -> o
    }
    val reason = if (payload.has("reason") && !payload.isNull("reason")) payload.optString("reason") else null
    val retryable = if (payload.has("retryable")) payload.optBoolean("retryable") else null
    if (msg.isNotEmpty() && msg !in KNOWN_CODES) {
        val codeNum = o.opt("code")
        if (codeNum is Number && codeNum.toInt() < 0) {
            return DisplayError(message = msg, code = codeNum, reason = reason, retryable = retryable, raw = data)
        }
        return DisplayError(
            message = "failure ($msg)",
            code = "upstream_error",
            reason = reason,
            retryable = false,
            raw = data,
        )
    }
    val codeStr = msg.takeIf { it in KNOWN_CODES }
    return DisplayError(
        message = msg,
        code = codeStr ?: o.opt("code"),
        reason = reason,
        retryable = retryable,
        raw = data,
    )
}

/** JSON-RPC / hello frame builders — unit-tested without a socket. */
object RpcFrames {
    fun hello(): JSONObject = JSONObject()
        .put("protocol_version", PROTOCOL_VERSION)
        .put("kind", "bot_client")

    fun rpc(id: Int, method: String, params: JSONObject): JSONObject = JSONObject()
        .put("jsonrpc", "2.0")
        .put("id", id)
        .put("method", method)
        .put("params", params)

    fun status(): JSONObject = JSONObject()
    fun roster(): JSONObject = JSONObject()
    fun subscribe(agentIds: List<String>, fullFidelity: Boolean = false): JSONObject {
        val o = JSONObject().put("agentIds", JSONArray(agentIds))
        if (fullFidelity) o.put("fullFidelity", true)
        return o
    }
    fun unsubscribe(agentIds: List<String>): JSONObject =
        JSONObject().put("agentIds", JSONArray(agentIds))

    fun command(agentId: String, name: String, args: JSONObject): JSONObject =
        JSONObject().put("agentId", agentId).put("name", name).put("args", args)

    fun sendPromptArgs(agentId: String, prompt: String, immediate: Boolean = false): JSONObject {
        val args = JSONObject().put("agentId", agentId).put("prompt", prompt)
        if (immediate) args.put("immediate", true)
        return args
    }

    fun transcriptTailArgs(agentId: String, limit: Int = 20): JSONObject =
        JSONObject().put("id", agentId).put("limit", limit)
}

interface HubClientListener {
    fun onState(state: ConnState, detail: String? = null)
    fun onLog(level: String, msg: String, data: Any? = null)
    fun onHelloAck(ack: HelloAck)
    fun onEvent(ev: BotEventEnvelope)
    fun onRpcError(err: DisplayError)
}

class HubClient(
    private var url: String = DEFAULT_HUB_WS,
    private val listener: HubClientListener? = null,
) {
    private val client = OkHttpClient.Builder()
        .pingInterval(20, TimeUnit.SECONDS)
        .readTimeout(0, TimeUnit.MILLISECONDS)
        .build()

    private var ws: WebSocket? = null
    private val nextId = AtomicInteger(1)
    private val pending = ConcurrentHashMap<Any, Pending>()
    @Volatile private var state: ConnState = ConnState.DISCONNECTED
    private val lastSeq = ConcurrentHashMap<String, Long>()
    private val subscribed = ConcurrentHashMap.newKeySet<String>()

    val connectionState: ConnState get() = state
    val subscribedAgents: Set<String> get() = subscribed.toSet()

    fun setUrl(url: String) { this.url = url }

    private data class Pending(
        val resolve: (Any?) -> Unit,
        val reject: (DisplayError) -> Unit,
    )

    private fun setState(s: ConnState, detail: String? = null) {
        state = s
        listener?.onState(s, detail)
    }

    private fun log(level: String, msg: String, data: Any? = null) {
        listener?.onLog(level, msg, data)
    }

    fun connect(onReady: (HelloAck) -> Unit, onFail: (DisplayError) -> Unit) {
        disconnect()
        setState(ConnState.CONNECTING)
        var settled = false
        val request = Request.Builder().url(url).build()
        ws = client.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                setState(ConnState.HELLO)
                val hello = RpcFrames.hello()
                log("info", "→ hello", hello)
                webSocket.send(hello.toString())
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                val data = try {
                    JSONObject(text)
                } catch (_: Exception) {
                    log("warn", "non-JSON frame", text)
                    return
                }
                if (!data.has("jsonrpc") && data.has("connection_id")) {
                    val caps = mutableListOf<String>()
                    data.optJSONArray("capabilities")?.let { arr ->
                        for (i in 0 until arr.length()) caps.add(arr.getString(i))
                    }
                    val versions = mutableListOf<String>()
                    data.optJSONArray("supported_protocol_versions")?.let { arr ->
                        for (i in 0 until arr.length()) versions.add(arr.getString(i))
                    }
                    val ack = HelloAck(
                        connectionId = data.optString("connection_id"),
                        userId = data.optString("user_id"),
                        computerHubVersion = data.optString("computer_hub_version"),
                        supportedProtocolVersions = versions,
                        capabilities = caps,
                    )
                    setState(ConnState.READY)
                    log("info", "← hello_ack", data)
                    listener?.onHelloAck(ack)
                    if (!settled) {
                        settled = true
                        onReady(ack)
                    }
                    return
                }
                if (data.optString("jsonrpc") == "2.0" && data.has("id")) {
                    val id = data.get("id")
                    val p = pending.remove(id)
                    if (p == null) {
                        log("warn", "orphan response", data)
                        return
                    }
                    if (data.has("error")) {
                        val err = normalizeError(data.get("error"))
                        log("error", "RPC error id=$id", err)
                        listener?.onRpcError(err)
                        p.reject(err)
                    } else {
                        log("info", "← result id=$id", data.opt("result"))
                        p.resolve(data.opt("result"))
                    }
                    return
                }
                if (data.optString("jsonrpc") == "2.0" && data.optString("method") == "bot.event") {
                    val params = data.optJSONObject("params") ?: return
                    handleEvent(params)
                    return
                }
                log("warn", "ignored frame", data)
            }

            override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
                onMessage(webSocket, bytes.utf8())
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                setState(ConnState.ERROR, t.message)
                log("error", "websocket error", t.message)
                if (!settled) {
                    settled = true
                    onFail(normalizeError(mapOf("message" to (t.message ?: "websocket error"))))
                }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                setState(ConnState.DISCONNECTED)
                log("warn", "disconnected — reconnect will re-hello (seq not comparable)")
                pending.forEach { (_, p) -> p.reject(normalizeError(mapOf("message" to "connection closed"))) }
                pending.clear()
                lastSeq.clear()
                subscribed.clear()
                ws = null
                if (!settled) {
                    settled = true
                    onFail(normalizeError(mapOf("message" to "connection closed before hello_ack")))
                }
            }
        })
    }

    fun disconnect() {
        ws?.close(1000, "bye")
        ws = null
        pending.clear()
        setState(ConnState.DISCONNECTED)
    }

    private fun handleEvent(params: JSONObject) {
        val agentId = params.optString("agentId")
        val seq = params.optLong("seq")
        val channel = params.optString("channel")
        val prev = lastSeq[agentId]
        if (prev != null && seq != prev + 1) {
            log(
                "warn",
                "seq discontinuity agent=$agentId prev=$prev got=$seq (display only; no auto-resync)",
            )
        }
        lastSeq[agentId] = seq
        when (channel) {
            HubChannel.ResyncRequired.wireValue(), "hub:resync_required" ->
                log("warn", "hub:resync_required — re-fetch transcript (explicit)", params)
            HubChannel.TurnFinished.wireValue(), "hub:turn_finished" ->
                log("event", "hub:turn_finished", params)
            else -> log("event", "bot.event $channel", params)
        }
        if (agentId !in subscribed) {
            log("info", "ignoring event for unsubscribed agent $agentId")
            return
        }
        listener?.onEvent(
            BotEventEnvelope(
                v = params.optInt("v", BOT_EVENT_ENVELOPE_V),
                agentId = agentId,
                seq = seq,
                channel = channel,
                event = params.opt("event"),
            ),
        )
    }

    private fun rpc(method: String, params: JSONObject, onOk: (Any?) -> Unit, onErr: (DisplayError) -> Unit) {
        val socket = ws
        if (socket == null || state != ConnState.READY) {
            onErr(normalizeError(mapOf("message" to "not connected")))
            return
        }
        val id = nextId.getAndIncrement()
        val frame = RpcFrames.rpc(id, method, params)
        log("info", "→ $method id=$id", params)
        pending[id] = Pending(resolve = onOk, reject = onErr)
        socket.send(frame.toString())
    }

    fun status(onOk: (String) -> Unit, onErr: (DisplayError) -> Unit) {
        rpc("bot.status", RpcFrames.status(), { r ->
            val runState = (r as? JSONObject)?.optString("runState") ?: "?"
            onOk(runState)
        }, onErr)
    }

    fun roster(onOk: (List<RosterEntry>) -> Unit, onErr: (DisplayError) -> Unit) {
        rpc("bot.roster", RpcFrames.roster(), { r ->
            val agents = mutableListOf<RosterEntry>()
            val arr = (r as? JSONObject)?.optJSONArray("agents")
            if (arr != null) {
                for (i in 0 until arr.length()) {
                    val a = arr.getJSONObject(i)
                    agents.add(
                        RosterEntry(
                            agentId = a.optString("agentId"),
                            name = a.optString("name"),
                            status = a.optString("status"),
                            lastTurnAt = if (a.has("lastTurnAt") && !a.isNull("lastTurnAt")) a.optLong("lastTurnAt") else null,
                        ),
                    )
                }
            }
            onOk(agents)
        }, onErr)
    }

    fun subscribe(agentIds: List<String>, onOk: () -> Unit, onErr: (DisplayError) -> Unit) {
        rpc("bot.subscribe", RpcFrames.subscribe(agentIds), {
            subscribed.addAll(agentIds)
            agentIds.forEach { lastSeq.remove(it) }
            onOk()
        }, onErr)
    }

    fun unsubscribe(agentIds: List<String>, onOk: () -> Unit, onErr: (DisplayError) -> Unit) {
        rpc("bot.unsubscribe", RpcFrames.unsubscribe(agentIds), {
            subscribed.removeAll(agentIds.toSet())
            onOk()
        }, onErr)
    }

    fun command(
        agentId: String,
        name: String,
        args: JSONObject = JSONObject(),
        onOk: (Any?) -> Unit,
        onErr: (DisplayError) -> Unit,
    ) {
        rpc("bot.command", RpcFrames.command(agentId, name, args), onOk, onErr)
    }

    fun sendPrompt(
        agentId: String,
        prompt: String,
        immediate: Boolean = false,
        onOk: (Any?) -> Unit,
        onErr: (DisplayError) -> Unit,
    ) {
        command(agentId, "sendPrompt", RpcFrames.sendPromptArgs(agentId, prompt, immediate), onOk, onErr)
    }

    fun getAgentTranscriptTail(
        agentId: String,
        limit: Int = 20,
        onOk: (Any?) -> Unit,
        onErr: (DisplayError) -> Unit,
    ) {
        command(agentId, "getAgentTranscriptTail", RpcFrames.transcriptTailArgs(agentId, limit), onOk, onErr)
    }

    companion object {
        /** Exposed for UI / tests — agent_id_mismatch must be shown, never silently rewritten. */
        const val AGENT_ID_MISMATCH = COMMAND_REJECTED_AGENT_ID_MISMATCH
    }
}

/** kotlinx-serialization enum SerialName helper for hub channels. */
private fun HubChannel.wireValue(): String = when (this) {
    HubChannel.TurnFinished -> "hub:turn_finished"
    HubChannel.ResyncRequired -> "hub:resync_required"
}
