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
  <h1>atlas-bot PC · P2 bot_client</h1>
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
      <h2>Cold path (status / roster — not bot.command)</h2>
      <div class="row">
        <button id="btnStatus">bot.status</button>
        <button id="btnRoster">bot.roster</button>
      </div>
      <div class="mono" id="statusOut">runState: —</div>
      <ul class="agents" id="roster"></ul>
    </div>
    <div class="panel">
      <h2>Subscribe + hot path</h2>
      <div class="row">
        <label>agent</label>
        <input id="agentId" value="agt_1" />
        <button id="btnSub">subscribe</button>
        <button id="btnUnsub" class="secondary">unsubscribe</button>
      </div>
      <div class="row">
        <button id="btnList">listAgents</button>
        <button id="btnTail">getAgentTranscriptTail</button>
      </div>
      <textarea id="prompt" placeholder="prompt text">hello P2</textarea>
      <div class="row" style="margin-top:8px">
        <button id="btnSend">sendPrompt</button>
      </div>
    </div>
  </div>
  <div class="grid" style="margin-top:12px">
    <div class="panel">
      <h2>Events (seq sort/display only)</h2>
      <div id="events" class="mono"></div>
    </div>
    <div class="panel">
      <h2>Transcript tail</h2>
      <div id="transcript" class="mono"></div>
    </div>
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
const agentEl = $<HTMLInputElement>("agentId");
const promptEl = $<HTMLTextAreaElement>("prompt");
const logEl = $("log");
const eventsEl = $("events");
const transcriptEl = $("transcript");
const failEl = $("fail");

const events: BotEventEnvelope[] = [];
let client: HubClient | null = null;

function showFail(err: DisplayError) {
  const bits = [
    err.message,
    err.code != null ? `code=${err.code}` : null,
    err.reason ? `reason=${err.reason}` : null,
    err.retryable === false ? "do-not-blind-retry" : null,
  ].filter(Boolean);
  failEl.textContent = `Failure: ${bits.join(" · ")}`;
  failEl.classList.add("show");
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

function renderEvents() {
  const sorted = sortEventsBySeq(events);
  eventsEl.textContent = sorted
    .map(
      (e) =>
        `#${e.seq} ${e.agentId} ${e.channel} ${JSON.stringify(e.event)}`,
    )
    .join("\n");
}

function renderRoster(agents: RosterEntry[]) {
  rosterEl.innerHTML = "";
  for (const a of agents) {
    const li = document.createElement("li");
    li.textContent = `${a.agentId} · ${a.name} · ${a.status}`;
    if (a.agentId === agentEl.value) li.classList.add("selected");
    li.onclick = () => {
      agentEl.value = a.agentId;
      renderRoster(agents);
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
        if (ev.channel === "hub:turn_finished") {
          appendLog("event", "turn finished — fetching transcript tail");
          void refreshTail();
        }
        if (ev.channel === "hub:resync_required") {
          appendLog("warn", "explicit resync_required — re-fetching transcript");
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
    const r = await c.getAgentTranscriptTail(agentEl.value, 20);
    transcriptEl.textContent = JSON.stringify(r, null, 2);
  } catch (e) {
    showFail(e as DisplayError);
  }
}

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
    // cold path immediately
    const st = await c.status();
    statusOut.textContent = `runState: ${st.runState}`;
    const ro = await c.roster();
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
    renderRoster(ro.agents || []);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnSub").onclick = async () => {
  clearFail();
  try {
    await ensureClient().subscribe([agentEl.value]);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnUnsub").onclick = async () => {
  clearFail();
  try {
    await ensureClient().unsubscribe([agentEl.value]);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnList").onclick = async () => {
  clearFail();
  try {
    const r = await ensureClient().listAgents(agentEl.value);
    appendLog("info", "listAgents result", r);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnSend").onclick = async () => {
  clearFail();
  try {
    const r = await ensureClient().sendPrompt(agentEl.value, promptEl.value);
    appendLog("info", "sendPrompt result", r);
  } catch (e) {
    showFail(e as DisplayError);
  }
};

$("btnTail").onclick = async () => {
  clearFail();
  await refreshTail();
};
