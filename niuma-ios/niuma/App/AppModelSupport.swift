import CryptoKit
import Foundation

enum StorageKey: String {
    case selectedModelID
    case selectedReasoningEffort
    case approvalPermissionPreset
    case customApprovalPolicy
    case customApprovalsReviewer
    case customSandboxMode
    case appLanguage
    case appTheme
    case serverBaseURL
    case apnsDeviceToken
}

nonisolated struct PushThreadRoute: Hashable {
    let threadID: String
}

nonisolated struct TaskProgressNotificationPayload: Decodable {
    let threadID: String

    enum CodingKeys: String, CodingKey {
        case threadID = "thread_id"
    }
}

extension AppModel {
    /// Resolves the first server URL used at app launch from the phone-local setting.
    static func initialServerBaseURL(storage: UserDefaultsStore) -> URL? {
        if let stored = storage.string(forKey: StorageKey.serverBaseURL.rawValue),
           let url = normalizedServerBaseURL(from: stored) {
            return url
        }
        return nil
    }

    static func displayString(forServerBaseURL url: URL) -> String {
        normalizedServerBaseURL(from: url.absoluteString)?.absoluteString ?? url.absoluteString
    }

    static func normalizedServerBaseURL(from rawValue: String) -> URL? {
        var candidate = rawValue.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !candidate.isEmpty else { return nil }
        if !candidate.contains("://") {
            candidate = "https://\(candidate)"
        }
        guard var components = URLComponents(string: candidate),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              let host = components.host,
              !host.isEmpty
        else {
            return nil
        }
        components.scheme = scheme
        components.query = nil
        components.fragment = nil
        while components.path.count > 1, components.path.hasSuffix("/") {
            components.path.removeLast()
        }
        return components.url
    }
}

extension ThreadTimeline {
    /// Sorts timeline entries by the Codex-projected sequence and then by timestamp.
    nonisolated mutating func sortForTimelineDisplay() {
        entries.sort { left, right in
            if left.seq != right.seq {
                return left.seq < right.seq
            }
            return left.createdAt < right.createdAt
        }
    }
}

extension ThreadEntry {
    /// Matches a local-only optimistic user row to the canonical Codex user row.
    /// - Returns: `Bool` indicating whether this transient row has been superseded.
    nonisolated func matchesCanonicalUserPrompt(_ canonicalEntry: ThreadEntry) -> Bool {
        guard role == .user, canonicalEntry.role == .user else { return false }
        let localText = text.normalizedTimelineText
        let canonicalText = canonicalEntry.text.normalizedTimelineText
        if !localText.isEmpty, localText == canonicalText {
            return true
        }
        let localParts = contentPartSignature(contentParts)
        let canonicalParts = contentPartSignature(canonicalEntry.contentParts)
        return !localParts.isEmpty && localParts == canonicalParts
    }

    /// Builds a stable comparison key from content parts without exposing it as a protocol id.
    nonisolated func contentPartSignature(_ parts: [ContentPart]?) -> String {
        (parts ?? []).map { part in
            [
                part.kind.rawValue,
                part.text?.normalizedTimelineText ?? "",
                part.transferID ?? "",
                part.fileName ?? "",
                part.mimeType ?? ""
            ].joined(separator: "|")
        }
        .joined(separator: "\n")
    }

}

extension String {
    nonisolated var normalizedTimelineText: String {
        split(whereSeparator: \.isWhitespace).joined(separator: " ")
    }
}

func sha256Hex(_ data: Data) -> String {
    SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
}
