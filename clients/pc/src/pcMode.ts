/**
 * U2 PC shell routes. Pure functions — no DOM and no HubClient.
 *
 * Priority (deep link wins over memory):
 * 1. `?debug=1` on the load that still honors it → `#/debug` (accordion forced open).
 * 2. Explicit `#/chat` or `#/debug` → that page, and overwrite `atlas-pc-mode`.
 * 3. Empty hash / `#/` / unknown hash + valid `atlas-pc-mode` → restore that page.
 * 4. Otherwise the gate (`#/`).
 *
 * `atlas-pc-debug` only remembers the debug-page accordion (`0` closed,
 * missing or `1` open). It does not pick the route.
 */

export const PC_MODE_STORAGE_KEY = "atlas-pc-mode";

export type PcMode = "chat" | "debug";
export type PcRoute = "gate" | PcMode;

export type RouteInput = {
  hash: string;
  storedMode: string | null;
  /** True only while the caller still honors a one-shot `?debug=1`. */
  debugQuery: boolean;
  /** Value of localStorage `atlas-pc-debug`, if any. */
  debugPanelFlag: string | null;
};

export type RouteDecision = {
  route: PcRoute;
  /** Hash path to assign (`/chat`), or null to leave `location.hash` alone. */
  navigateHash: string | null;
  /** Write `atlas-pc-mode`, or null to leave storage unchanged. */
  persistMode: PcMode | null;
  /** When true, set the debug `<details>` open state. Never do this on Chat/gate. */
  syncAccordion: boolean;
  debugAccordionOpen: boolean;
  /** Caller should drop `?debug=1` and stop honoring it for later hash changes. */
  consumedDebugQuery: boolean;
};

export function normalizeHashPath(hash: string): string {
  let raw = (hash ?? "").trim();
  if (raw.startsWith("#")) raw = raw.slice(1);
  const q = raw.indexOf("?");
  if (q >= 0) raw = raw.slice(0, q);
  raw = raw.trim();
  if (raw === "" || raw === "/") return "/";
  if (!raw.startsWith("/")) raw = `/${raw}`;
  raw = raw.replace(/\/+$/, "");
  if (raw === "") return "/";
  return raw.toLowerCase();
}

export function parseStoredMode(raw: string | null | undefined): PcMode | null {
  if (raw === "chat" || raw === "debug") return raw;
  return null;
}

function accordionOpen(flag: string | null, force: boolean): boolean {
  if (force) return true;
  return flag !== "0";
}

export function resolvePcRoute(input: RouteInput): RouteDecision {
  const path = normalizeHashPath(input.hash);
  const explicit: PcMode | null = path === "/chat" ? "chat" : path === "/debug" ? "debug" : null;
  const stored = parseStoredMode(input.storedMode);
  const gateLike = path === "/";

  if (input.debugQuery) {
    return {
      route: "debug",
      navigateHash: path === "/debug" ? null : "/debug",
      persistMode: "debug",
      syncAccordion: true,
      debugAccordionOpen: true,
      consumedDebugQuery: true,
    };
  }

  if (explicit) {
    return {
      route: explicit,
      navigateHash: null,
      persistMode: explicit,
      syncAccordion: explicit === "debug",
      debugAccordionOpen: explicit === "debug" ? accordionOpen(input.debugPanelFlag, false) : false,
      consumedDebugQuery: false,
    };
  }

  if (stored) {
    return {
      route: stored,
      navigateHash: `/${stored}`,
      persistMode: stored,
      syncAccordion: stored === "debug",
      debugAccordionOpen: stored === "debug" ? accordionOpen(input.debugPanelFlag, false) : false,
      consumedDebugQuery: false,
    };
  }

  return {
    route: "gate",
    navigateHash: gateLike ? null : "/",
    persistMode: null,
    syncAccordion: false,
    debugAccordionOpen: false,
    consumedDebugQuery: false,
  };
}

/** Gate button: persist mode, then navigate. */
export function gateChoice(mode: PcMode): { persistMode: PcMode; hash: string } {
  return { persistMode: mode, hash: `#/${mode}` };
}
