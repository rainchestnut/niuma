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

    func selectApprovalPermissionPreset(_ preset: ApprovalPermissionPreset) {
        approvalPermissionPreset = preset
        storage.set(preset.rawValue, forKey: StorageKey.approvalPermissionPreset.rawValue)
    }

    func selectCustomApprovalPolicy(_ policy: CodexApprovalPolicy) {
        customApprovalPolicy = policy
        storage.set(policy.rawValue, forKey: StorageKey.customApprovalPolicy.rawValue)
    }

    func selectCustomApprovalsReviewer(_ reviewer: CodexApprovalsReviewer) {
        customApprovalsReviewer = reviewer
        storage.set(reviewer.rawValue, forKey: StorageKey.customApprovalsReviewer.rawValue)
    }

    func selectCustomSandboxMode(_ mode: CodexSandboxMode) {
        customSandboxMode = mode
        storage.set(mode.rawValue, forKey: StorageKey.customSandboxMode.rawValue)
    }

    /// Converts the mobile permission preset into Codex app-server overrides.
    /// The default preset intentionally omits overrides so desktop config stays authoritative.
    func approvalPermissionOverrides() -> ApprovalPermissionOverrides {
        switch approvalPermissionPreset {
        case .defaultPermissions:
            return ApprovalPermissionOverrides(
                approvalPolicy: nil,
                approvalsReviewer: nil,
                sandboxMode: nil
            )
        case .autoReview:
            return ApprovalPermissionOverrides(
                approvalPolicy: .onRequest,
                approvalsReviewer: .guardianSubagent,
                sandboxMode: .workspaceWrite
            )
        case .fullAccess:
            return ApprovalPermissionOverrides(
                approvalPolicy: .never,
                approvalsReviewer: nil,
                sandboxMode: .dangerFullAccess
            )
        case .custom:
            return ApprovalPermissionOverrides(
                approvalPolicy: customApprovalPolicy,
                approvalsReviewer: customApprovalsReviewer,
                sandboxMode: customSandboxMode
            )
        }
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
