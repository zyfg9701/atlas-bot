/**
 * I2.1 PC OIDC PKCE helpers (browser/vite unit-testable).
 *
 * Production PC path: Tauri rust `connect_ws` sends Authorization on upgrade
 * (browser WebSocket API cannot set custom headers). Plain vite/browser: paste
 * token or use CLI; PKCE URL+callback parsing is tested here.
 */

export interface PkcePair {
  verifier: string;
  challenge: string;
  method: "S256";
}

const VERIFIER_ALPHABET =
  "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";

function base64Url(bytes: ArrayBuffer | Uint8Array): string {
  const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let s = "";
  for (let i = 0; i < u8.length; i++) s += String.fromCharCode(u8[i]!);
  const b64 = btoa(s);
  return b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

export function randomVerifier(len = 64): string {
  const n = Math.max(43, Math.min(128, len));
  const out: string[] = [];
  const rand = new Uint8Array(n);
  crypto.getRandomValues(rand);
  for (let i = 0; i < n; i++) {
    out.push(VERIFIER_ALPHABET[rand[i]! % VERIFIER_ALPHABET.length]!);
  }
  return out.join("");
}

export async function s256Challenge(verifier: string): Promise<string> {
  const data = new TextEncoder().encode(verifier);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return base64Url(digest);
}

export async function generatePkce(): Promise<PkcePair> {
  const verifier = randomVerifier(64);
  const challenge = await s256Challenge(verifier);
  return { verifier, challenge, method: "S256" };
}

export interface AuthorizeParams {
  authorizeUrl: string;
  clientId: string;
  redirectUri: string;
  scopes?: string[];
  state: string;
  pkce: PkcePair;
  audience?: string;
}

export function buildAuthorizeUrl(p: AuthorizeParams): string {
  const u = new URL(p.authorizeUrl);
  u.searchParams.set("response_type", "code");
  u.searchParams.set("client_id", p.clientId);
  u.searchParams.set("redirect_uri", p.redirectUri);
  u.searchParams.set("scope", (p.scopes ?? ["openid", "profile"]).join(" "));
  u.searchParams.set("state", p.state);
  u.searchParams.set("code_challenge", p.pkce.challenge);
  u.searchParams.set("code_challenge_method", p.pkce.method);
  if (p.audience) u.searchParams.set("audience", p.audience);
  return u.toString();
}

/** Parse `?code=&state=` (or error) from a callback URL. */
export function parseCallbackUrl(callbackUrl: string): {
  code?: string;
  state?: string;
  error?: string;
} {
  const u = new URL(callbackUrl);
  const err = u.searchParams.get("error") ?? undefined;
  if (err) return { error: err, state: u.searchParams.get("state") ?? undefined };
  return {
    code: u.searchParams.get("code") ?? undefined,
    state: u.searchParams.get("state") ?? undefined,
  };
}

export interface TokenResponse {
  access_token?: string;
  id_token?: string;
  refresh_token?: string;
  token_type?: string;
  expires_in?: number;
}

export function looksLikeJwt(token: string): boolean {
  const parts = token.split(".");
  return parts.length === 3 && parts.every((p) => p.length > 0);
}

/** Prefer id_token; else JWT access_token (Hub oidc JWKS path). */
export function pickHubBearer(tr: TokenResponse): string | undefined {
  if (tr.id_token) return tr.id_token;
  if (tr.access_token && looksLikeJwt(tr.access_token)) return tr.access_token;
  return tr.access_token || tr.id_token;
}

export async function exchangeCode(opts: {
  tokenUrl: string;
  clientId: string;
  redirectUri: string;
  code: string;
  codeVerifier: string;
}): Promise<TokenResponse> {
  const body = new URLSearchParams({
    grant_type: "authorization_code",
    code: opts.code,
    redirect_uri: opts.redirectUri,
    client_id: opts.clientId,
    code_verifier: opts.codeVerifier,
  });
  const resp = await fetch(opts.tokenUrl, {
    method: "POST",
    headers: {
      accept: "application/json",
      "content-type": "application/x-www-form-urlencoded",
    },
    body,
  });
  if (!resp.ok) {
    throw new Error(`token endpoint ${resp.status}: ${await resp.text()}`);
  }
  return (await resp.json()) as TokenResponse;
}

export function detectTauri(): boolean {
  return typeof window !== "undefined" && !!(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
}
