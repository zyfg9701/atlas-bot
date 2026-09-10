import "./styles.css";
import {
  DEFAULT_HUB_WS,
  HubClient,
  sortEventsBySeq,
  type BotEventEnvelope,
  type ConnState,
  type DisplayError,
  type HelloAck,
  type RosterEntry,
} from "./hubClient";
import {
  buildAuthorizeUrl,
  detectTauri,
  generatePkce,
  parseCallbackUrl,
  pickHubBearer,
  exchangeCode,
} from "./authClient";

const app = document.querySelector("#app")!;

const wantsDebugOpen = (() => {
  try {
    if (new URLSearchParams(location.search).get("debug") === "1") return true;
    if (localStorage.getItem("atlas-pc-debug") === "1") return true;
  } catch {
    /* ignore */
  }
  return false;
})();

app.innerHTML = `
  <header class="topbar">
    <div class="topbar-brand">
      <strong>atlas-bot</strong>
      <span class="topbar-sub">PC · Chat</span>
    </div>
    <div class="topbar-actions">
      <span id="state" class="badge disconnected">disconnected</span>
      <button id="btnConnect" type="button">Connect</button>
      <button id="btnDisconnect" class="secondary" type="button">Disconnect</button>
      <button id="btnLogin" type="button" title="OIDC PKCE (non-dev Hub)">Login</button>
      <button id="btnLogout" class="secondary" type="button">Logout</button>
    </div>
  </header>

  <details class="advanced" id="advancedHub">
    <summary>高级 · Hub URL / Bearer</summary>
    <div class="row">
      <label for="url">Hub WS</label>
      <input id="url" value="${DEFAULT_HUB_WS}" />
    </div>
    <div class="row">
      <label for="tokenPaste">Bearer</label>
      <input id="tokenPaste" placeholder="optional paste token (dev) or after Login" />
    </div>
    <div class="mono" id="authOut">auth: (none) · Tauri Bearer WS for production oidc/static</div>
  </details>

  <div id="fail" class="fail-banner" role="alert"></div>

  <main class="chat-layout">
    <aside class="agent-pane panel">
      <h2>Agent</h2>
      <div class="row">
        <select id="agentId" aria-label="Select agent"></select>
      </div>
      <div class="row">
        <input id="newName" placeholder="new agent name" value="Scout" />
        <button id="btnCreateMain" type="button">Create</button>
      </div>
      <p class="hint mono">Connect 后自动 listAgents；发送时自动 subscribe。</p>
    </aside>

    <section class="chat-pane panel">
      <h2>Conversation</h2>
      <div id="conversation" class="conversation mono" aria-live="polite"></div>
      <div class="composer">
        <textarea id="prompt" placeholder="Write a prompt…" rows="3">hello U1</textarea>
        <div class="composer-actions">
          <button id="btnSend" type="button">Send</button>
          <button id="btnInterrupt" class="secondary" type="button">Interrupt</button>
          <label class="mono immediate-label"><input type="checkbox" id="immediate" /> immediate</label>
          <div class="composer-tools">
            <button id="btnAttachIcon" class="icon-btn secondary" type="button" title="Attach file (debug upload)">📎</button>
            <button id="btnVncIcon" class="icon-btn secondary" type="button" title="Open desktop">🖥</button>
            <input id="filePickMain" type="file" class="sr-only" />
          </div>
        </div>
      </div>
    </section>
  </main>

  <details class="debug-accordion" id="debugPanel"${wantsDebugOpen ? " open" : ""}>
    <summary>更多 / 调试 ▾</summary>
    <div class="debug-body">
      <div class="debug-grid">
        <div class="panel debug-section">
          <h2>Cold path</h2>
          <div class="row">
            <button id="btnStatus" type="button">status</button>
            <button id="btnRoster" type="button">roster</button>
            <button id="btnOffbox" type="button">offbox</button>
            <button id="btnOffboxNext" class="secondary" type="button">offbox next</button>
          </div>
          <div class="mono" id="statusOut">runState: —</div>
          <ul class="agents" id="roster"></ul>
          <div class="mono" id="offboxCursor">cursor: (first page)</div>
          <div id="offbox" class="mono scrollbox"></div>
        </div>

        <div class="panel debug-section">
          <h2>Protocol · hot path</h2>
          <div class="row">
            <button id="btnSub" type="button">subscribe</button>
            <button id="btnUnsub" class="secondary" type="button">unsubscribe</button>
            <button id="btnTail" type="button">getTail</button>
            <button id="btnList" type="button">listAgents</button>
          </div>
          <div class="row">
            <button id="btnCreate" type="button">createAgent</button>
            <span class="hint mono">（主栏 Create 同逻辑）</span>
          </div>
          <div class="mono" id="listOut"></div>
          <div class="mono" id="caps">capabilities: —</div>
          <div class="mono" id="connMeta">connection_id: —</div>
        </div>

        <div class="panel debug-section">
          <h2>CLI primary stack</h2>
          <p class="hint mono">主路径：先起栈再 Connect。详见 docs/cli-primary-runbook.md</p>
          <div class="row">
            <button id="btnCopyStackCmd" type="button" title="Copy primary start commands">复制起栈命令</button>
            <button id="btnProbeGw" class="secondary" type="button" title="GET http://127.0.0.1:8787/healthz">探 gateway healthz</button>
          </div>
          <div class="mono" id="gwHealthOut">gateway: —</div>
          <textarea id="stackCmdBox" class="mono" rows="4" readonly style="width:100%;margin-top:6px;font-size:12px">ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh
# 真机: ./scripts/dev-cli-stack.sh
# 然后 PC Connect → ws://127.0.0.1:7700/ws → Send</textarea>
        </div>

        <div class="panel debug-section">
          <h2>Desktop · attachments</h2>
          <div class="row">
            <button id="btnVnc" type="button">Open desktop</button>
            <span class="mono" id="vncOut">vnc: —</span>
          </div>
          <div class="row">
            <input id="filePick" type="file" />
            <button id="btnUpload" type="button">uploadAttachment</button>
            <button id="btnAttach" class="secondary" type="button">attachUpload (last id)</button>
          </div>
          <div class="mono" id="uploadOut">upload path: —</div>
          <p class="hint mono">UI caps file pick at 1.5 MiB (args JSON hard limit 3 MiB → reason args_too_large).</p>
        </div>
      </div>

      <div class="panel debug-section" style="margin-top:12px">
        <h2>Raw panes (transcript / events) · Log</h2>
        <div class="debug-split">
          <div>
            <h3 class="subh">Events</h3>
            <div id="events" class="mono scrollbox"></div>
          </div>
          <div>
            <h3 class="subh">Hot transcript tail</h3>
            <div id="transcript" class="mono scrollbox"></div>
          </div>
        </div>
        <h3 class="subh">Log</h3>
        <div id="log" class="mono scrollbox logbox"></div>
        <p class="hint mono">Tip: <code>?debug=1</code> or localStorage <code>atlas-pc-debug=1</code> opens this panel on load.</p>
      </div>
    </div>
  </details>
`;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const urlEl = $<HTMLInputElement>("url");
const stateEl = $("state");
const capsEl = $("caps");
const connMetaEl = $("connMeta");
const statusOut = $("statusOut");
const rosterEl = $("roster");
const agentEl = $<HTMLSelectElement>("agentId");
const promptEl = $<HTMLTextAreaElement>("prompt");
const newNameEl = $<HTMLInputElement>("newName");
const immediateEl = $<HTMLInputElement>("immediate");
const logEl = $("log");
const eventsEl = $("events");
const transcriptEl = $("transcript");
const conversationEl = $("conversation");
const offboxEl = $("offbox");
const offboxCursorEl = $("offboxCursor");
const listOut = $("listOut");
const failEl = $("fail");
const vncOut = $("vncOut");
const uploadOut = $("uploadOut");
const filePick = $<HTMLInputElement>("filePick");
const filePickMain = $<HTMLInputElement>("filePickMain");
const tokenPaste = $<HTMLInputElement>("tokenPaste");
const authOut = $("authOut");
const debugPanel = $<HTMLDetailsElement>("debugPanel");
const gwHealthOut = $("gwHealthOut");
const stackCmdBox = $<HTMLTextAreaElement>("stackCmdBox");

let sessionToken: string | undefined;

const events: BotEventEnvelope[] = [];
let client: HubClient | null = null;
let knownAgents: { id: string; name: string }[] = [{ id: "agt_1", name: "Watcher" }];
let offboxNextCursor: string | null | undefined = undefined;
let lastRoster: RosterEntry[] = [];
let lastUploadId: string | null = null;
let lastTailRaw: unknown = null;

/** PC-side soft cap — far below gateway 3 MiB args JSON limit. */
const UI_UPLOAD_MAX_BYTES = Math.floor(1.5 * 1024 * 1024);

debugPanel.addEventListener("toggle", () => {
  try {
    localStorage.setItem("atlas-pc-debug", debugPanel.open ? "1" : "0");
  } catch {
    /* ignore */
  }
});

const PRIMARY_STACK_CMD = `ATLAS_AGENT_CLI=tools/mock-cli/mock-atlas-agent-cli.sh ./scripts/dev-cli-stack.sh
# real agent on PATH:
# ./scripts/dev-cli-stack.sh
# then: Connect → ws://127.0.0.1:7700/ws → Send
# docs/cli-primary-runbook.md`;

$("btnCopyStackCmd").onclick = async () => {
  stackCmdBox.value = PRIMARY_STACK_CMD;
  try {
    await navigator.clipboard.writeText(PRIMARY_STACK_CMD);
    gwHealthOut.textContent = "gateway: (copied start commands to clipboard)";
    appendLog("info", "copied CLI primary stack commands");
  } catch {
    stackCmdBox.select();
    gwHealthOut.textContent = "gateway: (select+copy the commands below — clipboard unavailable)";
  }
};

$("btnProbeGw").onclick = async () => {
  const url = "http://127.0.0.1:8787/healthz";
  gwHealthOut.textContent = `gateway: probing ${url} …`;
  try {
    const ctrl = new AbortController();
    const t = window.setTimeout(() => ctrl.abort(), 2500);
    const res = await fetch(url, { signal: ctrl.signal, cache: "no-store" });
    window.clearTimeout(t);
    const textBody = await res.text();
    let summary = `HTTP ${res.status}`;
    try {
      const j = JSON.parse(textBody) as {
        ok?: boolean;
        backend?: string;
        agent_cli?: string;
        agent_cli_found?: boolean;
      };
      summary = [
        `backend=${j.backend ?? "?"}`,
        `agent_cli_found=${j.agent_cli_found ?? "?"}`,
        j.agent_cli ? `agent_cli=${j.agent_cli}` : null,
        j.ok === false ? "ok=false" : null,
      ]
        .filter(Boolean)
        .join(" · ");
    } catch {
      summary = `${summary} body=${textBody.slice(0, 120)}`;
    }
    gwHealthOut.textContent = `gateway: ${summary}`;
    appendLog("info", `healthz ${url} → ${summary}`);
  } catch (e) {
    const msg = e instanceof Error ? e.message : String(e);
    gwHealthOut.textContent =
      `gateway: unreachable (${msg}). Start ./scripts/dev-cli-stack.sh first — loopback only, no WAN.`;
    appendLog("warn", `healthz failed: ${msg}`);
  }
};

function showFail(err: DisplayError) {
  const bits = [
    err.message,
    err.code != null ? `code=${err.code}` : null,
    err.reason ? `reason=${err.reason}` : null,
    err.retryable === false ? "do-not-blind-retry" : null,
  ].filter(Boolean);
  failEl.textContent = `Failure: ${bits.join(" · ")}`;
  failEl.classList.add("show");
  if (err.reason) {
    appendLog("error", `protocol error reason=${err.reason}`, err);
  }
}

function clearFail() {
  failEl.classList.remove("show");
  failEl.textContent = "";
}

function appendLog(level: string, msg: string, data?: unknown) {
  const line = document.createElement("div");
  line.className = `log-${level}`;
  const ts = new Date().toLocaleTimeString();
  line.textContent =
    `[${ts}] ${msg}` + (data !== undefined ? " " + JSON.stringify(data) : "");
  logEl.prepend(line);
}

function setStateBadge(s: ConnState) {
  stateEl.textContent = s;
  stateEl.className = `badge ${s}`;
}

function currentAgentId(): string {
  return agentEl.value || "agt_1";
}

function renderAgentSelect() {
  const prev = agentEl.value;
  agentEl.innerHTML = "";
  for (const a of knownAgents) {
    const opt = document.createElement("option");
    opt.value = a.id;
    opt.textContent = `${a.id} · ${a.name}`;
    agentEl.appendChild(opt);
  }
  if (prev && knownAgents.some((a) => a.id === prev)) {
    agentEl.value = prev;
  } else if (knownAgents.length) {
    agentEl.value = knownAgents[0].id;
  }
}

function mergeKnownFromRoster(agents: RosterEntry[]) {
  for (const a of agents) {
    const idx = knownAgents.findIndex((x) => x.id === a.agentId);
    if (idx >= 0) knownAgents[idx] = { id: a.agentId, name: a.name };
    else knownAgents.push({ id: a.agentId, name: a.name });
  }
  knownAgents.sort((a, b) => a.id.localeCompare(b.id));
  renderAgentSelect();
}

function mergeKnownFromList(list: unknown) {
  if (!Array.isArray(list)) return;
  for (const row of list) {
    if (!row || typeof row !== "object") continue;
    const o = row as Record<string, unknown>;
    const id = String(o.id ?? "");
    const name = String(o.name ?? id);
    if (!id) continue;
    const idx = knownAgents.findIndex((x) => x.id === id);
    if (idx >= 0) knownAgents[idx] = { id, name };
    else knownAgents.push({ id, name });
  }
  knownAgents.sort((a, b) => a.id.localeCompare(b.id));
  renderAgentSelect();
}

function formatEventLine(e: BotEventEnvelope): string {
  const body =
    typeof e.event === "object" && e.event !== null
      ? JSON.stringify(e.event)
      : String(e.event ?? "");
  return `▸ #${e.seq} ${e.channel} ${body}`;
}

function renderConversation() {
  const aid = currentAgentId();
  const parts: string[] = [];

  if (lastTailRaw != null) {
    parts.push("—— transcript ——");
    try {
      parts.push(
        typeof lastTailRaw === "string"
          ? lastTailRaw
          : JSON.stringify(lastTailRaw, null, 2),
      );
    } catch {
      parts.push(String(lastTailRaw));
    }
  }

  const sorted = sortEventsBySeq(events.filter((e) => e.agentId === aid));
  if (sorted.length) {
    parts.push("—— live events ——");
    for (const e of sorted) parts.push(formatEventLine(e));
  }

  if (!parts.length) {
    conversationEl.textContent =
      "Connect → pick agent → Send. Live events and transcript appear here.";
    return;
  }
  conversationEl.textContent = parts.join("\n");
  conversationEl.scrollTop = conversationEl.scrollHeight;
}

function renderEvents() {
  const aid = currentAgentId();
  const sorted = sortEventsBySeq(events.filter((e) => e.agentId === aid));
  eventsEl.textContent = sorted
    .map((e) => `#${e.seq} ${e.agentId} ${e.channel} ${JSON.stringify(e.event)}`)
    .join("\n");
  renderConversation();
}

function renderRoster(agents: RosterEntry[]) {
  lastRoster = agents;
  rosterEl.innerHTML = "";
  for (const a of agents) {
    const li = document.createElement("li");
    li.textContent = `${a.agentId} · ${a.name} · ${a.status}`;
    if (a.agentId === currentAgentId()) li.classList.add("selected");
    li.onclick = () => {
      agentEl.value = a.agentId;
      offboxNextCursor = undefined;
      offboxCursorEl.textContent = "cursor: (first page)";
      offboxEl.textContent = "";
      renderRoster(lastRoster);
      renderEvents();
      void refreshTail();
    };
    rosterEl.appendChild(li);
  }
}

function ensureClient(): HubClient {
  if (!client) {
    client = new HubClient(urlEl.value, {
      onState: (s, d) => {
        setStateBadge(s);
        if (d) appendLog("info", `state=${s}`, d);
      },
      onLog: (level, msg, data) => appendLog(level, msg, data),
      onHelloAck: (ack: HelloAck) => {
        capsEl.textContent =
          "capabilities: " + (ack.capabilities?.join(", ") || "(none)");
        connMetaEl.textContent = `connection_id: ${ack.connection_id} · user_id=${ack.user_id}`;
      },
      onEvent: (ev) => {
        events.push(ev);
        renderEvents();
        if (ev.agentId !== currentAgentId()) {
          appendLog("event", `event for other agent ${ev.agentId} (not mixed into current view)`);
          return;
        }
        if (ev.channel === "hub:turn_finished") {
          appendLog("event", "turn finished — fetching transcript tail");
          void refreshTail();
        }
        if (ev.channel === "hub:resync_required") {
          appendLog("warn", "explicit resync_required — re-fetching offbox + tail");
          offboxNextCursor = undefined;
          void loadOffbox(undefined);
          void refreshTail();
        }
      },
      onRpcError: (err) => showFail(err),
    });
  } else {
    client.setUrl(urlEl.value);
  }
  return client;
}

async function refreshTail() {
  const c = client;
  if (!c || c.connectionState !== "ready") return;
  try {
    const r = await c.getAgentTranscriptTail(currentAgentId(), 20);
    lastTailRaw = r;
    transcriptEl.textContent = JSON.stringify(r, null, 2);
    renderConversation();
  } catch (e) {
    showFail(e as DisplayError);
  }
}

async function loadOffbox(cursor?: string | null) {
  const c = ensureClient();
  try {
    const page = await c.transcriptOffbox(currentAgentId(), cursor ?? undefined);
    offboxNextCursor = page.nextCursor ?? null;
    offboxCursorEl.textContent = cursor
      ? `cursor: ${cursor} → next=${offboxNextCursor ?? "(end)"}`
      : `cursor: (first page) → next=${offboxNextCursor ?? "(end)"}`;
    offboxEl.textContent = JSON.stringify(page, null, 2);
  } catch (e) {
    showFail(e as DisplayError);
  }
}

async function ensureSubscribed(agentId: string) {
  const c = ensureClient();
  if (c.subscribedAgents.includes(agentId)) return;
  await c.subscribe([agentId]);
  appendLog("info", `auto-subscribe ${agentId}`);
}

async function doListAgents() {
  const c = ensureClient();
  const r = await c.listAgents(currentAgentId());
  listOut.textContent = JSON.stringify(r, null, 2);
  mergeKnownFromList(r);
  appendLog("info", "listAgents result", r);
  return r;
}

async function doCreateAgent() {
  const name = newNameEl.value.trim() || "Agent";
  const r = (await ensureClient().createAgent(currentAgentId(), name)) as {
    agentId?: string;
    agent?: { id?: string; name?: string };
  };
  appendLog("info", "createAgent result", r);
  const id = r.agentId || r.agent?.id;
  if (id) {
    mergeKnownFromList([{ id, name: r.agent?.name || name }]);
    agentEl.value = id;
    const ro = await ensureClient().roster();
    mergeKnownFromRoster(ro.agents || []);
    renderRoster(ro.agents || []);
    await ensureClient().subscribe([id]);
  }
}

async function doVnc() {
  const r = await ensureClient().vncDescriptor(currentAgentId());
  const exp =
    r.expiresHint == null
      ? "expiresHint=null"
      : `expiresHint=${r.expiresHint} (${new Date(r.expiresHint).toLocaleString()})`;
  vncOut.textContent = `vncUrl=${r.vncUrl} · ${exp}`;
  appendLog("info", "bot.vncDescriptor", r);
  window.open(r.vncUrl, "_blank", "noopener,noreferrer");
}

async function doUpload(fileInput: HTMLInputElement) {
  const file = fileInput.files?.[0];
  if (!file) {
    showFail({ message: "pick a file first", code: "upstream_error" });
    return;
  }
  if (file.size > UI_UPLOAD_MAX_BYTES) {
    showFail({
      message: "command_rejected",
      code: "command_rejected",
      reason: "args_too_large",
      retryable: false,
    });
    appendLog("warn", `UI rejected file ${file.size} bytes > ${UI_UPLOAD_MAX_BYTES}`);
    return;
  }
  const b64 = await readFileAsBase64(file);
  const r = await ensureClient().uploadAttachment(currentAgentId(), file.name, b64);
  lastUploadId = r.uploadId || null;
  uploadOut.textContent = `upload path: ${r.path}` + (lastUploadId ? ` · uploadId=${lastUploadId}` : "");
  appendLog("info", "uploadAttachment", r);
}

renderAgentSelect();
renderConversation();

agentEl.onchange = () => {
  offboxNextCursor = undefined;
  offboxCursorEl.textContent = "cursor: (first page)";
  offboxEl.textContent = "";
  lastTailRaw = null;
  transcriptEl.textContent = "";
  renderRoster(lastRoster);
  renderEvents();
  void refreshTail();
};

$("btnConnect").onclick = async () => {
  clearFail();
  events.length = 0;
  lastTailRaw = null;
  renderEvents();
  const pasted = tokenPaste.value.trim() || sessionToken;
  if (pasted) sessionToken = pasted;
  const c = ensureClient();
  c.setUrl(urlEl.value);
  c.setAuthorization(sessionToken);

  // Production path: Tauri rust WS with Authorization header (browser cannot).
  if (sessionToken && detectTauri()) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const r = await invoke<{ ok: boolean; detail: string }>("connect_ws", {
        url: urlEl.value,
        authorization: sessionToken,
      });
      appendLog("info", "tauri connect_ws", r);
      authOut.textContent = `auth: Bearer set · ${r.detail}`;
    } catch (e) {
      appendLog("warn", "tauri connect_ws failed; falling back to browser WS", e);
    }
  } else if (sessionToken && !detectTauri()) {
    authOut.textContent =
      "auth: token held in memory — plain browser cannot set WS Authorization; use Tauri PC build or CLI for oidc/static Hub";
  } else {
    authOut.textContent = "auth: (none) · Hub dev mode OK";
  }

  try {
    const ack = await c.connect({ authorization: sessionToken });
    appendLog("info", "connected", {
      connection_id: ack.connection_id,
      user_id: ack.user_id,
      hub: ack.computer_hub_version,
    });
    capsEl.textContent =
      "capabilities: " + (ack.capabilities?.join(", ") || "(none)") +
      ` · user_id=${ack.user_id}`;
    connMetaEl.textContent = `connection_id: ${ack.connection_id} · user_id=${ack.user_id}`;
    const st = await c.status();
    statusOut.textContent = `runState: ${st.runState}`;
    const ro = await c.roster();
    mergeKnownFromRoster(ro.agents || []);
    renderRoster(ro.agents || []);
    // U1: auto listAgents + select first / keep selection
    try {
      await doListAgents();
    } catch (e) {
      appendLog("warn", "listAgents after connect failed (non-fatal)", e);
    }
    if (knownAgents.length && !agentEl.value) {
      agentEl.value = knownAgents[0].id;
    }
    void refreshTail();
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnLogin").onclick = async () => {
  clearFail();
  const issuer = (prompt("OIDC issuer (e.g. http://127.0.0.1:PORT)", "") || "").trim();
  if (!issuer) {
    appendLog("warn", "login cancelled — no issuer");
    return;
  }
  const clientId = (prompt("client_id", "atlas-bot-pc") || "atlas-bot-pc").trim();
  const audience = (prompt("audience (optional)", "atlas-hub") || "").trim() || undefined;
  try {
    const pkce = await generatePkce();
    const state = crypto.randomUUID();
    // Loopback redirect — for full flow use CLI or a local helper; here we
    // document manual callback paste for vite smoke without system browser bind.
    const redirectUri =
      prompt("redirect_uri", "http://127.0.0.1:8765/callback") ||
      "http://127.0.0.1:8765/callback";
    const authorizeUrl = buildAuthorizeUrl({
      authorizeUrl: issuer.replace(/\/$/, "") + "/authorize",
      clientId,
      redirectUri,
      state,
      pkce,
      audience,
    });
    appendLog("info", "authorize_url", authorizeUrl);
    window.open(authorizeUrl, "_blank", "noopener,noreferrer");
    const callback = prompt(
      "After IdP redirects, paste the full callback URL (…/callback?code=…&state=…)",
    );
    if (!callback) {
      appendLog("warn", "login cancelled — no callback");
      return;
    }
    const parsed = parseCallbackUrl(callback);
    if (parsed.error) throw new Error(parsed.error);
    if (!parsed.code) throw new Error("missing code");
    if (parsed.state && parsed.state !== state) throw new Error("state mismatch");
    const tr = await exchangeCode({
      tokenUrl: issuer.replace(/\/$/, "") + "/token",
      clientId,
      redirectUri,
      code: parsed.code,
      codeVerifier: pkce.verifier,
    });
    const bearer = pickHubBearer(tr);
    if (!bearer) throw new Error("no id_token/access_token");
    sessionToken = bearer;
    tokenPaste.value = bearer;
    authOut.textContent = "auth: logged in (id_token preferred) — Connect uses Tauri Bearer when available";
    appendLog("info", "login ok", { has_id_token: !!tr.id_token });
  } catch (e) {
    showFail({ message: String(e), code: "upstream_error" });
  }
};

$("btnLogout").onclick = () => {
  sessionToken = undefined;
  tokenPaste.value = "";
  client?.setAuthorization(undefined);
  client?.disconnect();
  client = null;
  authOut.textContent = "auth: logged out";
  capsEl.textContent = "capabilities: —";
  connMetaEl.textContent = "connection_id: —";
  appendLog("info", "logout");
};

$("btnDisconnect").onclick = () => {
  client?.disconnect();
  client = null;
  capsEl.textContent = "capabilities: —";
  connMetaEl.textContent = "connection_id: —";
};

$("btnStatus").onclick = async () => {
  clearFail();
  try {
    const st = await ensureClient().status();
    statusOut.textContent = `runState: ${st.runState}`;
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnRoster").onclick = async () => {
  clearFail();
  try {
    const ro = await ensureClient().roster();
    mergeKnownFromRoster(ro.agents || []);
    renderRoster(ro.agents || []);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnOffbox").onclick = async () => {
  clearFail();
  offboxNextCursor = undefined;
  await loadOffbox(undefined);
};

$("btnOffboxNext").onclick = async () => {
  clearFail();
  if (!offboxNextCursor) {
    appendLog("warn", "no nextCursor — already at end or load first page");
    return;
  }
  await loadOffbox(offboxNextCursor);
};

$("btnSub").onclick = async () => {
  clearFail();
  try {
    await ensureClient().subscribe([currentAgentId()]);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnUnsub").onclick = async () => {
  clearFail();
  try {
    await ensureClient().unsubscribe([currentAgentId()]);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnList").onclick = async () => {
  clearFail();
  try {
    await doListAgents();
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnCreate").onclick = async () => {
  clearFail();
  try {
    await doCreateAgent();
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnCreateMain").onclick = async () => {
  clearFail();
  try {
    await doCreateAgent();
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnSend").onclick = async () => {
  clearFail();
  try {
    const aid = currentAgentId();
    await ensureSubscribed(aid);
    const text = promptEl.value;
    const r = await ensureClient().sendPrompt(aid, text, {
      immediate: immediateEl.checked,
    });
    appendLog("info", "sendPrompt result", r);
    void refreshTail();
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnInterrupt").onclick = async () => {
  clearFail();
  try {
    const r = await ensureClient().interruptAgentRun(currentAgentId());
    appendLog("info", "interruptAgentRun result", r);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnTail").onclick = async () => {
  clearFail();
  await refreshTail();
};

$("btnVnc").onclick = async () => {
  clearFail();
  try {
    await doVnc();
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnVncIcon").onclick = async () => {
  clearFail();
  try {
    await doVnc();
  } catch (e) {
    showFail(e as DisplayError);
  }
};

function readFileAsBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = String(reader.result || "");
      const comma = dataUrl.indexOf(",");
      resolve(comma >= 0 ? dataUrl.slice(comma + 1) : dataUrl);
    };
    reader.onerror = () => reject(reader.error || new Error("read failed"));
    reader.readAsDataURL(file);
  });
}

$("btnUpload").onclick = async () => {
  clearFail();
  try {
    await doUpload(filePick);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnAttachIcon").onclick = () => {
  filePickMain.click();
};

filePickMain.onchange = async () => {
  clearFail();
  try {
    // mirror into debug file input for visibility
    if (filePickMain.files?.length) {
      const dt = new DataTransfer();
      dt.items.add(filePickMain.files[0]);
      filePick.files = dt.files;
    }
    await doUpload(filePickMain);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnAttach").onclick = async () => {
  clearFail();
  if (!lastUploadId) {
    showFail({
      message: "command_rejected",
      code: "command_rejected",
      reason: "attachment_not_found",
      retryable: false,
    });
    return;
  }
  try {
    const r = await ensureClient().attachUpload(currentAgentId(), lastUploadId);
    uploadOut.textContent = `attach path: ${r.path} · uploadId=${lastUploadId}`;
    appendLog("info", "attachUpload", r);
  } catch (e) {
    showFail(e as DisplayError);
  }
};
