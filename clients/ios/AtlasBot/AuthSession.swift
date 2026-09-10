import Foundation
import AuthenticationServices

#if canImport(UIKit)
import UIKit
#endif

/**
 M1 iOS login: `ASWebAuthenticationSession` + PKCE S256.
 Primary path is the **system browser session** — not an in-app WebView.

 Redirect: `atlasbot://auth/callback` (`callbackURLScheme` = `atlasbot`).
 */
@MainActor
public final class AuthSession: NSObject {
    public static let shared = AuthSession()

    private let tokenStore: TokenStore
    private var webAuthSession: ASWebAuthenticationSession?
    private var pendingState: String?
    private var pendingPkce: PkcePair?
    private var pendingCfg: OidcClientConfig?
    private var pendingRedirect: String = MOBILE_REDIRECT_URI

    public init(tokenStore: TokenStore = KeychainTokenStore()) {
        self.tokenStore = tokenStore
    }

    public func hasTicket() -> Bool {
        !(tokenStore.loadBearer() ?? "").isEmpty
    }

    public func currentBearer() -> String? { tokenStore.loadBearer() }

    public func saveManualBearer(_ token: String) {
        tokenStore.saveBearer(token.trimmingCharacters(in: .whitespacesAndNewlines))
    }

    public func logout() {
        pendingState = nil
        pendingPkce = nil
        pendingCfg = nil
        webAuthSession?.cancel()
        webAuthSession = nil
        tokenStore.clear()
    }

    public func startLogin(
        cfg: OidcClientConfig,
        redirectUri: String = MOBILE_REDIRECT_URI,
        presentationAnchor: ASPresentationAnchor? = nil,
        completion: @escaping (Result<String, Error>) -> Void
    ) {
        let pkce = generatePkce()
        let state = randomState()
        pendingPkce = pkce
        pendingState = state
        pendingCfg = cfg
        pendingRedirect = redirectUri
        let url = buildAuthorizeUrl(cfg: cfg, redirectUri: redirectUri, state: state, pkce: pkce)

        let session = ASWebAuthenticationSession(
            url: url,
            callbackURLScheme: MOBILE_CALLBACK_SCHEME
        ) { [weak self] callbackURL, error in
            Task { @MainActor in
                guard let self else { return }
                if let error {
                    completion(.failure(error))
                    return
                }
                guard let callbackURL else {
                    completion(.failure(NSError(domain: "oidc", code: 2, userInfo: [NSLocalizedDescriptionKey: "missing callback"])))
                    return
                }
                self.finishLogin(callbackURL: callbackURL, completion: completion)
            }
        }
        session.prefersEphemeralWebBrowserSession = false
        #if canImport(UIKit)
        if let anchor = presentationAnchor {
            session.presentationContextProvider = AnchorProvider(anchor: anchor)
        } else if let scene = UIApplication.shared.connectedScenes.first as? UIWindowScene,
                  let window = scene.windows.first(where: \.isKeyWindow) ?? scene.windows.first {
            session.presentationContextProvider = AnchorProvider(anchor: window)
        }
        #endif
        webAuthSession = session
        if !session.start() {
            completion(.failure(NSError(domain: "oidc", code: 3, userInfo: [NSLocalizedDescriptionKey: "failed to start ASWebAuthenticationSession"])))
        }
    }

    /// Optional deep-link entry (URL Types) if session callback is delivered via `onOpenURL`.
    public func handleCallbackURL(_ url: URL, completion: @escaping (Result<String, Error>) -> Void) {
        guard url.scheme == MOBILE_CALLBACK_SCHEME else {
            completion(.failure(NSError(domain: "oidc", code: 4, userInfo: [NSLocalizedDescriptionKey: "unexpected scheme"])))
            return
        }
        finishLogin(callbackURL: url, completion: completion)
    }

    private func finishLogin(callbackURL: URL, completion: @escaping (Result<String, Error>) -> Void) {
        guard let cfg = pendingCfg, let pkce = pendingPkce, let expectedState = pendingState else {
            completion(.failure(NSError(domain: "oidc", code: 5, userInfo: [NSLocalizedDescriptionKey: "no pending login"])))
            return
        }
        let parsed = parseCallbackUrl(callbackURL.absoluteString)
        if let err = parsed.error {
            completion(.failure(NSError(domain: "oidc", code: 6, userInfo: [NSLocalizedDescriptionKey: err])))
            return
        }
        guard let code = parsed.code, !code.isEmpty else {
            completion(.failure(NSError(domain: "oidc", code: 7, userInfo: [NSLocalizedDescriptionKey: "missing code"])))
            return
        }
        if let st = parsed.state, st != expectedState {
            completion(.failure(NSError(domain: "oidc", code: 8, userInfo: [NSLocalizedDescriptionKey: "state mismatch"])))
            return
        }
        let redirect = pendingRedirect
        exchangeCode(cfg: cfg, redirectUri: redirect, code: code, codeVerifier: pkce.verifier) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .failure(let e):
                    completion(.failure(e))
                case .success(let tr):
                    guard let bearer = pickHubBearer(tr) else {
                        completion(.failure(NSError(domain: "oidc", code: 9, userInfo: [NSLocalizedDescriptionKey: "no id_token/access_token"])))
                        return
                    }
                    self?.tokenStore.saveBearer(bearer)
                    self?.pendingCfg = nil
                    self?.pendingPkce = nil
                    self?.pendingState = nil
                    completion(.success(bearer))
                }
            }
        }
    }
}

#if canImport(UIKit)
private final class AnchorProvider: NSObject, ASWebAuthenticationPresentationContextProviding {
    let anchor: ASPresentationAnchor
    init(anchor: ASPresentationAnchor) { self.anchor = anchor }
    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor { anchor }
}
#endif
