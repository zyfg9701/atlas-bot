import { describe, it } from "node:test";
import assert from "node:assert/strict";
import { DEFAULT_HUB_WS } from "./hubClient";
import {
  HUB_BEARER_STORAGE_KEY,
  HUB_WS_STORAGE_KEY,
  loadHubConnection,
  normalizeHubBearer,
  normalizeHubWs,
  saveHubBearer,
  saveHubWs,
  type HubPrefStore,
} from "./hubConnection";

const GW_HTTP = "http://127.0.0.1:8787";

function memoryStore(initial: Record<string, string> = {}): HubPrefStore & { snapshot(): Record<string, string> } {
  const data = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return data.has(key) ? data.get(key)! : null;
    },
    setItem(key, value) {
      data.set(key, value);
    },
    removeItem(key) {
      data.delete(key);
    },
    snapshot() {
      return Object.fromEntries(data);
    },
  };
}

describe("normalizeHubWs", () => {
  it("falls back to the default Hub socket when empty or invalid", () => {
    assert.equal(normalizeHubWs(null), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs(undefined), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs(""), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs("   "), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs("not a url"), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs("127.0.0.1:7700"), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs("javascript:alert(1)"), DEFAULT_HUB_WS);
  });

  it("does not accept Gateway HTTP (or any http/https) as a Hub URL", () => {
    assert.equal(normalizeHubWs(GW_HTTP), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs("http://127.0.0.1:8787/"), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs("https://127.0.0.1:8787/approve"), DEFAULT_HUB_WS);
    assert.equal(normalizeHubWs("http://127.0.0.1:7700/ws"), DEFAULT_HUB_WS);
  });

  it("keeps ws and wss URLs, including a websocket on port 8787", () => {
    assert.equal(normalizeHubWs("  ws://192.168.1.9:7700/ws  "), "ws://192.168.1.9:7700/ws");
    assert.equal(normalizeHubWs("wss://hub.example/ws"), "wss://hub.example/ws");
    assert.equal(normalizeHubWs("ws://[::1]:7700/ws"), "ws://[::1]:7700/ws");
    assert.equal(normalizeHubWs("ws://127.0.0.1:8787/ws"), "ws://127.0.0.1:8787/ws");
    assert.equal(normalizeHubWs(DEFAULT_HUB_WS), DEFAULT_HUB_WS);
  });
});

describe("normalizeHubBearer", () => {
  it("trims and treats missing as empty", () => {
    assert.equal(normalizeHubBearer(null), "");
    assert.equal(normalizeHubBearer(undefined), "");
    assert.equal(normalizeHubBearer("   "), "");
    assert.equal(normalizeHubBearer("  tok  "), "tok");
  });
});

describe("load / save", () => {
  it("round-trips a custom Hub URL and optional Bearer", () => {
    const store = memoryStore();
    assert.equal(saveHubWs(store, "wss://self-host/ws"), "wss://self-host/ws");
    assert.equal(saveHubBearer(store, " secret "), "secret");
    assert.deepEqual(store.snapshot(), {
      [HUB_WS_STORAGE_KEY]: "wss://self-host/ws",
      [HUB_BEARER_STORAGE_KEY]: "secret",
    });
    assert.deepEqual(loadHubConnection(store), {
      url: "wss://self-host/ws",
      bearer: "secret",
    });
  });

  it("loads the default URL and an empty Bearer when nothing is stored", () => {
    const store = memoryStore();
    assert.deepEqual(loadHubConnection(store), { url: DEFAULT_HUB_WS, bearer: "" });
    assert.deepEqual(store.snapshot(), {});
  });

  it("stores the default when the URL is cleared or is Gateway HTTP", () => {
    const store = memoryStore({ [HUB_WS_STORAGE_KEY]: "ws://custom/ws" });
    assert.equal(saveHubWs(store, "  "), DEFAULT_HUB_WS);
    assert.equal(store.snapshot()[HUB_WS_STORAGE_KEY], DEFAULT_HUB_WS);
    assert.equal(saveHubWs(store, GW_HTTP), DEFAULT_HUB_WS);
    assert.equal(loadHubConnection(store).url, DEFAULT_HUB_WS);
    assert.equal(store.snapshot()[HUB_WS_STORAGE_KEY], DEFAULT_HUB_WS);
  });

  it("rewrites an invalid stored URL on load and drops a blank Bearer", () => {
    const store = memoryStore({
      [HUB_WS_STORAGE_KEY]: GW_HTTP,
      [HUB_BEARER_STORAGE_KEY]: "   ",
    });
    assert.deepEqual(loadHubConnection(store), { url: DEFAULT_HUB_WS, bearer: "" });
    assert.deepEqual(store.snapshot(), { [HUB_WS_STORAGE_KEY]: DEFAULT_HUB_WS });
  });

  it("removes the Bearer key when saved empty", () => {
    const store = memoryStore({ [HUB_BEARER_STORAGE_KEY]: "tok" });
    assert.equal(saveHubBearer(store, ""), "");
    assert.equal(store.getItem(HUB_BEARER_STORAGE_KEY), null);
    assert.equal(loadHubConnection(store).bearer, "");
  });
});
