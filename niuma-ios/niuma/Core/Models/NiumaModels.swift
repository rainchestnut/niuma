import Foundation

nonisolated let conversationProjectID = "__conversation__"

nonisolated enum AppLanguage: String, Codable, CaseIterable {
    case chinese
    case english
}

nonisolated enum AppTheme: String, Codable, CaseIterable {
    case system
    case light
    case dark
}

nonisolated enum DeviceLifecycleState: String, Codable {
    case uninitialized
    case deviceRegistered
    case unpaired
    case pairing
    case paired
    case pairFailed
}

nonisolated enum RealtimeConnectionState: String, Codable {
    case disconnected
    case authenticating
    case connected
    case degraded
    case retrying
}

nonisolated enum ThreadRuntimeState: String, Codable {
    case idle
    case submitting
    case streaming
    case waitingApproval
    case reconnecting
    case completed
    case failed
}

/// Reasoning effort values exposed by mobile for Codex `turn/start`.
nonisolated enum ReasoningEffort: String, Codable, CaseIterable, Identifiable {
    case low
    case medium
    case high
    case xhigh

    var id: String { rawValue }
}

/// Mobile-facing presets for Codex approval and sandbox settings.
nonisolated enum ApprovalPermissionPreset: String, Codable, CaseIterable, Identifiable {
    case defaultPermissions
    case autoReview
    case fullAccess
    case custom

    var id: String { rawValue }
}

nonisolated enum CodexApprovalPolicy: String, Codable, CaseIterable, Identifiable {
    case untrusted
    case onRequest = "on-request"
    case never

    var id: String { rawValue }
}

nonisolated enum CodexApprovalsReviewer: String, Codable, CaseIterable, Identifiable {
    case user
    case guardianSubagent = "guardian_subagent"

    var id: String { rawValue }
}

nonisolated enum CodexSandboxMode: String, Codable, CaseIterable, Identifiable {
    case readOnly = "read-only"
    case workspaceWrite = "workspace-write"
    case dangerFullAccess = "danger-full-access"

    var id: String { rawValue }
}

nonisolated struct ApprovalPermissionOverrides: Codable, Hashable {
    let approvalPolicy: CodexApprovalPolicy?
    let approvalsReviewer: CodexApprovalsReviewer?
    let sandboxMode: CodexSandboxMode?
}

/// Thread lifecycle values accepted from both Niuma and Codex App Server projections.
nonisolated enum ThreadStatus: String, Codable, CaseIterable {
    case notLoaded
    case created
    case idle
    case running
    case waitingApproval = "waiting_approval"
    case pending
    case threadStarted = "thread_started"
    case completed
    case failed
    case systemError = "systemError"
    case cancelled
    case closed
    case archived
    case unknown

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        let rawValue = try container.decode(String.self)
        self = ThreadStatus(rawValue: rawValue) ?? .unknown
    }

    /// Encodes the thread status raw value while preserving unknown fallback behavior.
    func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(rawValue)
    }
}

nonisolated enum ApprovalStatus: String, Codable {
    case pending
    case submitting
    case resolved
    case failed
}

nonisolated enum ThreadEntryRole: String, Codable {
    case user
    case assistant
    case system
    case approval
}

nonisolated enum ApprovalDecision: String, Codable {
    case allow
    case reject
}

nonisolated enum ApprovalGrantScopeKind: String, Codable {
    case turn
    case session
}

nonisolated struct ApprovalGrantScope: Codable, Hashable {
    let scope: ApprovalGrantScopeKind
    let threadID: String?
    let approvalType: String?
    let ttlSeconds: Int?

    enum CodingKeys: String, CodingKey {
        case scope
        case threadID = "thread_id"
        case approvalType = "approval_type"
        case ttlSeconds = "ttl_seconds"
    }

    static func session(threadID: String, approvalType: String) -> ApprovalGrantScope {
        ApprovalGrantScope(
            scope: .session,
            threadID: threadID,
            approvalType: approvalType,
            ttlSeconds: nil
        )
    }
}

/// Direction of an encrypted temporary file transfer through niuma-server.
nonisolated enum TransferDirection: String, Codable, Hashable {
    case iosToAgent = "ios_to_agent"
    case agentToIOS = "agent_to_ios"
}

/// Unified decrypted message content shape shared by text, images, videos, and files.
nonisolated enum ContentPartKind: String, Codable, Hashable {
    case text
    case fileRef = "file_ref"
    case fileChangeSummary = "file_change_summary"
}

nonisolated struct FileChangeFileSummary: Codable, Hashable {
    let path: String
    let changeType: String
    let additions: Int
    let deletions: Int

    enum CodingKeys: String, CodingKey {
        case path
        case changeType = "change_type"
        case additions
        case deletions
    }
}

nonisolated struct ContentPart: Codable, Hashable {
    let kind: ContentPartKind
    let text: String?
    let transferID: String?
    let fileType: String?
    let fileName: String?
    let mimeType: String?
    let sizeBytes: Int?
    let alt: String?
    let files: Int?
    let additions: Int?
    let deletions: Int?
    let filesSummary: [FileChangeFileSummary]?

    init(
        kind: ContentPartKind,
        text: String? = nil,
        transferID: String? = nil,
        fileType: String? = nil,
        fileName: String? = nil,
        mimeType: String? = nil,
        sizeBytes: Int? = nil,
        alt: String? = nil,
        files: Int? = nil,
        additions: Int? = nil,
        deletions: Int? = nil,
        filesSummary: [FileChangeFileSummary]? = nil
    ) {
        self.kind = kind
        self.text = text
        self.transferID = transferID
        self.fileType = fileType
        self.fileName = fileName
        self.mimeType = mimeType
        self.sizeBytes = sizeBytes
        self.alt = alt
        self.files = files
        self.additions = additions
        self.deletions = deletions
        self.filesSummary = filesSummary
    }

    enum CodingKeys: String, CodingKey {
        case kind = "type"
        case text
        case transferID = "transfer_id"
        case fileType = "file_type"
        case fileName = "file_name"
        case mimeType = "mime_type"
        case sizeBytes = "size_bytes"
        case alt
        case files
        case additions
        case deletions
        case filesSummary = "files_summary"
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        let decodedKind = try container.decode(ContentPartKind.self, forKey: .kind)
        kind = decodedKind
        text = try container.decodeIfPresent(String.self, forKey: .text)
        transferID = try container.decodeIfPresent(String.self, forKey: .transferID)
        fileName = try container.decodeIfPresent(String.self, forKey: .fileName)
        mimeType = try container.decodeIfPresent(String.self, forKey: .mimeType)
        sizeBytes = try container.decodeIfPresent(Int.self, forKey: .sizeBytes)
        alt = try container.decodeIfPresent(String.self, forKey: .alt)
        files = try container.decodeIfPresent(Int.self, forKey: .files)
        additions = try container.decodeIfPresent(Int.self, forKey: .additions)
        deletions = try container.decodeIfPresent(Int.self, forKey: .deletions)
        filesSummary = try container.decodeIfPresent([FileChangeFileSummary].self, forKey: .filesSummary)
        fileType = try container.decodeIfPresent(String.self, forKey: .fileType)
            ?? ContentPart.inferredFileType(kind: decodedKind, mimeType: mimeType)
    }

    func encode(to encoder: Encoder) throws {
        var container = encoder.container(keyedBy: CodingKeys.self)
        try container.encode(kind, forKey: .kind)
        try container.encodeIfPresent(text, forKey: .text)
        try container.encodeIfPresent(transferID, forKey: .transferID)
        try container.encodeIfPresent(fileType, forKey: .fileType)
        try container.encodeIfPresent(fileName, forKey: .fileName)
        try container.encodeIfPresent(mimeType, forKey: .mimeType)
        try container.encodeIfPresent(sizeBytes, forKey: .sizeBytes)
        try container.encodeIfPresent(alt, forKey: .alt)
        try container.encodeIfPresent(files, forKey: .files)
        try container.encodeIfPresent(additions, forKey: .additions)
        try container.encodeIfPresent(deletions, forKey: .deletions)
        try container.encodeIfPresent(filesSummary, forKey: .filesSummary)
    }

    private static func inferredFileType(
        kind: ContentPartKind,
        mimeType: String?
    ) -> String? {
        guard kind == .fileRef else { return nil }
        if let mimeType {
            if mimeType.hasPrefix("image/") { return "image" }
            if mimeType.hasPrefix("video/") { return "video" }
        }
        return "file"
    }
}

/// Decrypted business payload stored in `ciphertext` after E2E encryption.
nonisolated struct ContentPartsPayload: Codable, Hashable {
    let contentParts: [ContentPart]

    enum CodingKeys: String, CodingKey {
        case contentParts = "content_parts"
    }
}

nonisolated struct FileChangeDiffBundle: Codable, Hashable {
    let version: Int
    let source: String
    let turnID: String?
    let finalAnswerEntryID: String?
    let summary: FileChangeBundleSummary?
    let files: [FileChangeDiffFile]

    enum CodingKeys: String, CodingKey {
        case version
        case source
        case turnID = "turn_id"
        case finalAnswerEntryID = "final_answer_entry_id"
        case summary
        case files
    }
}

nonisolated struct FileChangeBundleSummary: Codable, Hashable {
    let files: Int
    let additions: Int
    let deletions: Int
}

nonisolated struct FileChangeDiffFile: Codable, Identifiable, Hashable {
    let path: String
    let oldPath: String?
    let changeType: String
    let additions: Int
    let deletions: Int
    let rawDiff: String
    let hunks: [FileChangeDiffHunk]

    var id: String { path }

    enum CodingKeys: String, CodingKey {
        case path
        case oldPath = "old_path"
        case changeType = "change_type"
        case additions
        case deletions
        case rawDiff = "raw_diff"
        case hunks
    }
}

nonisolated struct FileChangeDiffHunk: Codable, Identifiable, Hashable {
    let oldStart: Int
    let oldLines: Int
    let newStart: Int
    let newLines: Int
    let lines: [FileChangeDiffLine]

    var id: String { "\(oldStart)-\(newStart)-\(lines.count)" }

    enum CodingKeys: String, CodingKey {
        case oldStart = "old_start"
        case oldLines = "old_lines"
        case newStart = "new_start"
        case newLines = "new_lines"
        case lines
    }
}

nonisolated struct FileChangeDiffLine: Codable, Identifiable, Hashable {
    let kind: String
    let oldLine: Int?
    let newLine: Int?
    let content: String

    var id: String { "\(kind)-\(oldLine ?? -1)-\(newLine ?? -1)-\(content.hashValue)" }

    enum CodingKeys: String, CodingKey {
        case kind
        case oldLine = "old_line"
        case newLine = "new_line"
        case content
    }
}

nonisolated struct OutgoingAttachment: Identifiable, Hashable {
    let id = UUID()
    let fileType: String
    let fileName: String
    let mimeType: String
    let data: Data
    let alt: String?
}

nonisolated struct LocalDeviceIdentity: Codable, Equatable {
    let deviceID: String
    let publicKey: String
    let encryptionPublicKey: String
    let displayName: String
}

nonisolated struct PairedAgent: Codable, Identifiable, Hashable {
    let bindingID: String
    let agentID: String
    let agentEncryptionPublicKey: String
    var displayName: String
    var fingerprint: String
    var pairedAt: Date
    var sessionToken: String?
    var isOnline: Bool = false

    var id: String { agentID }
}

nonisolated struct ProjectSummary: Codable, Identifiable, Hashable {
    let projectID: String
    let agentID: String
    let projectName: String
    let updatedAt: Date

    var id: String { projectID }
}

nonisolated struct ThreadSummary: Codable, Identifiable, Hashable {
    let threadID: String
    let projectID: String
    let agentID: String
    let title: String
    let status: ThreadStatus
    let lastCheckpointSeen: String?
    let currentBranch: String?
    let updatedAt: Date

    var id: String { threadID }
}

nonisolated struct ApprovalSummary: Codable, Identifiable, Hashable {
    let approvalID: String
    let threadID: String
    let agentID: String
    let approvalType: String
    let requestMethod: String?
    let paramsJSON: String?
    var status: ApprovalStatus
    let updatedAt: Date

    var id: String { approvalID }
}

nonisolated struct UserInputQuestionOption: Codable, Hashable {
    let label: String
    let description: String
}

nonisolated struct UserInputQuestion: Codable, Hashable {
    let questionID: String
    let header: String
    let prompt: String
    let options: [UserInputQuestionOption]
    let isOther: Bool
    let isSecret: Bool
}

nonisolated struct UserInputRequestSummary: Codable, Identifiable, Hashable {
    let requestID: String
    let threadID: String
    let agentID: String
    let questions: [UserInputQuestion]
    var status: ApprovalStatus
    let updatedAt: Date

    var id: String { requestID }
}

nonisolated struct ThreadEntry: Codable, Identifiable, Hashable {
    let id: String
    let role: ThreadEntryRole
    let type: String
    let phase: String?
    let text: String
    let createdAt: Date?
    let seq: Int
    let contentParts: [ContentPart]?

    init(
        id: String,
        role: ThreadEntryRole,
        type: String,
        phase: String? = nil,
        text: String,
        createdAt: Date?,
        seq: Int,
        contentParts: [ContentPart]? = nil
    ) {
        self.id = id
        self.role = role
        self.type = type
        self.phase = phase
        self.text = text
        self.createdAt = createdAt
        self.seq = seq
        self.contentParts = contentParts
    }
}

nonisolated struct ThreadTimeline: Codable, Hashable {
    let threadID: String
    var ackSeq: Int
    var checkpoint: String?
    var entries: [ThreadEntry]
}

nonisolated enum ThreadRefreshPhase: String {
    case idle
    case refreshing
    case succeeded
    case failed
    case timedOut
}

nonisolated struct ThreadRefreshStatus {
    let refreshID: String?
    let phase: ThreadRefreshPhase
    let error: String?

    var isRefreshing: Bool {
        phase == .refreshing
    }

    static func idle() -> ThreadRefreshStatus {
        ThreadRefreshStatus(
            refreshID: nil,
            phase: .idle,
            error: nil
        )
    }
}

nonisolated struct ChallengePayload: Codable, Hashable {
    let challengeID: String
    let challenge: String
    let expiresAt: Int
}

nonisolated struct SessionTokenPayload: Codable, Hashable {
    let sessionToken: String
}

nonisolated struct RealtimeTaskUpdate: Hashable {
    let threadID: String
    let seq: Int
    let checkpoint: String?
    let entry: ThreadEntry
}

nonisolated struct ThreadSyncCompletion: Hashable {
    let threadID: String
    let cursor: Int
    let checkpoint: String?
    let entryCount: Int
}

nonisolated struct ThreadSyncFailure: Hashable {
    let threadID: String
    let cursor: Int
    let checkpoint: String?
    let error: String
}

nonisolated struct ModelSyncState: Codable, Hashable {
    let currentModel: String?
    let availableModels: [String]
}

nonisolated struct TransferReady: Hashable {
    let transferID: String
    let direction: TransferDirection
    let sourceDeviceID: String
    let targetDeviceID: String
    let encryptedSizeBytes: Int
    let expiresAt: Int
}

nonisolated struct LocalAttachmentPayload: Hashable {
    let transferID: String
    let direction: TransferDirection
    let sourceDeviceID: String
    let targetDeviceID: String
    let storedAt: Date
    let fileName: String?
    let mimeType: String?
    let sizeBytes: Int?
    let localRelativePath: String
}

nonisolated struct MetadataRefreshResult: Hashable {
    let requestID: String
    let succeeded: Bool
    let error: String?
}

nonisolated struct BranchChangesResult: Hashable {
    let requestID: String
    let threadID: String
    let succeeded: Bool
    let summary: FileChangeBundleSummary?
    let filesSummary: [FileChangeFileSummary]
    let transferID: String?
    let sizeBytes: Int?
    let error: String?
}

nonisolated struct ThreadArchiveResult: Hashable {
    let requestID: String
    let threadID: String
    let succeeded: Bool
    let error: String?
}

nonisolated struct ThreadRenameResult: Hashable {
    let requestID: String
    let threadID: String
    let succeeded: Bool
    let error: String?
}

nonisolated struct ApprovalResponseFailure: Hashable {
    let approvalID: String
    let error: String
}

nonisolated struct UserInputResponseFailure: Hashable {
    let requestID: String
    let error: String
}

nonisolated enum RealtimeEvent: Hashable {
    case projectSync(ProjectSummary)
    case threadSync(ThreadSummary)
    case taskUpdate(RealtimeTaskUpdate)
    case threadSyncCompleted(ThreadSyncCompletion)
    case threadSyncFailed(ThreadSyncFailure)
    case modelSync(ModelSyncState)
    case transferReady(TransferReady)
    case metadataRefreshResult(MetadataRefreshResult)
    case branchChangesResult(BranchChangesResult)
    case threadArchiveResult(ThreadArchiveResult)
    case threadRenameResult(ThreadRenameResult)
    case approvalRequest(ApprovalSummary)
    case approvalResponseFailed(ApprovalResponseFailure)
    case userInputRequest(UserInputRequestSummary)
    case userInputResponseFailed(UserInputResponseFailure)
    case deviceStatus(agentID: String, online: Bool)
}

nonisolated enum AppModelError: LocalizedError {
    case invalidPairPayload
    case pairingRevocationFailed
    case missingPrivateKey
    case missingSessionToken
    case realtimeNotConnected
    case realtimeOperationStalled
    case missingProjectSelection
    case missingPrompt
    case transferChecksumMismatch
    case transferIdentifierMismatch
    case serverForgotDevice
    case serverNotConfigured
    case missingDeviceIdentity
    case missingAgentBinding
    case approvalNotPending
    case userInputNotPending

    var errorDescription: String? {
        switch self {
        case .invalidPairPayload:
            return String(localized: "error.invalid_pair_payload", table: "Localizable")
        case .pairingRevocationFailed:
            return String(localized: "error.pairing_revocation_failed", table: "Localizable")
        case .missingPrivateKey:
            return String(localized: "error.missing_private_key", table: "Localizable")
        case .missingSessionToken:
            return String(localized: "error.missing_session_token", table: "Localizable")
        case .realtimeNotConnected:
            return String(localized: "error.realtime_not_connected", table: "Localizable")
        case .realtimeOperationStalled:
            return String(localized: "error.realtime_operation_stalled", table: "Localizable")
        case .missingProjectSelection:
            return String(localized: "error.missing_project_selection", table: "Localizable")
        case .missingPrompt:
            return String(localized: "error.missing_prompt", table: "Localizable")
        case .transferChecksumMismatch:
            return String(localized: "error.transfer_checksum_mismatch", table: "Localizable")
        case .transferIdentifierMismatch:
            return String(localized: "error.transfer_identifier_mismatch", table: "Localizable")
        case .serverForgotDevice:
            return String(localized: "error.server_forgot_device", table: "Localizable")
        case .serverNotConfigured:
            return String(localized: "error.server_not_configured", table: "Localizable")
        case .missingDeviceIdentity:
            return String(localized: "error.missing_device_identity", table: "Localizable")
        case .missingAgentBinding:
            return String(localized: "error.missing_agent_binding", table: "Localizable")
        case .approvalNotPending:
            return String(localized: "error.approval_not_pending", table: "Localizable")
        case .userInputNotPending:
            return String(localized: "error.user_input_not_pending", table: "Localizable")
        }
    }
}
