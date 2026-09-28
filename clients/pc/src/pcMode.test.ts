import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  PC_MODE_STORAGE_KEY,
  gateChoice,
  normalizeHashPath,
  parseStoredMode,
  resolvePcRoute,
} from "./pcMode";

describe("normalizeHashPath", () => {
  it("treats empty, hash-only, and #/ as the gate path", () => {
    assert.equal(normalizeHashPath(""), "/");
    assert.equal(normalizeHashPath("#"), "/");
    assert.equal(normalizeHashPath("#/"), "/");
    assert.equal(normalizeHashPath("#/?ignored=1"), "/");
  });

  it("accepts chat and debug with optional slash and case", () => {
    assert.equal(normalizeHashPath("#/chat"), "/chat");
    assert.equal(normalizeHashPath("#/chat/"), "/chat");
    assert.equal(normalizeHashPath("#/DEBUG"), "/debug");
    assert.equal(normalizeHashPath("#/debug?x=1"), "/debug");
  });
});

describe("parseStoredMode", () => {
  it("accepts only chat and debug", () => {
    assert.equal(parseStoredMode("chat"), "chat");
    assert.equal(parseStoredMode("debug"), "debug");
    assert.equal(parseStoredMode("Chat"), null);
    assert.equal(parseStoredMode(""), null);
    assert.equal(parseStoredMode(null), null);
    assert.equal(parseStoredMode("gate"), null);
  });
});

describe("resolvePcRoute", () => {
  it("cold start with no hash and no memory shows the gate", () => {
    const d = resolvePcRoute({
      hash: "",
      storedMode: null,
      debugQuery: false,
      debugPanelFlag: null,
    });
    assert.equal(d.route, "gate");
    assert.equal(d.navigateHash, null);
    assert.equal(d.persistMode, null);
    assert.equal(d.syncAccordion, false);
  });

  it("invalid memory on #/ still shows the gate", () => {
    const d = resolvePcRoute({
      hash: "#/",
      storedMode: "nope",
      debugQuery: false,
      debugPanelFlag: null,
    });
    assert.equal(d.route, "gate");
    assert.equal(d.persistMode, null);
  });

  it("restores atlas-pc-mode when the hash is empty", () => {
    const d = resolvePcRoute({
      hash: "",
      storedMode: "chat",
      debugQuery: false,
      debugPanelFlag: "1",
    });
    assert.equal(d.route, "chat");
    assert.equal(d.navigateHash, "/chat");
    assert.equal(d.persistMode, "chat");
    assert.equal(d.syncAccordion, false);
  });

  it("deep link overrides stored mode", () => {
    const d = resolvePcRoute({
      hash: "#/debug",
      storedMode: "chat",
      debugQuery: false,
      debugPanelFlag: null,
    });
    assert.equal(d.route, "debug");
    assert.equal(d.navigateHash, null);
    assert.equal(d.persistMode, "debug");
    assert.equal(d.syncAccordion, true);
    assert.equal(d.debugAccordionOpen, true);
  });

  it("keeps a closed accordion when atlas-pc-debug is 0", () => {
    const d = resolvePcRoute({
      hash: "#/debug",
      storedMode: null,
      debugQuery: false,
      debugPanelFlag: "0",
    });
    assert.equal(d.route, "debug");
    assert.equal(d.debugAccordionOpen, false);
  });

  it("does not let atlas-pc-debug pick the route", () => {
    const d = resolvePcRoute({
      hash: "#/chat",
      storedMode: "debug",
      debugQuery: false,
      debugPanelFlag: "1",
    });
    assert.equal(d.route, "chat");
    assert.equal(d.syncAccordion, false);
    assert.equal(d.persistMode, "chat");
  });

  it("?debug=1 forces #/debug and an open accordion once", () => {
    const fromChat = resolvePcRoute({
      hash: "#/chat",
      storedMode: "chat",
      debugQuery: true,
      debugPanelFlag: "0",
    });
    assert.equal(fromChat.route, "debug");
    assert.equal(fromChat.navigateHash, "/debug");
    assert.equal(fromChat.persistMode, "debug");
    assert.equal(fromChat.debugAccordionOpen, true);
    assert.equal(fromChat.consumedDebugQuery, true);

    const already = resolvePcRoute({
      hash: "#/debug",
      storedMode: "chat",
      debugQuery: true,
      debugPanelFlag: "0",
    });
    assert.equal(already.navigateHash, null);
    assert.equal(already.debugAccordionOpen, true);
  });

  it("unknown hash without memory normalizes to the gate", () => {
    const d = resolvePcRoute({
      hash: "#/nope",
      storedMode: null,
      debugQuery: false,
      debugPanelFlag: null,
    });
    assert.equal(d.route, "gate");
    assert.equal(d.navigateHash, "/");
  });

  it("unknown hash with memory restores that mode", () => {
    const d = resolvePcRoute({
      hash: "#/nope",
      storedMode: "debug",
      debugQuery: false,
      debugPanelFlag: null,
    });
    assert.equal(d.route, "debug");
    assert.equal(d.navigateHash, "/debug");
  });
});

describe("gateChoice", () => {
  it("writes atlas-pc-mode then navigates", () => {
    assert.equal(PC_MODE_STORAGE_KEY, "atlas-pc-mode");
    assert.deepEqual(gateChoice("chat"), { persistMode: "chat", hash: "#/chat" });
    assert.deepEqual(gateChoice("debug"), { persistMode: "debug", hash: "#/debug" });
  });
});
