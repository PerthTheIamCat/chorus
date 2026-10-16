import Foundation
import Security

/// Secure token manager backed by macOS Keychain Services.
///
/// Stores API keys (OpenAI, Anthropic, Gemini, GitHub) encrypted at rest,
/// avoiding plaintext token leakage in plain dotfiles or config repos.
final class KeychainManager {
    static let shared = KeychainManager()
    private let defaultService = "dev.chorus.app.tokens"

    private init() {}

    /// Saves or updates a token securely in Keychain.
    @discardableResult
    func saveToken(_ token: String, account: String, service: String? = nil) -> Bool {
        guard let data = token.data(using: .utf8) else { return false }
        let targetService = service ?? defaultService

        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: targetService,
            kSecAttrAccount as String: account,
        ]

        let updateAttributes: [String: Any] = [
            kSecValueData as String: data
        ]

        let status = SecItemUpdate(query as CFDictionary, updateAttributes as CFDictionary)
        if status == errSecSuccess {
            return true
        } else if status == errSecItemNotFound {
            var newItem = query
            newItem[kSecValueData as String] = data
            newItem[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
            let addStatus = SecItemAdd(newItem as CFDictionary, nil)
            return addStatus == errSecSuccess
        }

        return false
    }

    /// Retrieves an API token from Keychain.
    func loadToken(account: String, service: String? = nil) -> String? {
        let targetService = service ?? defaultService
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: targetService,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    /// Deletes a token from Keychain.
    @discardableResult
    func deleteToken(account: String, service: String? = nil) -> Bool {
        let targetService = service ?? defaultService
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: targetService,
            kSecAttrAccount as String: account
        ]

        let status = SecItemDelete(query as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound
    }
}
