import Foundation

/// WM1 WeComProvider — ASWebAuthenticationSession → code → Hub exchange → TokenStore.
///
/// Contract (aligned with PC `exchangeWeComCode` / W1):
/// - Authorize URL: corpId/agentId; **no PKCE** (state + one-time code).
/// - Callback remains `atlasbot://auth/callback`.
/// - Exchange: Hub `POST /auth/wecom/exchange` (secret Hub-only).
/// - Store Hub JWT in Keychain via `TokenStore`; `HubClient` Bearer on upgrade.
/// - **No WebView** primary path (ASWebAuthenticationSession only).
/// - Provider switch: `ATLAS_TICKET_PROVIDER=wecom`; default oidc.
///
/// See docs/i2-login-runbook.md § WeCom / § Mobile.
enum WeComAuth {
    static let provider = "wecom"
    static let redirectURI = "atlasbot://auth/callback"

    struct Config {
        var corpId: String
        var agentId: String
        var authorizeBase: String
        var hubHttpBase: String
        var redirectUri: String = WeComAuth.redirectURI
    }

    struct ExchangeResponse {
        var accessToken: String
        var tokenType: String?
        var expiresIn: Int64?
        var subject: String?
    }

    static func buildAuthorizeURL(cfg: Config, state: String) -> URL? {
        var root = cfg.authorizeBase
        while root.hasSuffix("/") { root.removeLast() }
        let authorize = root.hasSuffix("/authorize") ? root : root + "/authorize"
        var c = URLComponents(string: authorize)
        c?.queryItems = [
            URLQueryItem(name: "appid", value: cfg.corpId),
            URLQueryItem(name: "redirect_uri", value: cfg.redirectUri),
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "scope", value: "snsapi_base"),
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "agentid", value: cfg.agentId),
        ]
        c?.fragment = "wechat_redirect"
        return c?.url
    }

    static func subject(corpId: String, userid: String) -> String {
        "wecom:\(corpId):\(userid)"
    }

    static func exchangeURL(hubHttpBase: String) -> URL? {
        var root = hubHttpBase
        while root.hasSuffix("/") { root.removeLast() }
        return URL(string: root + "/auth/wecom/exchange")
    }

    /// POST Hub `/auth/wecom/exchange` — JSON `{code, state?}` → `access_token`.
    /// Client never holds `ATLAS_WECOM_SECRET`.
    static func exchangeCode(
        hubHttpBase: String,
        code: String,
        state: String? = nil,
        session: URLSession = .shared,
        completion: @escaping (Result<ExchangeResponse, Error>) -> Void
    ) {
        guard let url = exchangeURL(hubHttpBase: hubHttpBase) else {
            completion(.failure(NSError(domain: "wecom", code: 1, userInfo: [NSLocalizedDescriptionKey: "bad exchange url"])))
            return
        }
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Accept")
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        var body: [String: String] = ["code": code]
        if let state, !state.isEmpty { body["state"] = state }
        do {
            req.httpBody = try JSONSerialization.data(withJSONObject: body)
        } catch {
            completion(.failure(error))
            return
        }
        session.dataTask(with: req) { data, resp, err in
            if let err { completion(.failure(err)); return }
            let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
            let bodyData = data ?? Data()
            guard (200...299).contains(status) else {
                let text = String(data: bodyData, encoding: .utf8) ?? ""
                completion(.failure(NSError(domain: "wecom", code: status, userInfo: [NSLocalizedDescriptionKey: "wecom exchange \(status): \(text)"])))
                return
            }
            do {
                completion(.success(try parseExchangeJSON(bodyData)))
            } catch {
                completion(.failure(error))
            }
        }.resume()
    }

    static func parseExchangeJSON(_ data: Data) throws -> ExchangeResponse {
        let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
        guard let token = obj["access_token"] as? String, !token.isEmpty else {
            throw NSError(domain: "wecom", code: 2, userInfo: [NSLocalizedDescriptionKey: "exchange missing access_token"])
        }
        return ExchangeResponse(
            accessToken: token,
            tokenType: obj["token_type"] as? String,
            expiresIn: (obj["expires_in"] as? NSNumber)?.int64Value,
            subject: obj["subject"] as? String
        )
    }
}

/// Derive Hub HTTP base from a Hub WS URL.
public func wsToHttpBase(_ ws: String) -> String {
    var s = ws.trimmingCharacters(in: .whitespacesAndNewlines)
    if s.hasPrefix("ws://") { s = "http://" + s.dropFirst(5) }
    else if s.hasPrefix("wss://") { s = "https://" + s.dropFirst(6) }
    if s.hasSuffix("/ws") { s = String(s.dropLast(3)) }
    while s.hasSuffix("/") { s.removeLast() }
    return s
}

/// `ATLAS_TICKET_PROVIDER` → oidc|wecom (default oidc).
public func ticketProviderFromEnv(_ raw: String?) -> String {
    let v = (raw ?? "").trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    if v == WeComAuth.provider { return WeComAuth.provider }
    return "oidc"
}
