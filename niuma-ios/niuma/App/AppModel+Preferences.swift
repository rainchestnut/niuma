import Foundation

extension AppModel {
    func updateLanguage(_ language: AppLanguage) {
        appLanguage = language
        storage.set(language.rawValue, forKey: StorageKey.appLanguage.rawValue)
    }

    func updateTheme(_ theme: AppTheme) {
        appTheme = theme
        storage.set(theme.rawValue, forKey: StorageKey.appTheme.rawValue)
    }

    /// Persists the Niuma Server endpoint and rebuilds the transport layer when it changes.
    /// - Parameters:
    ///   - rawValue: User-entered server endpoint. Accepts `http`/`https` URLs and adds `https://` when omitted.
    /// - Returns: `true` when the endpoint is valid and applied.
    @discardableResult
    func updateServerBaseURL(from rawValue: String) async -> Bool {
        serverBaseURLValidationMessage = nil
        guard let nextURL = Self.normalizedServerBaseURL(from: rawValue) else {
            serverBaseURLValidationMessage = localized(
                "请输入有效的 http 或 https 服务地址。",
                "Enter a valid http or https server address."
            )
            return false
        }

        let nextText = Self.displayString(forServerBaseURL: nextURL)
        serverBaseURLText = nextText
        storage.set(nextText, forKey: StorageKey.serverBaseURL.rawValue)
        let isSameEndpoint = serverBaseURL?.absoluteString == nextURL.absoluteString
        guard !isSameEndpoint || controller == nil else { return true }

        isUpdatingServerBaseURL = true
        defer { isUpdatingServerBaseURL = false }
        replaceController(serverBaseURL: nextURL)
        if identity != nil, selectedAgent != nil {
            await refresh()
        }
        return true
    }

    /// Returns the localized string for the active app language.
    func localized(_ chinese: String, _ english: String) -> String {
        switch appLanguage {
        case .chinese:
            return chinese
        case .english:
            return english
        }
    }

    /// Authenticates the local identity with the paired desktop agent.
}
