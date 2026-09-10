import XCTest
@testable import AtlasBot

/// URLProtocol stub for Hub exchange (no live network / no qyapi).
final class WeComMockURLProtocol: URLProtocol {
    static var requestHandler: ((URLRequest) throws -> (HTTPURLResponse, Data))?

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        guard let handler = WeComMockURLProtocol.requestHandler else {
            client?.urlProtocol(self, didFailWithError: NSError(domain: "mock", code: 1))
            return
        }
        do {
            let (resp, data) = try handler(request)
            client?.urlProtocol(self, didReceive: resp, cacheStoragePolicy: .notAllowed)
            client?.urlProtocol(self, didLoad: data)
            client?.urlProtocolDidFinishLoading(self)
        } catch {
            client?.urlProtocol(self, didFailWithError: error)
        }
    }

    override func stopLoading() {}
}

@MainActor
final class WeComAuthTests: XCTestCase {
    func testBuildAuthorizeURLHasCorpAgentStateNoPKCE() {
        let cfg = WeComAuth.Config(
            corpId: "ww_mock_corp",
            agentId: "1000001",
            authorizeBase: "http://127.0.0.1:8091",
            hubHttpBase: "http://127.0.0.1:7700"
        )
        let url = WeComAuth.buildAuthorizeURL(cfg: cfg, state: "st1")
        XCTAssertNotNil(url)
        let s = url!.absoluteString
        XCTAssertTrue(s.contains("appid=ww_mock_corp"))
        XCTAssertTrue(s.contains("agentid=1000001"))
        XCTAssertTrue(s.contains("response_type=code"))
        XCTAssertTrue(s.contains("scope=snsapi_base"))
        XCTAssertTrue(s.contains("state=st1"))
        XCTAssertTrue(s.contains("wechat_redirect"))
        XCTAssertFalse(s.contains("code_challenge"))
        XCTAssertFalse(s.contains("code_challenge_method"))
    }

    func testSubjectShape() {
        XCTAssertEqual(WeComAuth.subject(corpId: "ww_c", userid: "user1"), "wecom:ww_c:user1")
    }

    func testTicketProviderDefaultsOidc() {
        XCTAssertEqual(ticketProviderFromEnv(nil), "oidc")
        XCTAssertEqual(ticketProviderFromEnv(""), "oidc")
        XCTAssertEqual(ticketProviderFromEnv("wecom"), "wecom")
        XCTAssertEqual(ticketProviderFromEnv(" WeCom "), "wecom")
    }

    func testWsToHttpBase() {
        XCTAssertEqual(wsToHttpBase("ws://127.0.0.1:7700/ws"), "http://127.0.0.1:7700")
        XCTAssertEqual(wsToHttpBase("wss://hub.example/ws"), "https://hub.example")
    }

    func testParseExchangeJSONRequiresAccessToken() throws {
        let data = #"{"access_token":"hub.jwt.here","subject":"wecom:c:u"}"#.data(using: .utf8)!
        let ok = try WeComAuth.parseExchangeJSON(data)
        XCTAssertEqual(ok.accessToken, "hub.jwt.here")
        XCTAssertEqual(ok.subject, "wecom:c:u")
        let bad = #"{"token_type":"Bearer"}"#.data(using: .utf8)!
        XCTAssertThrowsError(try WeComAuth.parseExchangeJSON(bad))
    }

    func testExchangeCodePostsJSONAndParsesAccessToken() {
        let exp = expectation(description: "exchange")
        WeComMockURLProtocol.requestHandler = { req in
            XCTAssertEqual(req.httpMethod, "POST")
            XCTAssertTrue(req.url?.path.hasSuffix("/auth/wecom/exchange") == true)
            XCTAssertEqual(req.value(forHTTPHeaderField: "Content-Type"), "application/json")
            let body = String(data: req.httpBody ?? Data(), encoding: .utf8) ?? ""
            XCTAssertTrue(body.contains("code-1"))
            XCTAssertTrue(body.contains("st-1"))
            XCTAssertFalse(body.contains("code_verifier"))
            XCTAssertFalse(body.contains("ATLAS_WECOM_SECRET"))
            let data = #"{"access_token":"a.b.c","token_type":"Bearer","subject":"wecom:ww_mock_corp:alice"}"#.data(using: .utf8)!
            let resp = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
            return (resp, data)
        }
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [WeComMockURLProtocol.self]
        let session = URLSession(configuration: config)
        WeComAuth.exchangeCode(
            hubHttpBase: "http://hub.test",
            code: "code-1",
            state: "st-1",
            session: session
        ) { result in
            switch result {
            case .success(let tr):
                XCTAssertEqual(tr.accessToken, "a.b.c")
                XCTAssertEqual(tr.subject, "wecom:ww_mock_corp:alice")
            case .failure(let e):
                XCTFail("\(e)")
            }
            exp.fulfill()
        }
        wait(for: [exp], timeout: 5)
        WeComMockURLProtocol.requestHandler = nil
    }

    func testAuthSessionStateMismatchFailsObservably() {
        let store = MemoryTokenStore()
        let session = AuthSession(tokenStore: store)
        let cfg = WeComAuth.Config(
            corpId: "ww",
            agentId: "1",
            authorizeBase: "http://127.0.0.1:9",
            hubHttpBase: "http://127.0.0.1:9"
        )
        _ = session.prepareWeComLogin(cfg: cfg, state: "expected-state")
        let exp = expectation(description: "mismatch")
        let url = URL(string: "atlasbot://auth/callback?code=x&state=wrong-state")!
        session.handleCallbackURL(url) { result in
            switch result {
            case .failure(let e):
                XCTAssertTrue(e.localizedDescription.contains("state mismatch"))
            case .success:
                XCTFail("expected failure")
            }
            exp.fulfill()
        }
        wait(for: [exp], timeout: 2)
        XCTAssertNil(store.loadBearer())
    }

    func testAuthSessionWeComExchangeSavesBearerAndProvider() {
        WeComMockURLProtocol.requestHandler = { req in
            let data = #"{"access_token":"hub.tok.en","subject":"wecom:ww:u"}"#.data(using: .utf8)!
            let resp = HTTPURLResponse(url: req.url!, statusCode: 200, httpVersion: nil, headerFields: nil)!
            return (resp, data)
        }
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [WeComMockURLProtocol.self]
        let urlSession = URLSession(configuration: config)
        let store = MemoryTokenStore()
        let session = AuthSession(tokenStore: store, urlSession: urlSession)
        let cfg = WeComAuth.Config(
            corpId: "ww",
            agentId: "1",
            authorizeBase: "http://127.0.0.1:9",
            hubHttpBase: "http://hub.test"
        )
        _ = session.prepareWeComLogin(cfg: cfg, state: "st")
        let exp = expectation(description: "exchange")
        let url = URL(string: "atlasbot://auth/callback?code=c1&state=st")!
        session.handleCallbackURL(url) { result in
            switch result {
            case .success(let tok):
                XCTAssertEqual(tok, "hub.tok.en")
            case .failure(let e):
                XCTFail("\(e)")
            }
            exp.fulfill()
        }
        wait(for: [exp], timeout: 5)
        XCTAssertEqual(store.loadBearer(), "hub.tok.en")
        XCTAssertEqual(store.loadProvider(), WeComAuth.provider)
        WeComMockURLProtocol.requestHandler = nil
    }

    func testLogoutClearsProvider() {
        let store = MemoryTokenStore()
        store.saveBearer("t", provider: WeComAuth.provider)
        XCTAssertEqual(store.loadProvider(), WeComAuth.provider)
        let session = AuthSession(tokenStore: store)
        session.logout()
        XCTAssertNil(store.loadBearer())
        XCTAssertNil(store.loadProvider())
    }

    func testHubClientBearerHeaderStillWorks() {
        let client = HubClient(url: "ws://127.0.0.1:9/ws")
        client.setAuthorization("hub.tok.en")
        XCTAssertEqual(client.buildConnectRequest()?.value(forHTTPHeaderField: "Authorization"), "Bearer hub.tok.en")
    }
}
