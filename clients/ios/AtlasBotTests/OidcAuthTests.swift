import XCTest
@testable import AtlasBot

final class OidcAuthTests: XCTestCase {
    func testPkceRfc7636AppendixB() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        let challenge = s256Challenge(verifier)
        XCTAssertEqual(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        XCTAssertFalse(challenge.contains("="))
        XCTAssertFalse(challenge.contains("+"))
        XCTAssertFalse(challenge.contains("/"))
    }

    func testGeneratePkceShape() {
        let p = generatePkce()
        XCTAssertEqual(p.method, "S256")
        XCTAssertTrue(p.verifier.count >= 43 && p.verifier.count <= 128)
        XCTAssertEqual(p.challenge, s256Challenge(p.verifier))
        XCTAssertEqual(p.challenge.count, 43)
    }

    func testParseCallbackCodeState() {
        let p = parseCallbackUrl("atlasbot://auth/callback?code=abc&state=xyz")
        XCTAssertEqual(p.code, "abc")
        XCTAssertEqual(p.state, "xyz")
        XCTAssertNil(p.error)
    }

    func testParseCallbackError() {
        let p = parseCallbackUrl("atlasbot://auth/callback?error=access_denied&state=s1")
        XCTAssertEqual(p.error, "access_denied")
        XCTAssertEqual(p.state, "s1")
        XCTAssertNil(p.code)
    }

    func testBuildAuthorizeUrlIncludesPkce() {
        let pkce = PkcePair(verifier: "v", challenge: "chal")
        let url = buildAuthorizeUrl(
            cfg: OidcClientConfig(issuer: "http://127.0.0.1:8090", clientId: "atlas-bot-ios"),
            redirectUri: MOBILE_REDIRECT_URI,
            state: "st",
            pkce: pkce
        )
        let s = url.absoluteString
        XCTAssertTrue(s.contains("response_type=code"))
        XCTAssertTrue(s.contains("client_id=atlas-bot-ios"))
        XCTAssertTrue(s.contains("code_challenge=chal"))
        XCTAssertTrue(s.contains("code_challenge_method=S256"))
        XCTAssertTrue(s.contains("openid"))
    }

    func testPickHubBearerPrefersIdToken() {
        var tr = TokenResponse()
        tr.accessToken = "a.b.c"
        tr.idToken = "id.tok.en"
        XCTAssertEqual(pickHubBearer(tr), "id.tok.en")
    }

    func testMemoryTokenStore() {
        let store = MemoryTokenStore()
        XCTAssertNil(store.loadBearer())
        store.saveBearer("tok")
        XCTAssertEqual(store.loadBearer(), "tok")
        store.clear()
        XCTAssertNil(store.loadBearer())
    }

    func testHubClientBearerHeader() {
        let client = HubClient(url: "ws://127.0.0.1:9/ws")
        XCTAssertNil(client.buildConnectRequest()?.value(forHTTPHeaderField: "Authorization"))
        client.setAuthorization("id.tok.en")
        XCTAssertEqual(client.buildConnectRequest()?.value(forHTTPHeaderField: "Authorization"), "Bearer id.tok.en")
        client.setAuthorization(nil)
        XCTAssertNil(client.buildConnectRequest()?.value(forHTTPHeaderField: "Authorization"))
    }

    func testRedirectLiteral() {
        XCTAssertEqual(MOBILE_REDIRECT_URI, "atlasbot://auth/callback")
        XCTAssertEqual(DEFAULT_IOS_CLIENT_ID, "atlas-bot-ios")
    }
}
