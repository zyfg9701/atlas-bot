import Foundation

/// W1 WeComProvider stub — same deep-link shape as I2.2 OIDC (M1).
///
/// Full mobile wiring is a follow-up; this PR nails the contract:
/// - Authorize URL: corpId/agentId; **no PKCE** (state + one-time code).
/// - Callback remains `atlasbot://auth/callback`.
/// - Exchange: Hub `POST /auth/wecom/exchange` (secret Hub-only).
/// - Store Hub JWT in Keychain via `TokenStore`; `HubClient` Bearer on upgrade.
/// - **No WebView** primary path (ASWebAuthenticationSession only).
/// - Provider switch: `ATLAS_TICKET_PROVIDER=wecom`; default oidc.
///
/// See docs/i2-login-runbook.md § WeCom.
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

    static func buildAuthorizeURL(cfg: Config, state: String) -> URL? {
        let base = cfg.authorizeBase.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        // keep trailing path: rebuild carefully
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
}
