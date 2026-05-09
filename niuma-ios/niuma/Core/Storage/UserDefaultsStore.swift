import Foundation

protocol KeyValueStore {
    /// Reads a string value from the key-value store.
    /// - Parameters:
    ///   - key: Storage key passed as the `forKey key: String` argument.
    /// - Returns: Stored string, or `nil` when the key is absent.
    func string(forKey key: String) -> String?

    /// Writes or clears a string value in the key-value store.
    /// - Parameters:
    ///   - value: Optional string supplied as the `_ value: String?` argument; `nil` clears the key.
    ///   - key: Storage key passed as the `forKey key: String` argument.
    func set(_ value: String?, forKey key: String)
}

final class UserDefaultsStore: KeyValueStore {
    private let defaults: UserDefaults

    init(defaults: UserDefaults) {
        self.defaults = defaults
    }

    func string(forKey key: String) -> String? {
        defaults.string(forKey: key)
    }

    func set(_ value: String?, forKey key: String) {
        if let value {
            defaults.set(value, forKey: key)
        } else {
            defaults.removeObject(forKey: key)
        }
    }

    /// Removes every value stored in the app's UserDefaults domain.
    func removeAllAppValues() {
        guard let bundleIdentifier = Bundle.main.bundleIdentifier else {
            for key in defaults.dictionaryRepresentation().keys {
                defaults.removeObject(forKey: key)
            }
            return
        }
        defaults.removePersistentDomain(forName: bundleIdentifier)
    }
}
