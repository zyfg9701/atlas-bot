/**
 * U4 Chat Hub connection prefs.
 *
 * Hub WebSocket + optional Bearer only. Gateway HTTP (`8787`, `atlas-pc-gw-http`)
 * stays on the debug advanced panel — never treat an http(s) approval base as a Hub URL.
 * No new `bot.*`. Bearer in localStorage is plaintext for a private self-host, not a keychain.
 */

import { DEFAULT_HUB_WS } from "./hubClient";

export const HUB_WS_STORAGE_KEY = "atlas-pc-hub-ws";
export const HUB_BEARER_STORAGE_KEY = "atlas-pc-hub-bearer";

export interface HubPrefStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
  removeItem(key: string): void;
}

export interface HubConnectionPrefs {
  url: string;
  bearer: string;
}

/** `ws:` / `wss:` with a host. Empty or anything else (including Gateway HTTP) → default. */
export function normalizeHubWs(raw: string | null | undefined): string {
  const trimmed = (raw ?? "").trim();
  if (!trimmed) return DEFAULT_HUB_WS;
  try {
    const u = new URL(trimmed);
    if ((u.protocol !== "ws:" && u.protocol !== "wss:") || !u.hostname) return DEFAULT_HUB_WS;
    return trimmed;
  } catch {
    return DEFAULT_HUB_WS;
  }
}

/** Optional. Empty stays empty — Connect may run without a Bearer in dev. */
export function normalizeHubBearer(raw: string | null | undefined): string {
  return (raw ?? "").trim();
}

/**
 * Read prefs. A stored URL that does not normalize to itself is rewritten to the
 * fallback so a stale `http://…:8787` cannot come back as the Hub address.
 * A whitespace-only Bearer is removed.
 */
export function loadHubConnection(store: HubPrefStore): HubConnectionPrefs {
  const rawUrl = store.getItem(HUB_WS_STORAGE_KEY);
  const url = normalizeHubWs(rawUrl);
  if (rawUrl !== null && rawUrl !== url) store.setItem(HUB_WS_STORAGE_KEY, url);

  const rawBearer = store.getItem(HUB_BEARER_STORAGE_KEY);
  const bearer = normalizeHubBearer(rawBearer);
  if (rawBearer !== null && rawBearer !== bearer) {
    if (bearer) store.setItem(HUB_BEARER_STORAGE_KEY, bearer);
    else store.removeItem(HUB_BEARER_STORAGE_KEY);
  }
  return { url, bearer };
}

/** Persist the normalized Hub URL (invalid input is stored as the default). */
export function saveHubWs(store: HubPrefStore, raw: string | null | undefined): string {
  const url = normalizeHubWs(raw);
  store.setItem(HUB_WS_STORAGE_KEY, url);
  return url;
}

/** Persist a trimmed Bearer, or remove the key when empty. */
export function saveHubBearer(store: HubPrefStore, raw: string | null | undefined): string {
  const bearer = normalizeHubBearer(raw);
  if (bearer) store.setItem(HUB_BEARER_STORAGE_KEY, bearer);
  else store.removeItem(HUB_BEARER_STORAGE_KEY);
  return bearer;
}
