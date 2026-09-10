import Foundation
import AuthenticationServices

#if canImport(UIKit)
import UIKit
#endif

/**
 M1 / WM1 iOS login: `ASWebAuthenticationSession`.
 OIDC uses PKCE S256; WeCom uses state + one-time code (no PKCE) → Hub exchange.
 Primary path is the **system browser session** — not an in-app WebView.

 Redirect: `atlasbot://auth/callback` (`callbackURLScheme` = `atlasbot`).
 Pending is typed separately (Oidc vs WeCom) so providers cannot mix.
 */
@MainActor
public final class AuthSession: NSObject {
    public static let shared = AuthSession()

    private enum Pending {
        case oidc(cfg: OidcClientConfig, redirect: String, state: String, pkce: PkcePair)
        case wecom(cfg: WeComAuth.Config, state: String)
    }

    private let tokenStore: TokenStore
    private var webAuthSession: ASWebAuthenticationSession?
    private var pending: Pending?
    private var urlSession: URLSession

    public init(tokenStore: TokenStore = KeychainTokenStore(), urlSession: URLSession = .shared) {
        self.tokenStore = tokenStore
        self.urlSession = urlSession
    }

    public func hasTicket() -> Bool {
        !(tokenStore.loadBearer() ?? "").isEmpty
    }

    public func currentBearer() -> String? { tokenStore.loadBearer() }

    public func currentProvider() -> String? { tokenStore.loadProvider() }

    public func saveManualBearer(_ token: String, provider: String? = nil) {
        tokenStore.saveBearer(token.trimmingCharacters(in: .whitespacesAndNewlines), provider: provider)
    }

    public func logout() {
        pending = nil
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
        pending = .oidc(cfg: cfg, redirect: redirectUri, state: state, pkce: pkce)
        let url = buildAuthorizeUrl(cfg: cfg, redirectUri: redirectUri, state: state, pkce: pkce)
        launchWebAuth(url: url, presentationAnchor: presentationAnchor, completion: completion)
    }

    /// Start WeCom authorize (no PKCE). Callback → Hub exchange → TokenStore (provider=wecom).
    public func startWeComLogin(
        cfg: WeComAuth.Config,
        presentationAnchor: ASPresentationAnchor? = nil,
        completion: @escaping (Result<String, Error>) -> Void
    ) {
        let state = randomState()
        pending = .wecom(cfg: cfg, state: state)
        guard let url = WeComAuth.buildAuthorizeURL(cfg: cfg, state: state) else {
            completion(.failure(NSError(domain: "wecom", code: 10, userInfo: [NSLocalizedDescriptionKey: "bad authorize url"])))
            return
        }
        launchWebAuth(url: url, presentationAnchor: presentationAnchor, completion: completion)
    }

    /// Thin dispatcher: `provider=wecom` → startWeComLogin; else OIDC startLogin.
    public func startLogin(
        provider: String,
        oidc: OidcClientConfig? = nil,
        wecom: WeComAuth.Config? = nil,
        redirectUri: String = MOBILE_REDIRECT_URI,
        presentationAnchor: ASPresentationAnchor? = nil,
        completion: @escaping (Result<String, Error>) -> Void
    ) {
        if ticketProviderFromEnv(provider) == WeComAuth.provider {
            guard let wecom else {
                completion(.failure(NSError(domain: "wecom", code: 11, userInfo: [NSLocalizedDescriptionKey: "wecom config required"])))
                return
            }
            startWeComLogin(cfg: wecom, presentationAnchor: presentationAnchor, completion: completion)
        } else {
            guard let oidc else {
                completion(.failure(NSError(domain: "oidc", code: 11, userInfo: [NSLocalizedDescriptionKey: "oidc config required"])))
                return
            }
            startLogin(cfg: oidc, redirectUri: redirectUri, presentationAnchor: presentationAnchor, completion: completion)
        }
    }

    /// Install WeCom pending without launching a browser (unit tests).
    public func prepareWeComLogin(cfg: WeComAuth.Config, state: String = randomState()) -> String {
        pending = .wecom(cfg: cfg, state: state)
        return state
    }

    /// Install OIDC pending without launching a browser (unit tests).
    public func prepareOidcLogin(
        cfg: OidcClientConfig,
        redirectUri: String = MOBILE_REDIRECT_URI,
        state: String = randomState(),
        pkce: PkcePair = generatePkce()
    ) -> String {
        pending = .oidc(cfg: cfg, redirect: redirectUri, state: state, pkce: pkce)
        return state
    }

    private func launchWebAuth(
        url: URL,
        presentationAnchor: ASPresentationAnchor?,
        completion: @escaping (Result<String, Error>) -> Void
    ) {
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
        guard let pending else {
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
        switch pending {
        case .oidc(let cfg, let redirect, let expectedState, let pkce):
            if let st = parsed.state, st != expectedState {
                completion(.failure(NSError(domain: "oidc", code: 8, userInfo: [NSLocalizedDescriptionKey: "state mismatch"])))
                return
            }
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
                        self?.tokenStore.saveBearer(bearer, provider: "oidc")
                        self?.pending = nil
                        completion(.success(bearer))
                    }
                }
            }
        case .wecom(let cfg, let expectedState):
            if let st = parsed.state, st != expectedState {
                completion(.failure(NSError(domain: "wecom", code: 8, userInfo: [NSLocalizedDescriptionKey: "state mismatch"])))
                return
            }
            WeComAuth.exchangeCode(
                hubHttpBase: cfg.hubHttpBase,
                code: code,
                state: expectedState,
                session: urlSession
            ) { [weak self] result in
                Task { @MainActor in
                    switch result {
                    case .failure(let e):
                        completion(.failure(e))
                    case .success(let tr):
                        self?.tokenStore.saveBearer(tr.accessToken, provider: WeComAuth.provider)
                        self?.pending = nil
                        completion(.success(tr.accessToken))
                    }
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
