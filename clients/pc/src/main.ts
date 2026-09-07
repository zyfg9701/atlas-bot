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

const app = document.querySelector("#app")!;

app.innerHTML = `
  <h1>atlas-bot PC · P5 bot_client</h1>
  <div id="fail" class="fail-banner"></div>
  <div class="panel" style="margin-bottom:12px">
    <div class="row">
      <label>Hub WS</label>
      <input id="url" value="${DEFAULT_HUB_WS}" />
      <button id="btnConnect">Connect + hello</button>
      <button id="btnDisconnect" class="secondary">Disconnect</button>
      <span id="state" class="badge disconnected">disconnected</span>
    </div>
    <div class="mono" id="caps">capabilities: —</div>
  </div>
  <div class="grid">
    <div class="panel">
      <h2>Cold path (status / roster / transcript.offbox)</h2>
      <div class="row">
        <button id="btnStatus">bot.status</button>
        <button id="btnRoster">bot.roster</button>
        <button id="btnOffbox">offbox page</button>
        <button id="btnOffboxNext" class="secondary">offbox next</button>
      </div>
      <div class="mono" id="statusOut">runState: —</div>
      <ul class="agents" id="roster"></ul>
      <h2 style="margin-top:10px">Cold / offline transcript</h2>
      <div class="mono" id="offboxCursor">cursor: (first page)</div>
      <div id="offbox" class="mono"></div>
    </div>
    <div class="panel">
      <h2>Multi-agent + hot path</h2>
      <div class="row">
        <label>agent</label>
        <select id="agentId"></select>
        <button id="btnSub">subscribe</button>
        <button id="btnUnsub" class="secondary">unsubscribe</button>
      </div>
      <div class="row">
        <input id="newName" placeholder="new agent name" value="Scout" />
        <button id="btnCreate">createAgent</button>
        <button id="btnList">listAgents</button>
      </div>
      <div class="row">
        <button id="btnTail">getAgentTranscriptTail</button>
        <button id="btnInterrupt" class="secondary">interruptAgentRun</button>
      </div>
      <textarea id="prompt" placeholder="prompt text">hello P3</textarea>
      <div class="row" style="margin-top:8px">
        <button id="btnSend">sendPrompt</button>
        <label class="mono"><input type="checkbox" id="immediate" /> immediate (skip interrupt window)</label>
      </div>
      <div class="mono" id="listOut" style="margin-top:8px"></div>
    </div>
  </div>
  <div class="grid" style="margin-top:12px">
    <div class="panel">
      <h2>Events (seq sort/display only; per-agent)</h2>
      <div id="events" class="mono"></div>
    </div>
    <div class="panel">
      <h2>Hot transcript tail (current agent)</h2>
      <div id="transcript" class="mono"></div>
    </div>
  </div>
  <div class="panel" style="margin-top:12px">
    <h2>P5 · Desktop (bot.vncDescriptor) + attachments</h2>
    <div class="row">
      <button id="btnVnc">Open desktop</button>
      <span class="mono" id="vncOut">vnc: —</span>
    </div>
    <div class="row">
      <input id="filePick" type="file" />
      <button id="btnUpload">uploadAttachment</button>
      <button id="btnAttach" class="secondary">attachUpload (last id)</button>
    </div>
    <div class="mono" id="uploadOut">upload path: —</div>
    <p class="mono" style="color:var(--muted);margin:6px 0 0">
      UI caps file pick at 1.5 MiB (args JSON hard limit 3 MiB → reason args_too_large).
      Closed-set reasons show in the Failure banner.
    </p>
  </div>
  <div class="panel" style="margin-top:12px">
    <h2>Log</h2>
    <div id="log" class="mono"></div>
  </div>
`;

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

const urlEl = $<HTMLInputElement>("url");
const stateEl = $("state");
const capsEl = $("caps");
const statusOut = $("statusOut");
const rosterEl = $("roster");
const agentEl = $<HTMLSelectElement>("agentId");
const promptEl = $<HTMLTextAreaElement>("prompt");
const newNameEl = $<HTMLInputElement>("newName");
const immediateEl = $<HTMLInputElement>("immediate");
const logEl = $("log");
const eventsEl = $("events");
const transcriptEl = $("transcript");
const offboxEl = $("offbox");
const offboxCursorEl = $("offboxCursor");
const listOut = $("listOut");
const failEl = $("fail");
const vncOut = $("vncOut");
const uploadOut = $("uploadOut");
const filePick = $<HTMLInputElement>("filePick");

const events: BotEventEnvelope[] = [];
let client: HubClient | null = null;
let knownAgents: { id: string; name: string }[] = [{ id: "agt_1", name: "Watcher" }];
let offboxNextCursor: string | null | undefined = undefined;
let lastRoster: RosterEntry[] = [];
let lastUploadId: string | null = null;

/** PC-side soft cap — far below gateway 3 MiB args JSON limit. */
const UI_UPLOAD_MAX_BYTES = Math.floor(1.5 * 1024 * 1024);

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

function renderEvents() {
  const aid = currentAgentId();
  const sorted = sortEventsBySeq(events.filter((e) => e.agentId === aid));
  eventsEl.textContent = sorted
    .map((e) => `#${e.seq} ${e.agentId} ${e.channel} ${JSON.stringify(e.event)}`)
    .join("\n");
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
    transcriptEl.textContent = JSON.stringify(r, null, 2);
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

renderAgentSelect();

agentEl.onchange = () => {
  offboxNextCursor = undefined;
  offboxCursorEl.textContent = "cursor: (first page)";
  offboxEl.textContent = "";
  renderRoster(lastRoster);
  renderEvents();
  void refreshTail();
};

$("btnConnect").onclick = async () => {
  clearFail();
  events.length = 0;
  renderEvents();
  const c = ensureClient();
  c.setUrl(urlEl.value);
  try {
    const ack = await c.connect();
    appendLog("info", "connected", {
      connection_id: ack.connection_id,
      hub: ack.computer_hub_version,
    });
    const st = await c.status();
    statusOut.textContent = `runState: ${st.runState}`;
    const ro = await c.roster();
    mergeKnownFromRoster(ro.agents || []);
    renderRoster(ro.agents || []);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnDisconnect").onclick = () => {
  client?.disconnect();
  client = null;
  capsEl.textContent = "capabilities: —";
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
    const r = await ensureClient().listAgents(currentAgentId());
    listOut.textContent = JSON.stringify(r, null, 2);
    mergeKnownFromList(r);
    appendLog("info", "listAgents result", r);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnCreate").onclick = async () => {
  clearFail();
  try {
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
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnSend").onclick = async () => {
  clearFail();
  try {
    const r = await ensureClient().sendPrompt(currentAgentId(), promptEl.value, {
      immediate: immediateEl.checked,
    });
    appendLog("info", "sendPrompt result", r);
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
    const r = await ensureClient().vncDescriptor(currentAgentId());
    const exp =
      r.expiresHint == null
        ? "expiresHint=null"
        : `expiresHint=${r.expiresHint} (${new Date(r.expiresHint).toLocaleString()})`;
    vncOut.textContent = `vncUrl=${r.vncUrl} · ${exp}`;
    appendLog("info", "bot.vncDescriptor", r);
    window.open(r.vncUrl, "_blank", "noopener,noreferrer");
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
  const file = filePick.files?.[0];
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
  try {
    const b64 = await readFileAsBase64(file);
    const r = await ensureClient().uploadAttachment(currentAgentId(), file.name, b64);
    lastUploadId = r.uploadId || null;
    uploadOut.textContent = `upload path: ${r.path}` + (lastUploadId ? ` · uploadId=${lastUploadId}` : "");
    appendLog("info", "uploadAttachment", r);
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
