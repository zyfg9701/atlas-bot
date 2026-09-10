package com.atlasbot.client.ui

import android.content.Context
import android.os.Handler
import android.os.Looper
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExposedDropdownMenuBox
import androidx.compose.material3.ExposedDropdownMenuDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import com.atlasbot.client.BotEventEnvelope
import com.atlasbot.client.ConnState
import com.atlasbot.client.DEFAULT_HUB_WS
import com.atlasbot.client.DisplayError
import com.atlasbot.client.HelloAck
import com.atlasbot.client.HubClient
import com.atlasbot.client.HubClientListener
import com.atlasbot.client.RosterEntry
import com.atlasbot.client.auth.AuthSession
import com.atlasbot.client.auth.DEFAULT_ANDROID_CLIENT_ID
import com.atlasbot.client.auth.MOBILE_REDIRECT_URI
import com.atlasbot.client.auth.OidcClientConfig

private const val PREFS = "atlas_bot_mu1"
private const val KEY_DEBUG_OPEN = "debug_open"

@Composable
fun AtlasBotApp() {
    MaterialTheme {
        Surface(modifier = Modifier.fillMaxSize()) {
            AtlasBotScreen()
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AtlasBotScreen() {
    val context = LocalContext.current
    val auth = remember { AuthSession.obtain(context) }
    val prefs = remember { context.getSharedPreferences(PREFS, Context.MODE_PRIVATE) }

    var hubUrl by remember { mutableStateOf(DEFAULT_HUB_WS) }
    var oidcIssuer by remember { mutableStateOf("http://10.0.2.2:8090") }
    var bearerPaste by remember { mutableStateOf("") }
    var connState by remember { mutableStateOf(ConnState.DISCONNECTED) }
    var stateDetail by remember { mutableStateOf<String?>(null) }
    var capabilities by remember { mutableStateOf("") }
    var connectionId by remember { mutableStateOf("—") }
    var authStatus by remember {
        mutableStateOf(if (auth.hasTicket()) "auth: ticket stored" else "auth: no ticket (dev OK)")
    }
    var runState by remember { mutableStateOf("—") }
    var selectedAgent by remember { mutableStateOf("agt_1") }
    var agentMenuExpanded by remember { mutableStateOf(false) }
    var prompt by remember { mutableStateOf("hello from android") }
    var lastError by remember { mutableStateOf<String?>(null) }
    var advancedOpen by remember { mutableStateOf(false) }
    var debugOpen by remember { mutableStateOf(prefs.getBoolean(KEY_DEBUG_OPEN, false)) }
    var subscribedAgents by remember { mutableStateOf(setOf<String>()) }
    val roster = remember { mutableStateListOf<RosterEntry>() }
    val conversation = remember { mutableStateListOf<String>() }
    val logs = remember { mutableStateListOf<String>() }
    // Signal for turn_finished → auto transcript refresh (listener is created once).
    var pendingTailHint by remember { mutableStateOf("") }
    var pendingTailNonce by remember { mutableStateOf(0) }

    fun appendLog(line: String) {
        logs.add(0, line)
        if (logs.size > 200) logs.removeAt(logs.lastIndex)
    }

    fun appendConversation(line: String) {
        conversation.add(line)
        if (conversation.size > 300) conversation.removeAt(0)
    }

    val mainHandler = remember { Handler(Looper.getMainLooper()) }
    fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else mainHandler.post(block)
    }

    fun persistDebug(open: Boolean) {
        debugOpen = open
        prefs.edit().putBoolean(KEY_DEBUG_OPEN, open).apply()
    }

    val client = remember {
        HubClient(DEFAULT_HUB_WS, object : HubClientListener {
            override fun onState(state: ConnState, detail: String?) {
                onMain {
                    connState = state
                    stateDetail = detail
                    if (state == ConnState.DISCONNECTED) {
                        subscribedAgents = emptySet()
                        connectionId = "—"
                    }
                }
            }
            override fun onLog(level: String, msg: String, data: Any?) {
                onMain { appendLog("[$level] $msg") }
            }
            override fun onHelloAck(ack: HelloAck) {
                onMain {
                    capabilities = ack.capabilities.joinToString(", ")
                    connectionId = ack.connectionId
                    appendLog("hello_ack conn=${ack.connectionId} user=${ack.userId} caps=${ack.capabilities}")
                }
            }
            override fun onEvent(ev: BotEventEnvelope) {
                onMain {
                    val line = "event ${ev.channel} agent=${ev.agentId} seq=${ev.seq}"
                    appendLog(line)
                    appendConversation(line)
                    if (ev.channel == "hub:resync_required") {
                        lastError = "hub:resync_required — re-fetch transcript"
                    }
                    if (ev.channel == "hub:turn_finished") {
                        appendLog("turn_finished — refreshing transcript")
                        pendingTailHint = ev.agentId
                        pendingTailNonce += 1
                    }
                }
            }
            override fun onRpcError(err: DisplayError) {
                onMain {
                    val shown = formatErr(err)
                    lastError = shown
                    appendLog("[error] $shown")
                }
            }
        })
    }

    // Auto-refresh transcript when turn finishes.
    LaunchedEffect(pendingTailNonce) {
        if (pendingTailNonce == 0) return@LaunchedEffect
        val id = pendingTailHint.ifBlank { selectedAgent.trim() }
        if (id.isBlank()) return@LaunchedEffect
        client.getAgentTranscriptTail(
            agentId = id,
            onOk = { r ->
                onMain {
                    val text = r?.toString() ?: "(empty)"
                    appendConversation("— transcript —\n$text")
                    appendLog("transcriptTail refreshed")
                }
            },
            onErr = { e -> onMain { lastError = formatErr(e) } },
        )
    }

    fun refreshTranscript(agentId: String = selectedAgent.trim()) {
        client.getAgentTranscriptTail(
            agentId = agentId,
            onOk = { r ->
                onMain {
                    val text = r?.toString() ?: "(empty)"
                    appendConversation("— transcript —\n$text")
                    appendLog("transcriptTail refreshed")
                }
            },
            onErr = { e -> onMain { lastError = formatErr(e) } },
        )
    }

    fun fetchRosterAfterConnect() {
        client.roster(
            onOk = { agents ->
                onMain {
                    roster.clear()
                    roster.addAll(agents)
                    if (agents.isNotEmpty()) {
                        val keep = agents.any { it.agentId == selectedAgent }
                        if (!keep) selectedAgent = agents.first().agentId
                    }
                    appendLog("roster ${agents.size} agents (auto)")
                }
            },
            onErr = { e -> onMain { appendLog("roster after connect failed (non-fatal): ${e.message}") } },
        )
    }

    fun doSubscribe(id: String, then: (() -> Unit)? = null) {
        client.subscribe(
            listOf(id),
            onOk = {
                onMain {
                    subscribedAgents = subscribedAgents + id
                    appendLog("subscribed $id")
                    then?.invoke()
                }
            },
            onErr = { e -> onMain { lastError = formatErr(e) } },
        )
    }

    fun doSendPrompt() {
        val id = selectedAgent.trim()
        if (id.isEmpty()) {
            lastError = "agentId required"
            return
        }
        val send = {
            client.sendPrompt(
                agentId = id,
                prompt = prompt,
                immediate = true,
                onOk = {
                    onMain {
                        appendConversation("you: $prompt")
                        appendLog("sendPrompt ok")
                    }
                },
                onErr = { e -> onMain { lastError = formatErr(e) } },
            )
        }
        if (id !in subscribedAgents) {
            doSubscribe(id, then = send)
        } else {
            send()
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(12.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("Atlas Bot · Chat", style = MaterialTheme.typography.titleLarge)
        Text(
            "State: $connState${stateDetail?.let { " ($it)" } ?: ""}",
            style = MaterialTheme.typography.bodyMedium,
        )
        Text(authStatus, style = MaterialTheme.typography.bodySmall)
        if (connState == ConnState.DISCONNECTED) {
            Text(
                "开发态：dev Hub 无票可连 · Login 仍在主路径",
                style = MaterialTheme.typography.bodySmall,
                color = Color.Gray,
            )
        }

        Row(
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Button(onClick = {
                lastError = null
                client.setUrl(hubUrl.trim())
                val bearer = auth.currentBearer() ?: bearerPaste.trim().ifEmpty { null }
                client.setAuthorization(bearer)
                client.connect(
                    onReady = { ack ->
                        onMain {
                            appendLog("ready ${ack.connectionId} user=${ack.userId}")
                            fetchRosterAfterConnect()
                        }
                    },
                    onFail = { err -> onMain { lastError = err.message } },
                )
            }) { Text("Connect") }
            OutlinedButton(onClick = { client.disconnect() }) { Text("Disconnect") }
        }
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                lastError = null
                val issuer = oidcIssuer.trim()
                if (issuer.isEmpty()) {
                    lastError = "OIDC issuer required"
                    return@Button
                }
                appendLog("login → Custom Tabs PKCE ($MOBILE_REDIRECT_URI)")
                auth.startLogin(
                    context = context,
                    cfg = OidcClientConfig(issuer = issuer, clientId = DEFAULT_ANDROID_CLIENT_ID),
                ) { result ->
                    onMain {
                        result.onSuccess {
                            authStatus = "auth: ticket stored (id_token preferred)"
                            appendLog("login ok — Connect will send Authorization: Bearer")
                        }.onFailure { e ->
                            lastError = e.message
                            appendLog("login failed: ${e.message}")
                        }
                    }
                }
            }) { Text("Login") }
            OutlinedButton(onClick = {
                auth.logout()
                client.setAuthorization(null)
                client.disconnect()
                authStatus = "auth: logged out"
                appendLog("logout — token store cleared")
            }) { Text("Logout") }
        }

        CollapsibleSection(
            title = if (advancedOpen) "高级 · Hub URL / OIDC ▴" else "高级 · Hub URL / OIDC ▾",
            open = advancedOpen,
            onToggle = { advancedOpen = !advancedOpen },
        ) {
            OutlinedTextField(
                value = hubUrl,
                onValueChange = { hubUrl = it },
                label = { Text("Hub WS URL") },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
            )
            OutlinedTextField(
                value = oidcIssuer,
                onValueChange = { oidcIssuer = it },
                label = { Text("OIDC issuer (I2.2)") },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
            )
            OutlinedTextField(
                value = bearerPaste,
                onValueChange = { bearerPaste = it },
                label = { Text("Paste Bearer (optional)") },
                modifier = Modifier.fillMaxWidth(),
                singleLine = true,
            )
            Text(
                "Redirect: $MOBILE_REDIRECT_URI · client_id=$DEFAULT_ANDROID_CLIENT_ID · no WebView",
                style = MaterialTheme.typography.bodySmall,
            )
        }

        if (lastError != null) {
            Text("Protocol error: $lastError", color = Color(0xFFB00020))
        }

        Text("Agent", style = MaterialTheme.typography.titleMedium)
        ExposedDropdownMenuBox(
            expanded = agentMenuExpanded,
            onExpandedChange = { agentMenuExpanded = it },
        ) {
            OutlinedTextField(
                value = selectedAgent,
                onValueChange = { selectedAgent = it },
                label = { Text("agentId (picker or hand-fill)") },
                modifier = Modifier
                    .menuAnchor()
                    .fillMaxWidth(),
                trailingIcon = { ExposedDropdownMenuDefaults.TrailingIcon(expanded = agentMenuExpanded) },
                singleLine = true,
            )
            ExposedDropdownMenu(
                expanded = agentMenuExpanded,
                onDismissRequest = { agentMenuExpanded = false },
            ) {
                if (roster.isEmpty()) {
                    DropdownMenuItem(
                        text = { Text("(roster empty — Connect then refresh)") },
                        onClick = { agentMenuExpanded = false },
                    )
                } else {
                    roster.forEach { a ->
                        DropdownMenuItem(
                            text = { Text("${a.agentId}  ${a.name}  (${a.status})") },
                            onClick = {
                                selectedAgent = a.agentId
                                agentMenuExpanded = false
                            },
                        )
                    }
                }
            }
        }
        Text(
            "Connect 后自动 roster；Send 时未订则自动 subscribe。",
            style = MaterialTheme.typography.bodySmall,
            color = Color.Gray,
        )

        Text("Conversation", style = MaterialTheme.typography.titleMedium)
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 120.dp, max = 280.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            if (conversation.isEmpty()) {
                Text(
                    "(empty — Connect → Send；turn_finished 后自动刷新 transcript)",
                    style = MaterialTheme.typography.bodySmall,
                    color = Color.Gray,
                )
            } else {
                conversation.takeLast(80).forEach { line ->
                    Text(line, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
                }
            }
        }

        OutlinedTextField(
            value = prompt,
            onValueChange = { prompt = it },
            label = { Text("prompt") },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                lastError = null
                doSendPrompt()
            }) { Text("Send") }
            Text(
                "🖥📎 未接线 — 见调试占位",
                style = MaterialTheme.typography.bodySmall,
                color = Color.Gray,
                modifier = Modifier.align(Alignment.CenterVertically),
            )
        }

        Spacer(Modifier.height(4.dp))

        CollapsibleSection(
            title = if (debugOpen) "更多 / 调试 ▴" else "更多 / 调试 ▾",
            open = debugOpen,
            onToggle = { persistDebug(!debugOpen) },
        ) {
            Text("Cold path", style = MaterialTheme.typography.titleSmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = {
                    client.status(
                        onOk = { v -> onMain { runState = v; appendLog("status runState=$v") } },
                        onErr = { e -> onMain { lastError = e.message } },
                    )
                }) { Text("status") }
                Button(onClick = {
                    client.roster(
                        onOk = { agents ->
                            onMain {
                                roster.clear()
                                roster.addAll(agents)
                                if (agents.isNotEmpty() && selectedAgent.isBlank()) {
                                    selectedAgent = agents.first().agentId
                                }
                                appendLog("roster ${agents.size} agents")
                            }
                        },
                        onErr = { e -> onMain { lastError = e.message } },
                    )
                }) { Text("roster") }
            }
            Text("runState: $runState", style = MaterialTheme.typography.bodySmall)
            roster.forEach { a ->
                Text("• ${a.agentId}  ${a.name}  (${a.status})", style = MaterialTheme.typography.bodySmall)
            }

            Spacer(Modifier.height(4.dp))
            Text("Protocol · hot path", style = MaterialTheme.typography.titleSmall)
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(onClick = { doSubscribe(selectedAgent.trim()) }) { Text("subscribe") }
                OutlinedButton(onClick = {
                    val id = selectedAgent.trim()
                    client.unsubscribe(
                        listOf(id),
                        onOk = {
                            onMain {
                                subscribedAgents = subscribedAgents - id
                                appendLog("unsubscribed $id")
                            }
                        },
                        onErr = { e -> onMain { lastError = formatErr(e) } },
                    )
                }) { Text("unsubscribe") }
                Button(onClick = { refreshTranscript() }) { Text("transcriptTail") }
            }

            Spacer(Modifier.height(4.dp))
            Text("Connection", style = MaterialTheme.typography.titleSmall)
            Text(
                "capabilities: ${capabilities.ifBlank { "—" }}",
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
            Text(
                "connection_id: $connectionId",
                style = MaterialTheme.typography.bodySmall,
                fontFamily = FontFamily.Monospace,
            )
            Text(
                "Redirect: $MOBILE_REDIRECT_URI · no WebView primary",
                style = MaterialTheme.typography.bodySmall,
            )

            Spacer(Modifier.height(4.dp))
            Text("Desktop · upload", style = MaterialTheme.typography.titleSmall)
            Text(
                "未接线（MU1 占位）— 不砍未来 VNC/upload 约定；PC U1 已有产品入口。",
                style = MaterialTheme.typography.bodySmall,
                color = Color.Gray,
            )

            Spacer(Modifier.height(4.dp))
            Text("Log", style = MaterialTheme.typography.titleSmall)
            logs.take(40).forEach {
                Text(it, style = MaterialTheme.typography.bodySmall, fontFamily = FontFamily.Monospace)
            }
        }
    }
}

@Composable
private fun CollapsibleSection(
    title: String,
    open: Boolean,
    onToggle: () -> Unit,
    content: @Composable () -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        TextButton(onClick = onToggle, modifier = Modifier.fillMaxWidth()) {
            Text(title, modifier = Modifier.fillMaxWidth())
        }
        if (open) content()
    }
}

private fun formatErr(err: DisplayError): String = buildString {
    append(err.message)
    err.reason?.let { append(" reason=$it") }
    err.code?.let { append(" code=$it") }
}
