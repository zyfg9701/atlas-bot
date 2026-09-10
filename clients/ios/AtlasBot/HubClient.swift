import Foundation

#if canImport(UIKit)
import UIKit
#endif

/// Bot-Relay hub client (P4 iOS). Mirrors clients/pc Hub WS loop.
public let PROTOCOL_VERSION = "1.0.0"
#if targetEnvironment(simulator)
public let DEFAULT_HUB_WS = "ws://127.0.0.1:7700/ws"
#else
public let DEFAULT_HUB_WS = "ws://127.0.0.1:7700/ws" // replace with LAN IP on device
#endif

public enum ConnState: String {
    case disconnected, connecting, hello, ready, error
}

public struct HelloAck {
    public let connectionId: String
    public let userId: String
    public let computerHubVersion: String
    public let supportedProtocolVersions: [String]
    public let capabilities: [String]
}

public struct RosterEntry: Identifiable {
    public var id: String { agentId }
    public let agentId: String
    public let name: String
    public let status: String
    public let lastTurnAt: Int64?
}

public struct BotEventEnvelope {
    public let v: Int
    public let agentId: String
    public let seq: Int64
    public let channel: String
    public let event: Any?
}

public struct DisplayError: Error {
    public let message: String
    public let code: Any?
    public let reason: String?
    public let retryable: Bool?
    public let raw: Any?
}

private let knownCodes: Set<String> = [
    "command_rejected",
    "identity_unavailable",
    "link_state_unavailable",
    "upstream_error",
    "forbidden",
    "link_required",
    "link_removed",
    "consent_required",
    "box_unavailable",
    "computer_unavailable",
]

public func normalizeError(_ data: Any?) -> DisplayError {
    guard let dict = data as? [String: Any] else {
        return DisplayError(message: "unknown failure", code: "upstream_error", reason: nil, retryable: nil, raw: data)
    }
    if dict["message"] == nil && dict["code"] == nil {
        return DisplayError(message: "unknown failure", code: "upstream_error", reason: nil, retryable: nil, raw: data)
    }
    let msg = String(describing: dict["message"] ?? "error")
    let payload = (dict["data"] as? [String: Any]) ?? dict
    let reason = payload["reason"] as? String
    let retryable = payload["retryable"] as? Bool
    if !msg.isEmpty && !knownCodes.contains(msg) {
        if let codeNum = dict["code"] as? Int, codeNum < 0 {
            return DisplayError(message: msg, code: codeNum, reason: reason, retryable: retryable, raw: data)
        }
        return DisplayError(message: "failure (\(msg))", code: "upstream_error", reason: reason, retryable: false, raw: data)
    }
    let codeStr = knownCodes.contains(msg) ? msg : nil
    return DisplayError(message: msg, code: codeStr ?? dict["code"], reason: reason, retryable: retryable, raw: data)
}

/// JSON-RPC / hello frame builders — unit-tested without a socket.
public enum RpcFrames {
    public static func hello() -> [String: Any] {
        ["protocol_version": PROTOCOL_VERSION, "kind": "bot_client"]
    }

    public static func rpc(id: Int, method: String, params: [String: Any]) -> [String: Any] {
        ["jsonrpc": "2.0", "id": id, "method": method, "params": params]
    }

    public static func status() -> [String: Any] { [:] }
    public static func roster() -> [String: Any] { [:] }

    public static func subscribe(agentIds: [String], fullFidelity: Bool = false) -> [String: Any] {
        var o: [String: Any] = ["agentIds": agentIds]
        if fullFidelity { o["fullFidelity"] = true }
        return o
    }

    public static func unsubscribe(agentIds: [String]) -> [String: Any] {
        ["agentIds": agentIds]
    }

    public static func command(agentId: String, name: String, args: [String: Any]) -> [String: Any] {
        ["agentId": agentId, "name": name, "args": args]
    }

    public static func sendPromptArgs(agentId: String, prompt: String, immediate: Bool = false) -> [String: Any] {
        var args: [String: Any] = ["agentId": agentId, "prompt": prompt]
        if immediate { args["immediate"] = true }
        return args
    }

    public static func transcriptTailArgs(agentId: String, limit: Int = 20) -> [String: Any] {
        ["id": agentId, "limit": limit]
    }

    public static func encode(_ obj: [String: Any]) throws -> Data {
        try JSONSerialization.data(withJSONObject: obj, options: [])
    }
}

public protocol HubClientDelegate: AnyObject {
    func hubClient(_ client: HubClient, didChangeState state: ConnState, detail: String?)
    func hubClient(_ client: HubClient, didLog level: String, message: String)
    func hubClient(_ client: HubClient, didHelloAck ack: HelloAck)
    func hubClient(_ client: HubClient, didReceiveEvent event: BotEventEnvelope)
    func hubClient(_ client: HubClient, didRpcError error: DisplayError)
}

public final class HubClient: NSObject, URLSessionWebSocketDelegate {
    public private(set) var connectionState: ConnState = .disconnected
    public private(set) var subscribedAgents: Set<String> = []

    private var url: String
    private weak var delegate: HubClientDelegate?
    private var session: URLSession!
    private var task: URLSessionWebSocketTask?
    private var nextId = 1
    private var pending: [AnyHashable: (Result<Any?, DisplayError>) -> Void] = [:]
    private var lastSeq: [String: Int64] = [:]
    private var connectContinuation: ((Result<HelloAck, DisplayError>) -> Void)?
    /// Optional Hub Bearer. nil = today's no-token / ATLAS_AUTH_MODE=dev behavior.
    private var authorization: String?

    public init(url: String = DEFAULT_HUB_WS, delegate: HubClientDelegate? = nil) {
        self.url = url
        self.delegate = delegate
        super.init()
        let config = URLSessionConfiguration.default
        self.session = URLSession(configuration: config, delegate: self, delegateQueue: .main)
    }

    public func setUrl(_ url: String) { self.url = url }

    public func setAuthorization(_ token: String?) {
        let t = token?.trimmingCharacters(in: .whitespacesAndNewlines)
        authorization = (t?.isEmpty == false) ? t : nil
    }

    public func getAuthorization() -> String? { authorization }

    /// Build the WS upgrade request (unit-tested for optional Bearer header).
    public func buildConnectRequest() -> URLRequest? {
        guard let u = URL(string: url) else { return nil }
        var request = URLRequest(url: u)
        if let token = authorization {
            request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        }
        return request
    }

    private func setState(_ s: ConnState, detail: String? = nil) {
        connectionState = s
        delegate?.hubClient(self, didChangeState: s, detail: detail)
    }

    private func log(_ level: String, _ msg: String) {
        delegate?.hubClient(self, didLog: level, message: msg)
    }

    public func connect(authorization: String? = nil, completion: @escaping (Result<HelloAck, DisplayError>) -> Void) {
        if let authorization {
            setAuthorization(authorization)
        }
        disconnect()
        setState(.connecting)
        connectContinuation = completion
        guard let request = buildConnectRequest() else {
            let err = normalizeError(["message": "invalid url"])
            setState(.error, detail: err.message)
            completion(.failure(err))
            connectContinuation = nil
            return
        }
        task = session.webSocketTask(with: request)
        task?.resume()
        receiveLoop()
        // onOpen is not explicit for URLSessionWebSocket — send hello immediately after resume.
        setState(.hello)
        sendRaw(RpcFrames.hello())
        log("info", "→ hello")
    }

    public func disconnect() {
        task?.cancel(with: .goingAway, reason: nil)
        task = nil
        for (_, cb) in pending { cb(.failure(normalizeError(["message": "connection closed"]))) }
        pending.removeAll()
        lastSeq.removeAll()
        subscribedAgents.removeAll()
        setState(.disconnected)
    }

    private func receiveLoop() {
        task?.receive { [weak self] result in
            guard let self else { return }
            switch result {
            case .failure(let error):
                self.setState(.error, detail: error.localizedDescription)
                self.log("error", "websocket error \(error.localizedDescription)")
                if let cont = self.connectContinuation {
                    cont(.failure(normalizeError(["message": error.localizedDescription])))
                    self.connectContinuation = nil
                }
            case .success(let message):
                self.handleMessage(message)
                self.receiveLoop()
            }
        }
    }

    private func handleMessage(_ message: URLSessionWebSocketTask.Message) {
        let data: Data?
        switch message {
        case .string(let s): data = s.data(using: .utf8)
        case .data(let d): data = d
        @unknown default: data = nil
        }
        guard let data,
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            log("warn", "non-JSON frame")
            return
        }

        if obj["jsonrpc"] == nil, obj["connection_id"] != nil {
            let caps = obj["capabilities"] as? [String] ?? []
            let versions = obj["supported_protocol_versions"] as? [String] ?? []
            let ack = HelloAck(
                connectionId: obj["connection_id"] as? String ?? "",
                userId: obj["user_id"] as? String ?? "",
                computerHubVersion: obj["computer_hub_version"] as? String ?? "",
                supportedProtocolVersions: versions,
                capabilities: caps
            )
            setState(.ready)
            log("info", "← hello_ack")
            delegate?.hubClient(self, didHelloAck: ack)
            if let cont = connectContinuation {
                cont(.success(ack))
                connectContinuation = nil
            }
            return
        }

        if (obj["jsonrpc"] as? String) == "2.0", let id = obj["id"] {
            let key: AnyHashable
            if let i = id as? Int { key = i }
            else if let i = id as? NSNumber { key = i.intValue }
            else { key = "\(id)" }
            guard let cb = pending.removeValue(forKey: key) else {
                log("warn", "orphan response")
                return
            }
            if let errObj = obj["error"] {
                let err = normalizeError(errObj)
                log("error", "RPC error id=\(id)")
                delegate?.hubClient(self, didRpcError: err)
                cb(.failure(err))
            } else {
                log("info", "← result id=\(id)")
                cb(.success(obj["result"]))
            }
            return
        }

        if (obj["jsonrpc"] as? String) == "2.0", (obj["method"] as? String) == "bot.event",
           let params = obj["params"] as? [String: Any] {
            handleEvent(params)
            return
        }
        log("warn", "ignored frame")
    }

    private func handleEvent(_ params: [String: Any]) {
        let agentId = params["agentId"] as? String ?? ""
        let seq = (params["seq"] as? NSNumber)?.int64Value ?? Int64(params["seq"] as? Int ?? 0)
        let channel = params["channel"] as? String ?? ""
        if let prev = lastSeq[agentId], seq != prev + 1 {
            log("warn", "seq discontinuity agent=\(agentId) prev=\(prev) got=\(seq) (display only; no auto-resync)")
        }
        lastSeq[agentId] = seq
        if channel == "hub:resync_required" {
            log("warn", "hub:resync_required — re-fetch transcript (explicit)")
        } else if channel == "hub:turn_finished" {
            log("event", "hub:turn_finished")
        } else {
            log("event", "bot.event \(channel)")
        }
        let env = BotEventEnvelope(
            v: params["v"] as? Int ?? Int(BOT_EVENT_ENVELOPE_V),
            agentId: agentId,
            seq: seq,
            channel: channel,
            event: params["event"]
        )
        // Ignore events for unsubscribed agents after unsubscribe (still may arrive briefly).
        if !subscribedAgents.contains(agentId) {
            log("info", "ignoring event for unsubscribed agent \(agentId)")
            return
        }
        delegate?.hubClient(self, didReceiveEvent: env)
    }

    private func sendRaw(_ obj: [String: Any]) {
        guard let data = try? RpcFrames.encode(obj),
              let s = String(data: data, encoding: .utf8) else { return }
        task?.send(.string(s)) { [weak self] error in
            if let error {
                self?.log("error", "send failed \(error.localizedDescription)")
            }
        }
    }

    private func rpc(method: String, params: [String: Any], completion: @escaping (Result<Any?, DisplayError>) -> Void) {
        guard task != nil, connectionState == .ready else {
            completion(.failure(normalizeError(["message": "not connected"])))
            return
        }
        let id = nextId
        nextId += 1
        pending[id] = completion
        let frame = RpcFrames.rpc(id: id, method: method, params: params)
        log("info", "→ \(method) id=\(id)")
        sendRaw(frame)
    }

    public func status(completion: @escaping (Result<String, DisplayError>) -> Void) {
        rpc(method: "bot.status", params: RpcFrames.status()) { result in
            switch result {
            case .failure(let e): completion(.failure(e))
            case .success(let r):
                let runState = (r as? [String: Any])?["runState"] as? String ?? "?"
                completion(.success(runState))
            }
        }
    }

    public func roster(completion: @escaping (Result<[RosterEntry], DisplayError>) -> Void) {
        rpc(method: "bot.roster", params: RpcFrames.roster()) { result in
            switch result {
            case .failure(let e): completion(.failure(e))
            case .success(let r):
                let agents = ((r as? [String: Any])?["agents"] as? [[String: Any]] ?? []).map { a in
                    RosterEntry(
                        agentId: a["agentId"] as? String ?? "",
                        name: a["name"] as? String ?? "",
                        status: a["status"] as? String ?? "",
                        lastTurnAt: (a["lastTurnAt"] as? NSNumber)?.int64Value
                    )
                }
                completion(.success(agents))
            }
        }
    }

    public func subscribe(agentIds: [String], completion: @escaping (Result<Void, DisplayError>) -> Void) {
        rpc(method: "bot.subscribe", params: RpcFrames.subscribe(agentIds: agentIds)) { [weak self] result in
            switch result {
            case .failure(let e): completion(.failure(e))
            case .success:
                agentIds.forEach { self?.subscribedAgents.insert($0); self?.lastSeq.removeValue(forKey: $0) }
                completion(.success(()))
            }
        }
    }

    public func unsubscribe(agentIds: [String], completion: @escaping (Result<Void, DisplayError>) -> Void) {
        rpc(method: "bot.unsubscribe", params: RpcFrames.unsubscribe(agentIds: agentIds)) { [weak self] result in
            switch result {
            case .failure(let e): completion(.failure(e))
            case .success:
                agentIds.forEach { self?.subscribedAgents.remove($0) }
                completion(.success(()))
            }
        }
    }

    public func command(agentId: String, name: String, args: [String: Any] = [:],
                        completion: @escaping (Result<Any?, DisplayError>) -> Void) {
        rpc(method: "bot.command", params: RpcFrames.command(agentId: agentId, name: name, args: args), completion: completion)
    }

    public func sendPrompt(agentId: String, prompt: String, immediate: Bool = false,
                           completion: @escaping (Result<Any?, DisplayError>) -> Void) {
        command(agentId: agentId, name: "sendPrompt",
                args: RpcFrames.sendPromptArgs(agentId: agentId, prompt: prompt, immediate: immediate),
                completion: completion)
    }

    public func getAgentTranscriptTail(agentId: String, limit: Int = 20,
                                       completion: @escaping (Result<Any?, DisplayError>) -> Void) {
        command(agentId: agentId, name: "getAgentTranscriptTail",
                args: RpcFrames.transcriptTailArgs(agentId: agentId, limit: limit),
                completion: completion)
    }
}

// BOT_EVENT_ENVELOPE_V comes from generated BotRelayProtocol.swift (UInt32 / Int).
