import Foundation
import Security

protocol SecretStore {
    /// Reads secret bytes from secure storage.
    /// - Parameters:
    ///   - key: Secret account key passed as the `forKey key: String` argument.
    /// - Returns: Stored secret bytes, or `nil` when the key does not exist.
    func data(forKey key: String) -> Data?

    /// Writes secret bytes into secure storage.
    /// - Parameters:
    ///   - data: Secret bytes supplied as the `_ data: Data` argument.
    ///   - key: Secret account key passed as the `forKey key: String` argument.
    func set(_ data: Data, forKey key: String) throws
}

final class KeychainSecretStore: SecretStore {
    private let service: String

    init(service: String) {
        self.service = service
    }

    func data(forKey key: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]

        var item: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &item)
        guard status == errSecSuccess else { return nil }
        return item as? Data
    }

    func set(_ data: Data, forKey key: String) throws {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: key
        ]

        let attributes: [String: Any] = [
            kSecValueData as String: data
        ]

        let status: OSStatus
        if self.data(forKey: key) != nil {
            status = SecItemUpdate(query as CFDictionary, attributes as CFDictionary)
        } else {
            status = SecItemAdd(query.merging(attributes, uniquingKeysWith: { _, new in new }) as CFDictionary, nil)
        }

        guard status == errSecSuccess else {
            throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
        }
    }
}
