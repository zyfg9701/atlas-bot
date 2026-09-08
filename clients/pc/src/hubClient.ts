/**
 * Bot-Relay hub client (P3).
 *
 * Cold path: bot.status / bot.roster / bot.transcript.offbox — never via bot.command.
 * Hot path: bot.command (… / uploadAttachment / attachUpload) and bot.vncDescriptor (not a command).
 * Seq: display/sort only — never treat gaps as resync; only hub:resync_required.
 */

export const PROTOCOL_VERSION = "1.0.0";
export const DEFAULT_HUB_WS = "ws://127.0.0.1:7700/ws";

export type ConnState = "disconnected" | "connecting" | "hello" | "ready" | "error";

export interface HelloAck {
  connection_id: string;
  user_id: string;
  computer_hub_version: string;
  supported_protocol_versions: string[];
  capabilities?: string[];
}

export interface RosterEntry {
  agentId: string;
  name: string;
  status: string;
  lastTurnAt?: number | null;
}

export interface OffboxPage {
  entries: unknown;
  nextCursor?: string | null;
}

export interface BotEventEnvelope {
  v: number;
  agentId: string;
  seq: number;
  channel: string;
  event: unknown;
}

export type LogLevel = "info" | "warn" | "error" | "event";

export interface HubClientHandlers {
  onState?: (s: ConnState, detail?: string) => void;
  onLog?: (level: LogLevel, msg: string, data?: unknown) => void;
  onHelloAck?: (ack: HelloAck) => void;
  onEvent?: (ev: BotEventEnvelope) => void;
  onRpcError?: (err: DisplayError) => void;
}

export interface DisplayError {
  message: string;
  code?: string | number;
  reason?: string;
  retryable?: boolean;
  raw?: unknown;
}

interface Pending {
  resolve: (v: unknown) => void;
  reject: (e: DisplayError) => void;
}

/** Known Bot-Relay error codes; anything else → treat as failure (upstream_error). */
const KNOWN_CODES = new Set([
  "command_rejected",
  "identity_unavailable",
  "link_state_unavailable",
  "upstream_error",
  "forbidden",
  "unauthorized",
  "link_required",
]);

/** Closed-set command_rejected reasons PC must surface (P5 attachments + prior). */
export const CLOSED_SET_REASONS = new Set([
  "args_too_large",
  "attachment_too_large",
  "attachment_not_found",
  "attachment_not_ready",
  "attachment_wrong_source",
  "attachments_not_supported_in_live",
  "attachment_credential_unavailable",
  "agent_id_mismatch",
  "args_invalid",
  "gateway/unknown-method",
  "not_yet_enabled",
]);

export function formatClosedSetReason(reason?: string): string | undefined {
  if (!reason) return undefined;
  if (CLOSED_SET_REASONS.has(reason)) return reason;
  return reason; // still show unknown reason tokens; code may degrade separately
}

export function normalizeError(data: unknown): DisplayError {
  if (!data || typeof data !== "object") {
    return { message: "unknown failure", code: "upstream_error", raw: data };
  }
  const o = data as Record<string, unknown>;
  if ("message" in o || "code" in o) {
    const msg = String(o.message ?? "error");
    const wireCode = typeof o.message === "string" ? o.message : undefined;
    const payload = (o.data && typeof o.data === "object" ? o.data : o) as Record<
      string,
      unknown
    >;
    const codeStr = wireCode && KNOWN_CODES.has(wireCode) ? wireCode : undefined;
    const reason = typeof payload.reason === "string" ? payload.reason : undefined;
    const retryable = typeof payload.retryable === "boolean" ? payload.retryable : undefined;
    if (wireCode && !KNOWN_CODES.has(wireCode) && typeof o.code === "number" && o.code < 0) {
      return {
        message: msg,
        code: o.code as number,
        reason,
        retryable,
        raw: data,
      };
    }
    if (wireCode && !KNOWN_CODES.has(wireCode)) {
      return {
        message: `failure (${wireCode})`,
        code: "upstream_error",
        reason,
        retryable: false,
        raw: data,
      };
    }
    return {
      message: msg,
      code: codeStr ?? (o.code as string | number | undefined),
      reason,
      retryable,
      raw: data,
    };
  }
  return { message: "unknown failure", code: "upstream_error", raw: data };
}

/** Sort events by seq for display only — never infer resync from gaps. */
export function sortEventsBySeq(events: BotEventEnvelope[]): BotEventEnvelope[] {
  return [...events].sort((a, b) => {
    if (a.agentId !== b.agentId) return a.agentId.localeCompare(b.agentId);
    return a.seq - b.seq;
  });
}

export interface ConnectOptions {
  /** Bearer token (without "Bearer " prefix). Browser WS cannot set Authorization —
   * production PC uses Tauri `connect_ws`; this field is stored for Tauri path / paste. */
  authorization?: string;
}

export class HubClient {
  private ws: WebSocket | null = null;
  private nextId = 1;
  private pending = new Map<number | string, Pending>();
  private state: ConnState = "disconnected";
  private url: string;
  private handlers: HubClientHandlers;
  /** Per-agent last seen seq — for UI sort only, never gap→resync. */
  private lastSeq = new Map<string, number>();
  private subscribed = new Set<string>();
  /** Optional bearer for Tauri rust WS / paste UX (not sent by browser WebSocket). */
  private authorization?: string;

  constructor(url: string = DEFAULT_HUB_WS, handlers: HubClientHandlers = {}) {
    this.url = url;
    this.handlers = handlers;
  }

  setAuthorization(token?: string) {
    this.authorization = token || undefined;
  }

  getAuthorization(): string | undefined {
    return this.authorization;
  }

  get connectionState(): ConnState {
    return this.state;
  }

  get subscribedAgents(): string[] {
    return [...this.subscribed];
  }

  setUrl(url: string) {
    this.url = url;
  }

  private setState(s: ConnState, detail?: string) {
    this.state = s;
    this.handlers.onState?.(s, detail);
  }

  private log(level: LogLevel, msg: string, data?: unknown) {
    this.handlers.onLog?.(level, msg, data);
  }

  /**
   * Connect. Options.authorization is stored; browser WebSocket cannot set
   * Authorization headers — call Tauri `connect_ws` from UI when token is set.
   * Under Hub `dev`, connect without a token as before.
   */
  connect(opts?: ConnectOptions): Promise<HelloAck> {
    if (opts?.authorization !== undefined) {
      this.setAuthorization(opts.authorization);
    }
    this.disconnect();
    this.setState("connecting");
    return new Promise((resolve, reject) => {
      let settled = false;
      try {
        // Browser cannot attach Authorization. If a bearer is required, the
        // Tauri path must be used (see src-tauri connect_ws). For vite/dev
        // without Tauri, we still open a plain WS (Hub auth mode=dev).
        if (this.authorization) {
          this.log(
            "warn",
            "bearer set but browser WebSocket cannot send Authorization — use Tauri connect_ws for oidc/static",
          );
        }
        this.ws = new WebSocket(this.url);
      } catch (e) {
        this.setState("error", String(e));
        reject(normalizeError({ message: String(e) }));
        return;
      }

      this.ws.onopen = () => {
        this.setState("hello");
        const hello = { protocol_version: PROTOCOL_VERSION, kind: "bot_client" };
        this.log("info", "→ hello", hello);
        this.ws!.send(JSON.stringify(hello));
      };

      this.ws.onmessage = (ev) => {
        let data: unknown;
        try {
          data = JSON.parse(String(ev.data));
        } catch {
          this.log("warn", "non-JSON frame", ev.data);
          return;
        }
        const obj = data as Record<string, unknown>;

        if (!obj.jsonrpc && obj.connection_id) {
          const ack = obj as unknown as HelloAck;
          this.setState("ready");
          this.log("info", "← hello_ack", ack);
          this.handlers.onHelloAck?.(ack);
          if (!settled) {
            settled = true;
            resolve(ack);
          }
          return;
        }

        if (obj.jsonrpc === "2.0" && "id" in obj) {
          const id = obj.id as number | string;
          const p = this.pending.get(id);
          if (!p) {
            this.log("warn", "orphan response", obj);
            return;
          }
          this.pending.delete(id);
          if (obj.error) {
            const err = normalizeError(obj.error);
            this.log("error", `RPC error id=${id}`, err);
            this.handlers.onRpcError?.(err);
            p.reject(err);
          } else {
            this.log("info", `← result id=${id}`, obj.result);
            p.resolve(obj.result);
          }
          return;
        }

        if (obj.jsonrpc === "2.0" && obj.method === "bot.event") {
          const params = obj.params as BotEventEnvelope;
          this.handleEvent(params);
          return;
        }

        this.log("warn", "ignored frame", obj);
      };

      this.ws.onerror = () => {
        this.setState("error", "websocket error");
        this.log("error", "websocket error");
        if (!settled) {
          settled = true;
          reject(normalizeError({ message: "websocket error" }));
        }
      };

      this.ws.onclose = () => {
        this.setState("disconnected");
        this.log("warn", "disconnected — reconnect will re-hello (seq not comparable)");
        for (const [, p] of this.pending) {
          p.reject(normalizeError({ message: "connection closed" }));
        }
        this.pending.clear();
        this.lastSeq.clear();
        this.subscribed.clear();
        this.ws = null;
        if (!settled) {
          settled = true;
          reject(normalizeError({ message: "connection closed before hello_ack" }));
        }
      };
    });
  }

  disconnect() {
    if (this.ws) {
      try {
        this.ws.close();
      } catch {
        /* ignore */
      }
      this.ws = null;
    }
    this.pending.clear();
    this.setState("disconnected");
  }

  private handleEvent(params: BotEventEnvelope) {
    const prev = this.lastSeq.get(params.agentId);
    if (prev !== undefined && params.seq !== prev + 1) {
      this.log(
        "warn",
        `seq discontinuity agent=${params.agentId} prev=${prev} got=${params.seq} (display only; no auto-resync)`,
      );
    }
    this.lastSeq.set(params.agentId, params.seq);

    if (params.channel === "hub:resync_required") {
      this.log("warn", "hub:resync_required — re-fetch transcript (explicit)", params);
    } else if (params.channel === "hub:turn_finished") {
      this.log("event", "hub:turn_finished", params);
    } else {
      this.log("event", `bot.event ${params.channel}`, params);
    }
    this.handlers.onEvent?.(params);
  }

  private rpc(method: string, params: unknown): Promise<unknown> {
    if (!this.ws || this.ws.readyState !== WebSocket.OPEN || this.state !== "ready") {
      return Promise.reject(normalizeError({ message: "not connected" }));
    }
    const id = this.nextId++;
    const frame = { jsonrpc: "2.0", id, method, params };
    this.log("info", `→ ${method} id=${id}`, params);
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws!.send(JSON.stringify(frame));
    });
  }

  /** Cold — must NOT use bot.command */
  status(): Promise<{ runState: string }> {
    return this.rpc("bot.status", {}) as Promise<{ runState: string }>;
  }

  /** Cold — must NOT use bot.command */
  roster(): Promise<{ agents: RosterEntry[] }> {
    return this.rpc("bot.roster", {}) as Promise<{ agents: RosterEntry[] }>;
  }

  /** Cold off-box transcript page — must NOT use bot.command / must not wake gateway */
  transcriptOffbox(agentId: string, cursor?: string | null): Promise<OffboxPage> {
    const params: Record<string, unknown> = { agentId };
    if (cursor) params.cursor = cursor;
    return this.rpc("bot.transcript.offbox", params) as Promise<OffboxPage>;
  }

  subscribe(agentIds: string[], fullFidelity = false): Promise<unknown> {
    return this.rpc("bot.subscribe", { agentIds, ...(fullFidelity ? { fullFidelity: true } : {}) }).then(
      (r) => {
        for (const a of agentIds) this.subscribed.add(a);
        for (const a of agentIds) this.lastSeq.delete(a);
        return r;
      },
    );
  }

  unsubscribe(agentIds: string[]): Promise<unknown> {
    return this.rpc("bot.unsubscribe", { agentIds }).then((r) => {
      for (const a of agentIds) this.subscribed.delete(a);
      return r;
    });
  }

  /** Hot path */
  command(agentId: string, name: string, args: Record<string, unknown> = {}): Promise<unknown> {
    return this.rpc("bot.command", { agentId, name, args });
  }

  listAgents(agentId: string): Promise<unknown> {
    return this.command(agentId, "listAgents", {});
  }

  /** Minimal createAgent — args.name required */
  createAgent(routingAgentId: string, name: string, description = ""): Promise<unknown> {
    const args: Record<string, unknown> = { name };
    if (description) args.description = description;
    return this.command(routingAgentId, "createAgent", args);
  }

  sendPrompt(agentId: string, prompt: string, opts?: { immediate?: boolean }): Promise<unknown> {
    const args: Record<string, unknown> = { agentId, prompt };
    if (opts?.immediate) args.immediate = true;
    return this.command(agentId, "sendPrompt", args);
  }

  getAgentTranscriptTail(agentId: string, limit = 20): Promise<unknown> {
    return this.command(agentId, "getAgentTranscriptTail", { id: agentId, limit });
  }

  /** interruptAgentRun — args include agentId (acceptance); envelope must match */
  interruptAgentRun(agentId: string): Promise<unknown> {
    return this.command(agentId, "interruptAgentRun", { agentId });
  }

  /** Hot Hub method — NOT a bot.command name. */
  vncDescriptor(agentId: string): Promise<{ vncUrl: string; expiresHint: number | null }> {
    return this.rpc("bot.vncDescriptor", { agentId }) as Promise<{
      vncUrl: string;
      expiresHint: number | null;
    }>;
  }

  /** Hot upload — UI should keep files well under 3 MiB args JSON. */
  uploadAttachment(
    agentId: string,
    filename: string,
    bytesBase64: string,
  ): Promise<{ path: string; uploadId?: string }> {
    return this.command(agentId, "uploadAttachment", {
      bytesBase64,
      filename,
      agentId,
    }) as Promise<{ path: string; uploadId?: string }>;
  }

  attachUpload(agentId: string, uploadId: string): Promise<{ path: string }> {
    return this.command(agentId, "attachUpload", { uploadId, agentId }) as Promise<{
      path: string;
    }>;
  }
}
