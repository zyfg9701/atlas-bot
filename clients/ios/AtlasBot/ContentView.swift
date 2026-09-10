import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

struct ContentView: View {
    @StateObject private var model = BotViewModel()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Atlas Bot P4 (iOS)")
                        .font(.title2.bold())
                    Text("State: \(model.state.rawValue)\(model.stateDetail.map { " (\($0))" } ?? "")")
                    Text(model.authStatus).font(.caption)
                    if let err = model.lastError {
                        Text("Protocol error: \(err)")
                            .foregroundStyle(.red)
                    }

                    TextField("Hub WS URL", text: $model.hubUrl)
                        .textFieldStyle(.roundedBorder)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                    TextField("OIDC issuer (I2.2)", text: $model.oidcIssuer)
                        .textFieldStyle(.roundedBorder)
                        .autocapitalization(.none)
                        .disableAutocorrection(true)
                    HStack {
                        Button("Connect") { model.connect() }
                        Button("Disconnect") { model.disconnect() }
                    }
                    HStack {
                        Button("Login") { model.login() }
                        Button("Logout") { model.logout() }
                    }
                    Text("Redirect: \(MOBILE_REDIRECT_URI) · client_id=\(DEFAULT_IOS_CLIENT_ID) · no WebView")
                        .font(.caption2)
                    if !model.capabilities.isEmpty {
                        Text("Capabilities: \(model.capabilities)")
                            .font(.caption)
                    }

                    Text("Cold path").font(.headline)
                    HStack {
                        Button("status") { model.fetchStatus() }
                        Button("roster") { model.fetchRoster() }
                    }
                    Text("runState: \(model.runState)")
                    ForEach(model.roster) { a in
                        Text("• \(a.agentId)  \(a.name)  (\(a.status))")
                    }

                    Text("Hot path").font(.headline)
                    TextField("agentId", text: $model.selectedAgent)
                        .textFieldStyle(.roundedBorder)
                        .autocapitalization(.none)
                    HStack {
                        Button("subscribe") { model.subscribe() }
                        Button("unsubscribe") { model.unsubscribe() }
                    }
                    TextField("prompt", text: $model.prompt)
                        .textFieldStyle(.roundedBorder)
                    HStack {
                        Button("sendPrompt") { model.sendPrompt() }
                        Button("transcriptTail") { model.refreshTranscript() }
                    }

                    Text("Transcript").font(.headline)
                    Text(model.transcript.isEmpty
                        ? "(none yet — wait for hub:turn_finished then refresh)"
                        : model.transcript)
                        .font(.body.monospaced())

                    Text("Log").font(.headline)
                    ForEach(model.logs.prefix(40), id: \.self) { line in
                        Text(line).font(.caption.monospaced())
                    }
                }
                .padding()
            }
            .navigationTitle("Atlas Bot")
            .onOpenURL { url in
                // URL Types backup for atlasbot://auth/callback (ASWebAuthenticationSession is primary).
                model.handleOpenURL(url)
            }
        }
    }
}

@MainActor
final class BotViewModel: ObservableObject, HubClientDelegate {
    @Published var hubUrl = DEFAULT_HUB_WS
    @Published var oidcIssuer = "http://127.0.0.1:8090"
    @Published var state: ConnState = .disconnected
    @Published var stateDetail: String?
    @Published var capabilities = ""
    @Published var authStatus = AuthSession.shared.hasTicket() ? "auth: ticket stored" : "auth: no ticket (dev OK)"
    @Published var runState = "—"
    @Published var selectedAgent = "agt_1"
    @Published var prompt = "hello from ios"
    @Published var transcript = ""
    @Published var lastError: String?
    @Published var roster: [RosterEntry] = []
    @Published var logs: [String] = []

    private lazy var client: HubClient = HubClient(url: hubUrl, delegate: self)
    private let auth = AuthSession.shared

    private func appendLog(_ line: String) {
        logs.insert(line, at: 0)
        if logs.count > 200 { logs.removeLast() }
    }

    private func showErr(_ err: DisplayError) {
        var s = err.message
        if let r = err.reason { s += " reason=\(r)" }
        if let c = err.code { s += " code=\(c)" }
        lastError = s
        appendLog("[error] \(s)")
    }

    func connect() {
        lastError = nil
        client.setUrl(hubUrl.trimmingCharacters(in: .whitespacesAndNewlines))
        client.setAuthorization(auth.currentBearer())
        client.connect { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let ack): self?.appendLog("ready \(ack.connectionId) user=\(ack.userId)")
                case .failure(let err): self?.showErr(err)
                }
            }
        }
    }

    func disconnect() { client.disconnect() }

    func login() {
        lastError = nil
        let issuer = oidcIssuer.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !issuer.isEmpty else {
            lastError = "OIDC issuer required"
            return
        }
        appendLog("login → ASWebAuthenticationSession PKCE (\(MOBILE_REDIRECT_URI))")
        let cfg = OidcClientConfig(issuer: issuer, clientId: DEFAULT_IOS_CLIENT_ID)
        auth.startLogin(cfg: cfg) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success:
                    self?.authStatus = "auth: ticket stored (id_token preferred)"
                    self?.appendLog("login ok — Connect will send Authorization: Bearer")
                case .failure(let e):
                    self?.lastError = e.localizedDescription
                    self?.appendLog("login failed: \(e.localizedDescription)")
                }
            }
        }
    }

    func logout() {
        auth.logout()
        client.setAuthorization(nil)
        client.disconnect()
        authStatus = "auth: logged out"
        appendLog("logout — Keychain cleared")
    }

    func handleOpenURL(_ url: URL) {
        auth.handleCallbackURL(url) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success:
                    self?.authStatus = "auth: ticket stored (id_token preferred)"
                    self?.appendLog("login ok via onOpenURL")
                case .failure(let e):
                    self?.lastError = e.localizedDescription
                }
            }
        }
    }

    func fetchStatus() {
        client.status { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let s):
                    self?.runState = s
                    self?.appendLog("status runState=\(s)")
                case .failure(let e): self?.showErr(e)
                }
            }
        }
    }

    func fetchRoster() {
        client.roster { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let agents):
                    self?.roster = agents
                    self?.appendLog("roster \(agents.count) agents")
                case .failure(let e): self?.showErr(e)
                }
            }
        }
    }

    func subscribe() {
        let id = selectedAgent.trimmingCharacters(in: .whitespacesAndNewlines)
        client.subscribe(agentIds: [id]) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success: self?.appendLog("subscribed \(id)")
                case .failure(let e): self?.showErr(e)
                }
            }
        }
    }

    func unsubscribe() {
        let id = selectedAgent.trimmingCharacters(in: .whitespacesAndNewlines)
        client.unsubscribe(agentIds: [id]) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success: self?.appendLog("unsubscribed \(id)")
                case .failure(let e): self?.showErr(e)
                }
            }
        }
    }

    func sendPrompt() {
        let id = selectedAgent.trimmingCharacters(in: .whitespacesAndNewlines)
        client.sendPrompt(agentId: id, prompt: prompt, immediate: true) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success: self?.appendLog("sendPrompt ok")
                case .failure(let e): self?.showErr(e)
                }
            }
        }
    }

    func refreshTranscript() {
        let id = selectedAgent.trimmingCharacters(in: .whitespacesAndNewlines)
        client.getAgentTranscriptTail(agentId: id) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let r):
                    self?.transcript = "\(r ?? "(empty)")"
                    self?.appendLog("transcriptTail refreshed")
                case .failure(let e): self?.showErr(e)
                }
            }
        }
    }

    // MARK: - HubClientDelegate

    nonisolated func hubClient(_ client: HubClient, didChangeState state: ConnState, detail: String?) {
        Task { @MainActor in
            self.state = state
            self.stateDetail = detail
        }
    }

    nonisolated func hubClient(_ client: HubClient, didLog level: String, message: String) {
        Task { @MainActor in
            self.appendLog("[\(level)] \(message)")
        }
    }

    nonisolated func hubClient(_ client: HubClient, didHelloAck ack: HelloAck) {
        Task { @MainActor in
            self.capabilities = ack.capabilities.joined(separator: ", ")
            self.appendLog("hello_ack conn=\(ack.connectionId) user=\(ack.userId)")
        }
    }

    nonisolated func hubClient(_ client: HubClient, didReceiveEvent event: BotEventEnvelope) {
        Task { @MainActor in
            self.appendLog("event \(event.channel) agent=\(event.agentId) seq=\(event.seq)")
            if event.channel == "hub:resync_required" {
                self.lastError = "hub:resync_required — re-fetch transcript"
            }
            if event.channel == "hub:turn_finished" {
                self.appendLog("turn_finished — tap transcriptTail to refresh")
            }
        }
    }

    nonisolated func hubClient(_ client: HubClient, didRpcError error: DisplayError) {
        Task { @MainActor in
            self.showErr(error)
        }
    }
}

#Preview {
    ContentView()
}
