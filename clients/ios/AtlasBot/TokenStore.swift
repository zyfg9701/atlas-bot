import Foundation
import Security

public protocol TokenStore {
    func saveBearer(_ token: String)
    func loadBearer() -> String?
    func clear()
}

public final class MemoryTokenStore: TokenStore {
    private var bearer: String?
    public init() {}
    public func saveBearer(_ token: String) { bearer = token }
    public func loadBearer() -> String? { bearer }
    public func clear() { bearer = nil }
}

/// Keychain store (`kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`).
public final class KeychainTokenStore: TokenStore {
    private let service: String
    private let account: String

    public init(service: String = "com.atlasbot.client.auth", account: String = "hub_bearer") {
        self.service = service
        self.account = account
    }

    public func saveBearer(_ token: String) {
        clear()
        let data = Data(token.utf8)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecValueData as String: data,
            kSecAttrAccessible as String: kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly,
        ]
        SecItemAdd(query as CFDictionary, nil)
    }

    public func loadBearer() -> String? {
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

    public func clear() {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
        ]
        SecItemDelete(query as CFDictionary)
    }
}
