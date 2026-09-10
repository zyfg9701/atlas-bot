package com.atlasbot.client.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import android.os.Handler
import android.os.Looper
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
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

@Composable
fun AtlasBotApp() {
    MaterialTheme {
        Surface(modifier = Modifier.fillMaxSize()) {
            AtlasBotScreen()
        }
    }
}

@Composable
private fun AtlasBotScreen() {
    val context = LocalContext.current
    val auth = remember { AuthSession.obtain(context) }

    var hubUrl by remember { mutableStateOf(DEFAULT_HUB_WS) }
    var oidcIssuer by remember { mutableStateOf("http://10.0.2.2:8090") }
    var connState by remember { mutableStateOf(ConnState.DISCONNECTED) }
    var stateDetail by remember { mutableStateOf<String?>(null) }
    var capabilities by remember { mutableStateOf("") }
    var authStatus by remember {
        mutableStateOf(if (auth.hasTicket()) "auth: ticket stored" else "auth: no ticket (dev OK)")
    }
    var runState by remember { mutableStateOf("—") }
    var selectedAgent by remember { mutableStateOf("agt_1") }
    var prompt by remember { mutableStateOf("hello from android") }
    var transcript by remember { mutableStateOf("") }
    var lastError by remember { mutableStateOf<String?>(null) }
    val roster = remember { mutableStateListOf<RosterEntry>() }
    val logs = remember { mutableStateListOf<String>() }

    fun appendLog(line: String) {
        logs.add(0, line)
        if (logs.size > 200) logs.removeAt(logs.lastIndex)
    }

    val mainHandler = remember { Handler(Looper.getMainLooper()) }
    fun onMain(block: () -> Unit) {
        if (Looper.myLooper() == Looper.getMainLooper()) block() else mainHandler.post(block)
    }

    val client = remember {
        HubClient(DEFAULT_HUB_WS, object : HubClientListener {
            override fun onState(state: ConnState, detail: String?) {
                onMain {
                    connState = state
                    stateDetail = detail
                }
            }
            override fun onLog(level: String, msg: String, data: Any?) {
                onMain { appendLog("[$level] $msg") }
            }
            override fun onHelloAck(ack: HelloAck) {
                onMain {
                    capabilities = ack.capabilities.joinToString(", ")
                    appendLog("hello_ack conn=${ack.connectionId} user=${ack.userId} caps=${ack.capabilities}")
                }
            }
            override fun onEvent(ev: BotEventEnvelope) {
                onMain {
                    appendLog("event ${ev.channel} agent=${ev.agentId} seq=${ev.seq}")
                    if (ev.channel == "hub:resync_required") {
                        lastError = "hub:resync_required — re-fetch transcript"
                    }
                    if (ev.channel == "hub:turn_finished") {
                        appendLog("turn_finished — tap transcriptTail to refresh")
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

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(16.dp)
            .verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text("Atlas Bot P4 (Android)", style = MaterialTheme.typography.titleLarge)
        Text("State: $connState${stateDetail?.let { " ($it)" } ?: ""}")
        Text(authStatus, style = MaterialTheme.typography.bodySmall)
        if (lastError != null) {
            Text("Protocol error: $lastError", color = Color(0xFFB00020))
        }

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
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                lastError = null
                client.setUrl(hubUrl.trim())
                client.setAuthorization(auth.currentBearer())
                client.connect(
                    onReady = { ack -> onMain { appendLog("ready ${ack.connectionId} user=${ack.userId}") } },
                    onFail = { err -> onMain { lastError = err.message } },
                )
            }) { Text("Connect") }
            Button(onClick = { client.disconnect() }) { Text("Disconnect") }
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
            Button(onClick = {
                auth.logout()
                client.setAuthorization(null)
                client.disconnect()
                authStatus = "auth: logged out"
                appendLog("logout — token store cleared")
            }) { Text("Logout") }
        }
        Text(
            "Redirect: $MOBILE_REDIRECT_URI · client_id=$DEFAULT_ANDROID_CLIENT_ID · no WebView",
            style = MaterialTheme.typography.bodySmall,
        )
        if (capabilities.isNotEmpty()) {
            Text("Capabilities: $capabilities", style = MaterialTheme.typography.bodySmall)
        }

        Text("Cold path", style = MaterialTheme.typography.titleMedium)
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
        Text("runState: $runState")
        roster.forEach { a ->
            Text("• ${a.agentId}  ${a.name}  (${a.status})")
        }

        Text("Hot path", style = MaterialTheme.typography.titleMedium)
        OutlinedTextField(
            value = selectedAgent,
            onValueChange = { selectedAgent = it },
            label = { Text("agentId") },
            modifier = Modifier.fillMaxWidth(),
            singleLine = true,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                val id = selectedAgent.trim()
                client.subscribe(
                    listOf(id),
                    onOk = { onMain { appendLog("subscribed $id") } },
                    onErr = { e -> onMain { lastError = formatErr(e) } },
                )
            }) { Text("subscribe") }
            Button(onClick = {
                val id = selectedAgent.trim()
                client.unsubscribe(
                    listOf(id),
                    onOk = { onMain { appendLog("unsubscribed $id") } },
                    onErr = { e -> onMain { lastError = formatErr(e) } },
                )
            }) { Text("unsubscribe") }
        }
        OutlinedTextField(
            value = prompt,
            onValueChange = { prompt = it },
            label = { Text("prompt") },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            Button(onClick = {
                val id = selectedAgent.trim()
                client.sendPrompt(
                    agentId = id,
                    prompt = prompt,
                    immediate = true,
                    onOk = { onMain { appendLog("sendPrompt ok") } },
                    onErr = { e -> onMain { lastError = formatErr(e) } },
                )
            }) { Text("sendPrompt") }
            Button(onClick = {
                val id = selectedAgent.trim()
                client.getAgentTranscriptTail(
                    agentId = id,
                    onOk = { r ->
                        onMain {
                            transcript = r?.toString() ?: "(empty)"
                            appendLog("transcriptTail refreshed")
                        }
                    },
                    onErr = { e -> onMain { lastError = formatErr(e) } },
                )
            }) { Text("transcriptTail") }
        }
        Text("Transcript", style = MaterialTheme.typography.titleMedium)
        Text(transcript.ifBlank { "(none yet — wait for hub:turn_finished then refresh)" })

        Spacer(Modifier.height(8.dp))
        Text("Log", style = MaterialTheme.typography.titleMedium)
        logs.take(40).forEach { Text(it, style = MaterialTheme.typography.bodySmall) }
    }
}

private fun formatErr(err: DisplayError): String = buildString {
    append(err.message)
    err.reason?.let { append(" reason=$it") }
    err.code?.let { append(" code=$it") }
}
