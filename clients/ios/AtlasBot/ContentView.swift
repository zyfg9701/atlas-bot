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
                    // —— Top bar: Chat primary ——
                    Text("Atlas Bot · Chat")
                        .font(.title2.bold())
                    Text("State: \(model.state.rawValue)\(model.stateDetail.map { " (\($0))" } ?? "")")
                    Text(model.authStatus).font(.caption)
                    if model.state == .disconnected {
                        Text("开发态：dev Hub 无票可连 · Login 仍在主路径")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    HStack {
                        Button("Connect") { model.connect() }
                        Button("Disconnect") { model.disconnect() }
                    }
                    HStack {
                        Button("Login") { model.login() }
                        Button("Logout") { model.logout() }
                    }

                    DisclosureGroup(isExpanded: $model.advancedOpen) {
                        TextField("Hub WS URL", text: $model.hubUrl)
                            .textFieldStyle(.roundedBorder)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)
                        TextField("ATLAS_TICKET_PROVIDER (oidc|wecom)", text: $model.ticketProvider)
                            .textFieldStyle(.roundedBorder)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)
                        TextField("OIDC issuer (I2.2)", text: $model.oidcIssuer)
                            .textFieldStyle(.roundedBorder)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)
                        TextField("ATLAS_WECOM_CORP_ID", text: $model.wecomCorpId)
                            .textFieldStyle(.roundedBorder)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)
                        TextField("ATLAS_WECOM_AGENT_ID", text: $model.wecomAgentId)
                            .textFieldStyle(.roundedBorder)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)
                        TextField("ATLAS_WECOM_AUTHORIZE_BASE (mock)", text: $model.wecomAuthorizeBase)
                            .textFieldStyle(.roundedBorder)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)
                        TextField("Paste Bearer (optional)", text: $model.bearerPaste)
                            .textFieldStyle(.roundedBorder)
                            .autocapitalization(.none)
                            .disableAutocorrection(true)
                        Text("Redirect: \(MOBILE_REDIRECT_URI) · provider=\(model.ticketProvider) · no WebView · secret Hub-only")
                            .font(.caption2)
                    } label: {
                        Text("高级 · Hub / OIDC / WeCom")
                            .font(.subheadline.weight(.semibold))
                    }

                    if let err = model.lastError {
                        Text("Protocol error: \(err)")
                            .foregroundStyle(.red)
                    }

                    // —— Agent picker ——
                    Text("Agent").font(.headline)
                    Picker("Agent", selection: $model.selectedAgent) {
                        if model.roster.isEmpty {
                            Text(model.selectedAgent).tag(model.selectedAgent)
                        } else {
                            ForEach(model.roster) { a in
                                Text("\(a.agentId)  \(a.name)  (\(a.status))").tag(a.agentId)
                            }
                            if !model.roster.contains(where: { $0.agentId == model.selectedAgent }) {
                                Text(model.selectedAgent).tag(model.selectedAgent)
                            }
                        }
                    }
                    .pickerStyle(.menu)
                    TextField("agentId (hand-fill)", text: $model.selectedAgent)
                        .textFieldStyle(.roundedBorder)
                        .autocapitalization(.none)
                    Text("Connect 后自动 roster；Send 时未订则自动 subscribe。")
                        .font(.caption)
                        .foregroundStyle(.secondary)

                    // —— Conversation ——
                    Text("Conversation").font(.headline)
                    Group {
                        if model.conversation.isEmpty {
                            Text("(empty — Connect → Send；turn_finished 后自动刷新 transcript)")
                                .font(.caption)
                                .foregroundStyle(.secondary)
                        } else {
                            ForEach(Array(model.conversation.suffix(80).enumerated()), id: \.offset) { _, line in
                                Text(line).font(.caption.monospaced())
                            }
                        }
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .frame(minHeight: 100)

                    TextField("prompt", text: $model.prompt)
                        .textFieldStyle(.roundedBorder)
                    HStack {
                        Button("Send") { model.sendPrompt() }
                        Text("🖥📎 未接线 — 见调试占位")
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }

                    // —— More / Debug ——
                    DisclosureGroup(isExpanded: $model.debugOpen) {
                        Text("Cold path").font(.subheadline.weight(.semibold))
                        HStack {
                            Button("status") { model.fetchStatus() }
                            Button("roster") { model.fetchRoster() }
                        }
                        Text("runState: \(model.runState)").font(.caption)
                        ForEach(model.roster) { a in
                            Text("• \(a.agentId)  \(a.name)  (\(a.status))").font(.caption)
                        }

                        Text("Protocol · hot path").font(.subheadline.weight(.semibold))
                        HStack {
                            Button("subscribe") { model.subscribe() }
                            Button("unsubscribe") { model.unsubscribe() }
                            Button("transcriptTail") { model.refreshTranscript() }
                        }

                        Text("Connection").font(.subheadline.weight(.semibold))
                        Text("capabilities: \(model.capabilities.isEmpty ? "—" : model.capabilities)")
                            .font(.caption.monospaced())
                        Text("connection_id: \(model.connectionId)")
                            .font(.caption.monospaced())
                        Text("Redirect: \(MOBILE_REDIRECT_URI) · no WebView primary")
                            .font(.caption2)

                        Text("Desktop · upload").font(.subheadline.weight(.semibold))
                        Text("未接线（MU1 占位）— 不砍未来 VNC/upload 约定；PC U1 已有产品入口。")
                            .font(.caption)
                            .foregroundStyle(.secondary)

                        Text("Log").font(.subheadline.weight(.semibold))
                        ForEach(model.logs.prefix(40), id: \.self) { line in
                            Text(line).font(.caption.monospaced())
                        }
                    } label: {
                        Text("更多 / 调试")
                            .font(.subheadline.weight(.semibold))
                    }
                }
                .padding()
            }
            .navigationTitle("Atlas Bot")
            .onOpenURL { url in
                // URL Types backup for atlasbot://auth/callback (ASWebAuthenticationSession is primary).
                model.handleOpenURL(url)
            }
            .onChange(of: model.debugOpen) { open in
                UserDefaults.standard.set(open, forKey: BotViewModel.debugPrefsKey)
            } // iOS 16 single-parameter onChange
        }
    }
}

@MainActor
final class BotViewModel: ObservableObject, HubClientDelegate {
    static let debugPrefsKey = "atlas_bot_mu1_debug_open"

    @Published var hubUrl = DEFAULT_HUB_WS
    @Published var oidcIssuer = "http://127.0.0.1:8090"
    /// ATLAS_TICKET_PROVIDER (default oidc); advanced field — thin Login fork only.
    @Published var ticketProvider = ProcessInfo.processInfo.environment["ATLAS_TICKET_PROVIDER"].map { ticketProviderFromEnv($0) } ?? "oidc"
    @Published var wecomCorpId = ProcessInfo.processInfo.environment["ATLAS_WECOM_CORP_ID"] ?? "ww_mock_corp"
    @Published var wecomAgentId = ProcessInfo.processInfo.environment["ATLAS_WECOM_AGENT_ID"] ?? "1000001"
    @Published var wecomAuthorizeBase = ProcessInfo.processInfo.environment["ATLAS_WECOM_AUTHORIZE_BASE"] ?? "http://127.0.0.1:8091"
    @Published var bearerPaste = ""
    @Published var state: ConnState = .disconnected
    @Published var stateDetail: String?
    @Published var capabilities = ""
    @Published var connectionId = "—"
    @Published var authStatus = AuthSession.shared.hasTicket() ? "auth: ticket stored" : "auth: no ticket (dev OK)"
    @Published var runState = "—"
    @Published var selectedAgent = "agt_1"
    @Published var prompt = "hello from ios"
    @Published var transcript = ""
    @Published var lastError: String?
    @Published var roster: [RosterEntry] = []
    @Published var conversation: [String] = []
    @Published var logs: [String] = []
    @Published var advancedOpen = false
    @Published var debugOpen = UserDefaults.standard.bool(forKey: BotViewModel.debugPrefsKey)
    @Published var subscribedAgents: Set<String> = []

    private lazy var client: HubClient = HubClient(url: hubUrl, delegate: self)
    private let authSession = AuthSession.shared

    private func appendLog(_ line: String) {
        logs.insert(line, at: 0)
        if logs.count > 200 { logs.removeLast() }
    }

    private func appendConversation(_ line: String) {
        conversation.append(line)
        if conversation.count > 300 { conversation.removeFirst() }
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
        let pasted = bearerPaste.trimmingCharacters(in: .whitespacesAndNewlines)
        let bearer = authSession.currentBearer() ?? (pasted.isEmpty ? nil : pasted)
        client.setAuthorization(bearer)
        client.connect { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let ack):
                    self?.appendLog("ready \(ack.connectionId) user=\(ack.userId)")
                    self?.fetchRoster(auto: true)
                case .failure(let err): self?.showErr(err)
                }
            }
        }
    }

    func disconnect() { client.disconnect() }

    func login() {
        lastError = nil
        let provider = ticketProviderFromEnv(ticketProvider)
        let onDone: (Result<String, Error>) -> Void = { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success:
                    let p = self?.authSession.currentProvider() ?? provider
                    self?.authStatus = "auth: ticket stored (provider=\(p))"
                    self?.appendLog("login ok (\(p)) — Connect will send Authorization: Bearer")
                case .failure(let e):
                    self?.lastError = e.localizedDescription
                    self?.appendLog("login failed: \(e.localizedDescription)")
                }
            }
        }
        if provider == WeComAuth.provider {
            let corp = wecomCorpId.trimmingCharacters(in: .whitespacesAndNewlines)
            let agent = wecomAgentId.trimmingCharacters(in: .whitespacesAndNewlines)
            let authBase = wecomAuthorizeBase.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !corp.isEmpty, !agent.isEmpty, !authBase.isEmpty else {
                lastError = "WeCom corpId/agentId/authorizeBase required"
                return
            }
            let hubHttp = wsToHttpBase(hubUrl)
            appendLog("login → ASWebAuthenticationSession WeCom (\(MOBILE_REDIRECT_URI)) hub=\(hubHttp)")
            let cfg = WeComAuth.Config(
                corpId: corp,
                agentId: agent,
                authorizeBase: authBase,
                hubHttpBase: hubHttp
            )
            authSession.startWeComLogin(cfg: cfg, completion: onDone)
        } else {
            let issuer = oidcIssuer.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !issuer.isEmpty else {
                lastError = "OIDC issuer required"
                return
            }
            appendLog("login → ASWebAuthenticationSession PKCE (\(MOBILE_REDIRECT_URI))")
            let cfg = OidcClientConfig(issuer: issuer, clientId: DEFAULT_IOS_CLIENT_ID)
            authSession.startLogin(cfg: cfg, completion: onDone)
        }
    }

    func logout() {
        authSession.logout()
        client.setAuthorization(nil)
        client.disconnect()
        authStatus = "auth: logged out"
        appendLog("logout — Keychain cleared")
    }

    func handleOpenURL(_ url: URL) {
        authSession.handleCallbackURL(url) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success:
                    let p = self?.authSession.currentProvider() ?? "?"
                    self?.authStatus = "auth: ticket stored (provider=\(p))"
                    self?.appendLog("login ok via onOpenURL (\(p))")
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

    func fetchRoster(auto: Bool = false) {
        client.roster { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let agents):
                    self?.roster = agents
                    if let first = agents.first {
                        let keep = agents.contains(where: { $0.agentId == self?.selectedAgent })
                        if !keep { self?.selectedAgent = first.agentId }
                    }
                    self?.appendLog(auto ? "roster \(agents.count) agents (auto)" : "roster \(agents.count) agents")
                case .failure(let e):
                    if auto {
                        self?.appendLog("roster after connect failed (non-fatal): \(e.message)")
                    } else {
                        self?.showErr(e)
                    }
                }
            }
        }
    }

    func subscribe(then: (() -> Void)? = nil) {
        let id = selectedAgent.trimmingCharacters(in: .whitespacesAndNewlines)
        client.subscribe(agentIds: [id]) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success:
                    self?.subscribedAgents.insert(id)
                    self?.appendLog("subscribed \(id)")
                    then?()
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
                case .success:
                    self?.subscribedAgents.remove(id)
                    self?.appendLog("unsubscribed \(id)")
                case .failure(let e): self?.showErr(e)
                }
            }
        }
    }

    func sendPrompt() {
        let id = selectedAgent.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !id.isEmpty else {
            lastError = "agentId required"
            return
        }
        let doSend: () -> Void = { [weak self] in
            guard let self else { return }
            self.client.sendPrompt(agentId: id, prompt: self.prompt, immediate: true) { [weak self] result in
                Task { @MainActor in
                    switch result {
                    case .success:
                        self?.appendConversation("you: \(self?.prompt ?? "")")
                        self?.appendLog("sendPrompt ok")
                    case .failure(let e): self?.showErr(e)
                    }
                }
            }
        }
        if !subscribedAgents.contains(id) {
            subscribe(then: doSend)
        } else {
            doSend()
        }
    }

    func refreshTranscript() {
        let id = selectedAgent.trimmingCharacters(in: .whitespacesAndNewlines)
        client.getAgentTranscriptTail(agentId: id) { [weak self] result in
            Task { @MainActor in
                switch result {
                case .success(let r):
                    let text = "\(r ?? "(empty)")"
                    self?.transcript = text
                    self?.appendConversation("— transcript —\n\(text)")
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
            if state == .disconnected {
                self.subscribedAgents = []
                self.connectionId = "—"
            }
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
            self.connectionId = ack.connectionId
            self.appendLog("hello_ack conn=\(ack.connectionId) user=\(ack.userId)")
        }
    }

    nonisolated func hubClient(_ client: HubClient, didReceiveEvent event: BotEventEnvelope) {
        Task { @MainActor in
            let line = "event \(event.channel) agent=\(event.agentId) seq=\(event.seq)"
            self.appendLog(line)
            self.appendConversation(line)
            if event.channel == "hub:resync_required" {
                self.lastError = "hub:resync_required — re-fetch transcript"
            }
            if event.channel == "hub:turn_finished" {
                self.appendLog("turn_finished — refreshing transcript")
                let aid = event.agentId.isEmpty ? self.selectedAgent : event.agentId
                self.selectedAgent = self.selectedAgent // keep
                self.client.getAgentTranscriptTail(agentId: aid) { [weak self] result in
                    Task { @MainActor in
                        switch result {
                        case .success(let r):
                            let text = "\(r ?? "(empty)")"
                            self?.transcript = text
                            self?.appendConversation("— transcript —\n\(text)")
                            self?.appendLog("transcriptTail refreshed")
                        case .failure(let e): self?.showErr(e)
                        }
                    }
                }
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
