import { describe, it } from "node:test";
import assert from "node:assert/strict";
import {
  buildAuthorizeUrl,
  looksLikeJwt,
  parseCallbackUrl,
  pickHubBearer,
  s256Challenge,
} from "./authClient";

describe("PKCE S256", () => {
  it("matches RFC 7636 appendix B challenge", async () => {
    const verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const challenge = await s256Challenge(verifier);
    assert.equal(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
  });
});

describe("authorize + callback", () => {
  it("builds authorize URL with PKCE params", () => {
    const url = buildAuthorizeUrl({
      authorizeUrl: "http://idp.test/authorize",
      clientId: "pc",
      redirectUri: "http://127.0.0.1:9/callback",
      state: "st1",
      pkce: {
        verifier: "v",
        challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        method: "S256",
      },
    });
    const u = new URL(url);
    assert.equal(u.searchParams.get("response_type"), "code");
    assert.equal(u.searchParams.get("code_challenge_method"), "S256");
    assert.equal(u.searchParams.get("state"), "st1");
  });

  it("parses callback code/state", () => {
    const r = parseCallbackUrl("http://127.0.0.1:1/callback?code=abc&state=st");
    assert.equal(r.code, "abc");
    assert.equal(r.state, "st");
  });
});

describe("pickHubBearer", () => {
  it("prefers id_token", () => {
    assert.equal(
      pickHubBearer({ access_token: "a.b.c", id_token: "x.y.z" }),
      "x.y.z",
    );
  });
  it("falls back to JWT access", () => {
    assert.ok(looksLikeJwt("a.b.c"));
    assert.equal(pickHubBearer({ access_token: "a.b.c" }), "a.b.c");
  });
});
