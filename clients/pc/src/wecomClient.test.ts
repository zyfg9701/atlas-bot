import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  buildWeComAuthorizeUrl,
  ticketProviderFromEnv,
  wecomSubject,
  wsToHttpBase,
} from "./wecomClient";

describe("WeComProvider", () => {
  it("defaults provider to oidc", () => {
    assert.equal(ticketProviderFromEnv(undefined), "oidc");
    assert.equal(ticketProviderFromEnv("wecom"), "wecom");
  });

  it("builds authorize URL with state, no PKCE", () => {
    const url = buildWeComAuthorizeUrl({
      authorizeBase: "http://127.0.0.1:9",
      corpId: "ww_c",
      agentId: "1001",
      redirectUri: "http://127.0.0.1:1/callback",
      state: "st1",
    });
    const u = new URL(url);
    assert.equal(u.searchParams.get("response_type"), "code");
    assert.equal(u.searchParams.get("appid"), "ww_c");
    assert.equal(u.searchParams.get("agentid"), "1001");
    assert.equal(u.searchParams.get("state"), "st1");
    assert.equal(u.searchParams.get("code_challenge"), null);
  });

  it("nails sub mapping", () => {
    assert.equal(wecomSubject("ww_x", "alice"), "wecom:ww_x:alice");
  });

  it("derives hub http from ws", () => {
    assert.equal(wsToHttpBase("ws://127.0.0.1:7700/ws"), "http://127.0.0.1:7700");
  });
});
