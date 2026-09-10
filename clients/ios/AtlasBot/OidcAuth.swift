import Foundation
import CryptoKit

/// Custom scheme redirect (IdP must register alongside PC loopback).
public let MOBILE_REDIRECT_URI = "atlasbot://auth/callback"
public let MOBILE_CALLBACK_SCHEME = "atlasbot"
public let DEFAULT_IOS_CLIENT_ID = "atlas-bot-ios"

public struct PkcePair {
    public let verifier: String
    public let challenge: String
    public let method: String
    public init(verifier: String, challenge: String, method: String = "S256") {
        self.verifier = verifier
        self.challenge = challenge
        self.method = method
    }
}

public struct OidcClientConfig {
    public let issuer: String
    public let clientId: String
    public let scopes: [String]
    public let audience: String?

    public init(
        issuer: String,
        clientId: String = DEFAULT_IOS_CLIENT_ID,
        scopes: [String] = ["openid", "profile"],
        audience: String? = nil
    ) {
        self.issuer = issuer
        self.clientId = clientId
        self.scopes = scopes
        self.audience = audience
    }

    public var authorizeUrl: String { issuer.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "/authorize" }
    public var tokenUrl: String { issuer.trimmingCharacters(in: CharacterSet(charactersIn: "/")) + "/token" }
}

public struct TokenResponse {
    public var accessToken: String?
    public var idToken: String?
    public var refreshToken: String?
    public var tokenType: String?
    public var expiresIn: Int64?
}

public struct CallbackParse {
    public var code: String?
    public var state: String?
    public var error: String?
}

private let verifierAlphabet =
    Array("ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")

public func randomVerifier(_ len: Int = 64) -> String {
    let n = max(43, min(128, len))
    var out = ""
    out.reserveCapacity(n)
    for _ in 0..<n {
        let idx = Int.random(in: 0..<verifierAlphabet.count)
        out.append(verifierAlphabet[idx])
    }
    return out
}

public func s256Challenge(_ verifier: String) -> String {
    let data = Data(verifier.utf8)
    let digest = SHA256.hash(data: data)
    return Data(digest).base64URLEncodedString()
}

public func generatePkce() -> PkcePair {
    let verifier = randomVerifier(64)
    return PkcePair(verifier: verifier, challenge: s256Challenge(verifier))
}

public func randomState() -> String { randomVerifier(32) }

public func buildAuthorizeUrl(
    cfg: OidcClientConfig,
    redirectUri: String,
    state: String,
    pkce: PkcePair
) -> URL {
    var comps = URLComponents(string: cfg.authorizeUrl)!
    var items: [URLQueryItem] = [
        URLQueryItem(name: "response_type", value: "code"),
        URLQueryItem(name: "client_id", value: cfg.clientId),
        URLQueryItem(name: "redirect_uri", value: redirectUri),
        URLQueryItem(name: "scope", value: cfg.scopes.joined(separator: " ")),
        URLQueryItem(name: "state", value: state),
        URLQueryItem(name: "code_challenge", value: pkce.challenge),
        URLQueryItem(name: "code_challenge_method", value: pkce.method),
    ]
    if let aud = cfg.audience, !aud.isEmpty {
        items.append(URLQueryItem(name: "audience", value: aud))
    }
    comps.queryItems = items
    return comps.url!
}

public func parseCallbackUrl(_ callbackUrl: String) -> CallbackParse {
    guard let comps = URLComponents(string: callbackUrl) else {
        return CallbackParse()
    }
    let items = comps.queryItems ?? []
    func val(_ name: String) -> String? {
        items.first(where: { $0.name == name })?.value
    }
    if let err = val("error"), !err.isEmpty {
        return CallbackParse(error: err, state: val("state"))
    }
    return CallbackParse(code: val("code"), state: val("state"))
}

public func looksLikeJwt(_ token: String) -> Bool {
    let parts = token.split(separator: ".", omittingEmptySubsequences: false)
    return parts.count == 3 && parts.allSatisfy { !$0.isEmpty }
}

/// Prefer id_token; else JWT access_token (same as pick_hub_bearer / PC).
public func pickHubBearer(_ tr: TokenResponse) -> String? {
    if let id = tr.idToken, !id.isEmpty { return id }
    if let access = tr.accessToken, looksLikeJwt(access) { return access }
    if let access = tr.accessToken, !access.isEmpty { return access }
    if let id = tr.idToken, !id.isEmpty { return id }
    return nil
}

public func parseTokenJson(_ data: Data) throws -> TokenResponse {
    let obj = try JSONSerialization.jsonObject(with: data) as? [String: Any] ?? [:]
    return TokenResponse(
        accessToken: obj["access_token"] as? String,
        idToken: obj["id_token"] as? String,
        refreshToken: obj["refresh_token"] as? String,
        tokenType: obj["token_type"] as? String,
        expiresIn: (obj["expires_in"] as? NSNumber)?.int64Value
    )
}

public func exchangeCode(
    cfg: OidcClientConfig,
    redirectUri: String,
    code: String,
    codeVerifier: String,
    completion: @escaping (Result<TokenResponse, Error>) -> Void
) {
    guard let url = URL(string: cfg.tokenUrl) else {
        completion(.failure(NSError(domain: "oidc", code: 1, userInfo: [NSLocalizedDescriptionKey: "bad token url"])))
        return
    }
    var req = URLRequest(url: url)
    req.httpMethod = "POST"
    req.setValue("application/json", forHTTPHeaderField: "Accept")
    req.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
    var body = URLComponents()
    body.queryItems = [
        URLQueryItem(name: "grant_type", value: "authorization_code"),
        URLQueryItem(name: "code", value: code),
        URLQueryItem(name: "redirect_uri", value: redirectUri),
        URLQueryItem(name: "client_id", value: cfg.clientId),
        URLQueryItem(name: "code_verifier", value: codeVerifier),
    ]
    req.httpBody = body.percentEncodedQuery?.data(using: .utf8)
    URLSession.shared.dataTask(with: req) { data, resp, err in
        if let err { completion(.failure(err)); return }
        let status = (resp as? HTTPURLResponse)?.statusCode ?? 0
        let bodyData = data ?? Data()
        guard (200...299).contains(status) else {
            let text = String(data: bodyData, encoding: .utf8) ?? ""
            completion(.failure(NSError(domain: "oidc", code: status, userInfo: [NSLocalizedDescriptionKey: "token endpoint \(status): \(text)"])))
            return
        }
        do {
            completion(.success(try parseTokenJson(bodyData)))
        } catch {
            completion(.failure(error))
        }
    }.resume()
}

private extension Data {
    func base64URLEncodedString() -> String {
        base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }
}
