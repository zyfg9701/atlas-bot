import Foundation
import Security

public protocol TokenStore {
    func saveBearer(_ token: String, provider: String?)
    func loadBearer() -> String?
    func loadProvider() -> String?
    func clear()
}

public extension TokenStore {
    func saveBearer(_ token: String) { saveBearer(token, provider: nil) }
}

public final class MemoryTokenStore: TokenStore {
    private var bearer: String?
    private var provider: String?
    public init() {}
    public func saveBearer(_ token: String, provider: String?) {
        bearer = token
        self.provider = provider
    }
    public func loadBearer() -> String? { bearer }
    public func loadProvider() -> String? { provider }
    public func clear() {
        bearer = nil
        provider = nil
    }
}

/// Keychain store (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`).
public final class KeychainTokenStore: TokenStore {
    private let service: String
    private let account: String
    private let providerAccount: String

    public init(
        service: String = "com.atlasbot.client.auth",
        account: String = "hub_bearer",
        providerAccount: String = "ticket_provider"
    ) {
        self.service = service
        self.account = account
        self.providerAccount = providerAccount
    }

    public func saveBearer(_ token: String, provider: String?) {
        clear()
        write(account: account, value: token)
        if let provider, !provider.isEmpty {
            write(account: providerAccount, value: provider)
        }
    }

    public func loadBearer() -> String? { read(account: account) }

    public func loadProvider() -> String? { read(account: providerAccount) }

    public func clear() {
        delete(account: account)
        delete(account: providerAccount)
    }

    private func write(account: String, value: String) {
        let data = Data(value.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        SecItemAdd(query as CFDictionary, nil)
    }

    private func read(account: String) -> String? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var out: AnyObject?
        let status = SecItemCopyMatching(query as CFDictionary, &out)
        guard status == errSecSuccess, let data = out as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    private func delete(account: String) {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}
