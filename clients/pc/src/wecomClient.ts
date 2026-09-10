/**
 * W1 PC WeComProvider helpers (same Login → code → Hub exchange → Bearer shape as OIDC).
 * No PKCE (WeCom web OAuth); state + one-time code; secret stays on Hub.
 */

export type TicketProvider = "oidc" | "wecom";

export function ticketProviderFromEnv(
  raw?: string | null,
): TicketProvider {
  const v = (raw ?? "").trim().toLowerCase();
  if (v === "wecom") return "wecom";
  return "oidc";
}

export interface WeComAuthorizeParams {
  authorizeBase: string;
  corpId: string;
  agentId: string;
  redirectUri: string;
  state: string;
}

/** Build mock/live WeCom authorize URL (query shape aligned with mock-wecom). */
export function buildWeComAuthorizeUrl(p: WeComAuthorizeParams): string {
  const base = p.authorizeBase.replace(/\/$/, "");
  const authorize = base.endsWith("/authorize") ? base : `${base}/authorize`;
  const u = new URL(authorize);
  u.searchParams.set("appid", p.corpId);
  u.searchParams.set("redirect_uri", p.redirectUri);
  u.searchParams.set("response_type", "code");
  u.searchParams.set("scope", "snsapi_base");
  u.searchParams.set("state", p.state);
  u.searchParams.set("agentid", p.agentId);
  u.hash = "wechat_redirect";
  return u.toString();
}

export function wecomSubject(corpId: string, userid: string): string {
  return `wecom:${corpId}:${userid}`;
}

export interface WeComExchangeResponse {
  access_token: string;
  token_type?: string;
  expires_in?: number;
  subject?: string;
}

/** POST Hub `/auth/wecom/exchange` — client never holds ATLAS_WECOM_SECRET. */
export async function exchangeWeComCode(opts: {
  hubHttpBase: string;
  code: string;
  state?: string;
}): Promise<WeComExchangeResponse> {
  const url = `${opts.hubHttpBase.replace(/\/$/, "")}/auth/wecom/exchange`;
  const body: Record<string, string> = { code: opts.code };
  if (opts.state) body.state = opts.state;
  const resp = await fetch(url, {
    method: "POST",
    headers: {
      accept: "application/json",
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
  if (!resp.ok) {
    throw new Error(`wecom exchange ${resp.status}: ${await resp.text()}`);
  }
  const tr = (await resp.json()) as WeComExchangeResponse;
  if (!tr.access_token) throw new Error("exchange missing access_token");
  return tr;
}

/** Derive Hub HTTP from WS URL. */
export function wsToHttpBase(ws: string): string {
  let s = ws.trim();
  if (s.startsWith("ws://")) s = "http://" + s.slice(5);
  else if (s.startsWith("wss://")) s = "https://" + s.slice(6);
  if (s.endsWith("/ws")) s = s.slice(0, -3);
  return s.replace(/\/$/, "");
}
