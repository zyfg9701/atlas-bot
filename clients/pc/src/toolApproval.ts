/**
 * TA1 Chat tool-approval helpers.
 *
 * Protocol is unchanged GA1/CG1: `hub:tool` summary prefix
 * `[approval_pending approvalId=… tool=…]`, then `POST /approve`.
 * No new `bot.*`.
 */

export const DEFAULT_GW_HTTP = "http://127.0.0.1:8787";
export const GW_HTTP_STORAGE_KEY = "atlas-pc-gw-http";
export const GW_APPROVE_TOKEN_STORAGE_KEY = "atlas-pc-gw-approve-token";

const PENDING_RE = /^\[approval_pending\s+approvalId=([^\s\]]+)\s+tool=([^\]]+)\]/;

export interface ParsedApprovalPending {
  approvalId: string;
  tool: string;
}

export interface PendingApproval {
  approvalId: string;
  tool: string;
  summary: string;
  agentId: string;
}

export type ApprovalDecision = "allow" | "deny";

export function decisionLabel(decision: ApprovalDecision): "允许" | "拒绝" {
  return decision === "allow" ? "允许" : "拒绝";
}

/** GA1b / CG1 pending hint. Post-exec `hub:tool` summaries return null. */
export function parseApprovalPending(summary: string): ParsedApprovalPending | null {
  const m = summary.trim().match(PENDING_RE);
  if (!m) return null;
  const approvalId = m[1].trim();
  const tool = m[2].trim();
  if (!approvalId || !tool) return null;
  return { approvalId, tool };
}

function summaryFromEvent(event: unknown): string {
  if (!event || typeof event !== "object" || !("summary" in event)) return "";
  const summary = (event as { summary?: unknown }).summary;
  if (summary == null) return "";
  return String(summary);
}

/** Pull a pending record from a `bot.event` payload. Other channels return null. */
export function pendingFromHubTool(input: {
  channel: string;
  agentId: string;
  event: unknown;
}): PendingApproval | null {
  if (input.channel !== "hub:tool") return null;
  const summary = summaryFromEvent(input.event);
  const parsed = parseApprovalPending(summary);
  if (!parsed) return null;
  return {
    approvalId: parsed.approvalId,
    tool: parsed.tool,
    summary: summary.trim(),
    agentId: input.agentId,
  };
}

/**
 * Insert or refresh one pending. Same id updates in place and does not
 * evict any other id (Map insertion order is kept).
 */
export function rememberPending(queue: Map<string, PendingApproval>, item: PendingApproval): void {
  const prev = queue.get(item.approvalId);
  if (prev) {
    queue.set(item.approvalId, { ...prev, ...item });
    return;
  }
  queue.set(item.approvalId, item);
}

/** Store a hub:tool pending if the summary matches. Returns the stored row. */
export function ingestHubToolPending(
  queue: Map<string, PendingApproval>,
  input: { channel: string; agentId: string; event: unknown },
): PendingApproval | null {
  const item = pendingFromHubTool(input);
  if (!item) return null;
  rememberPending(queue, item);
  return item;
}

export function forgetPending(queue: Map<string, PendingApproval>, approvalId: string): boolean {
  return queue.delete(approvalId);
}

export interface ApprovalQueueView {
  /** Insertion order, current agent only. */
  mine: PendingApproval[];
  otherCount: number;
}

export function approvalQueueView(
  queue: ReadonlyMap<string, PendingApproval>,
  agentId: string,
): ApprovalQueueView {
  const mine: PendingApproval[] = [];
  let otherCount = 0;
  for (const item of queue.values()) {
    if (item.agentId === agentId) mine.push(item);
    else otherCount += 1;
  }
  return { mine, otherCount };
}

/** Chat shows the card whenever any pending id is still held. */
export function approvalCardVisible(view: ApprovalQueueView): boolean {
  return view.mine.length > 0 || view.otherCount > 0;
}

/** Loopback default when Advanced was never opened and storage is empty. */
export function resolveGatewayHttpBase(raw: string | null | undefined): string {
  const trimmed = (raw ?? "").trim().replace(/\/+$/, "");
  return trimmed || DEFAULT_GW_HTTP;
}

export function approvalFetchInit(
  approvalId: string,
  decision: ApprovalDecision,
  token: string | null | undefined,
): { headers: Record<string, string>; body: string } {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  const tok = (token ?? "").trim();
  if (tok) {
    headers.Authorization = `Bearer ${tok}`;
    headers["X-Atlas-Approval-Token"] = tok;
  }
  return {
    headers,
    body: JSON.stringify({ approvalId, decision }),
  };
}

/** 2xx or closed-reject 409 — drop the id. Other statuses stay in the queue. */
export function approvalSettled(status: number): boolean {
  return (status >= 200 && status < 300) || status === 409;
}

export function formatApprovalFailure(input: {
  decision: ApprovalDecision;
  status?: number;
  body?: string;
  networkError?: string;
}): string {
  const label = decisionLabel(input.decision);
  if (input.networkError) {
    return `审批失败（${label}）：无法连接 Gateway。${input.networkError}`;
  }
  const status = input.status ?? 0;
  const snippet = (input.body ?? "").trim().replace(/\s+/g, " ").slice(0, 180);
  return snippet
    ? `审批失败（${label}）：HTTP ${status} · ${snippet}`
    : `审批失败（${label}）：HTTP ${status}`;
}
