import Foundation

nonisolated struct DecodedCiphertextContent {
    let text: String
    let parts: [ContentPart]?
}

nonisolated struct EmptyBody: Encodable {}

nonisolated enum HTTPMethod: String {
    case get = "GET"
    case post = "POST"
    case put = "PUT"
}

nonisolated struct LiveRealtimeEnvelope: Decodable {
    let kind: String
}

nonisolated struct LiveTaskStartMessage: Encodable {
    let kind: String
    let deviceID: String
    let agentID: String
    let projectID: String
    let threadID: String?
    let ciphertext: String
    let signature: String
    let model: String?
    let effort: String?
    let approvalPolicy: String?
    let approvalsReviewer: String?
    let sandboxMode: String?

    enum CodingKeys: String, CodingKey {
        case kind
        case deviceID = "device_id"
        case agentID = "agent_id"
        case projectID = "project_id"
        case threadID = "thread_id"
        case ciphertext
        case signature
        case model
        case effort
        case approvalPolicy = "approval_policy"
        case approvalsReviewer = "approvals_reviewer"
        case sandboxMode = "sandbox_mode"
    }
}

nonisolated struct LiveMetadataRefreshMessage: Encodable {
    let kind: String
    let requestID: String
    let deviceID: String

    enum CodingKeys: String, CodingKey {
        case kind
        case requestID = "request_id"
        case deviceID = "device_id"
    }
}

nonisolated struct LiveBranchChangesRequestMessage: Encodable {
    let kind: String
    let requestID: String
    let deviceID: String
    let threadID: String
    let baseRef: String?

    enum CodingKeys: String, CodingKey {
        case kind
        case requestID = "request_id"
        case deviceID = "device_id"
        case threadID = "thread_id"
        case baseRef = "base_ref"
    }
}

nonisolated struct LiveThreadArchiveRequestMessage: Encodable {
    let kind: String
    let requestID: String
    let deviceID: String
    let threadID: String

    enum CodingKeys: String, CodingKey {
        case kind
        case requestID = "request_id"
        case deviceID = "device_id"
        case threadID = "thread_id"
    }
}

nonisolated struct LiveProjectSyncMessage: Decodable {
    let projectID: String
    let projectName: String
    let updatedAt: TimeInterval?
    let agentID: String?

    enum CodingKeys: String, CodingKey {
        case projectID = "project_id"
        case projectName = "project_name"
        case updatedAt = "updated_at"
        case agentID = "agent_id"
    }
}

nonisolated struct LiveThreadSyncMessage: Decodable {
    let threadID: String
    let projectID: String?
    let title: String
    let status: ThreadStatus
    let lastCheckpointSeen: String?
    let updatedAt: TimeInterval?
    let agentID: String?

    enum CodingKeys: String, CodingKey {
        case threadID = "thread_id"
        case projectID = "project_id"
        case title
        case status
        case lastCheckpointSeen = "last_checkpoint_seen"
        case updatedAt = "updated_at"
        case agentID = "agent_id"
    }
}

nonisolated struct LiveResumeThreadMessage: Encodable {
    let kind: String
    let threadID: String
    let cursor: Int
    let checkpoint: String?

    enum CodingKeys: String, CodingKey {
        case kind
        case threadID = "thread_id"
        case cursor
        case checkpoint
    }
}

nonisolated struct LiveThreadSyncCompletedMessage: Decodable {
    let threadID: String
    let cursor: Int
    let checkpoint: String?
    let entryCount: Int

    enum CodingKeys: String, CodingKey {
        case threadID = "thread_id"
        case cursor
        case checkpoint
        case entryCount = "entry_count"
    }
}

nonisolated struct LiveThreadSyncFailedMessage: Decodable {
    let threadID: String
    let cursor: Int
    let checkpoint: String?
    let error: String

    enum CodingKeys: String, CodingKey {
        case threadID = "thread_id"
        case cursor
        case checkpoint
        case error
    }
}

nonisolated struct LiveModelSyncMessage: Decodable {
    let currentModel: String?
    let availableModels: [String]

    enum CodingKeys: String, CodingKey {
        case currentModel = "current_model"
        case availableModels = "available_models"
    }
}

nonisolated struct LiveTaskUpdateMessage: Decodable {
    let kind: String
    let deviceID: String
    let threadID: String
    let seq: Int
    let ciphertext: String
    let checkpoint: String?
    let role: String?
    let type: String
    let phase: String?
    let projectID: String?
    let entryID: String?
    let createdAt: Double?

    enum CodingKeys: String, CodingKey {
        case kind
        case deviceID = "device_id"
        case threadID = "thread_id"
        case seq
        case ciphertext
        case checkpoint
        case role
        case type
        case phase
        case projectID = "project_id"
        case entryID = "entry_id"
        case createdAt = "created_at"
    }
}

nonisolated struct LiveTransferReadyMessage: Decodable {
    let kind: String
    let transferID: String
    let direction: TransferDirection
    let sourceDeviceID: String
    let targetDeviceID: String
    let encryptedSizeBytes: Int
    let expiresAt: Int

    enum CodingKeys: String, CodingKey {
        case kind
        case transferID = "transfer_id"
        case direction
        case sourceDeviceID = "source_device_id"
        case targetDeviceID = "target_device_id"
        case encryptedSizeBytes = "encrypted_size_bytes"
        case expiresAt = "expires_at"
    }
}

nonisolated struct LiveMetadataRefreshCompletedMessage: Decodable {
    let requestID: String

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
    }
}

nonisolated struct LiveMetadataRefreshFailedMessage: Decodable {
    let requestID: String
    let error: String

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case error
    }
}

nonisolated struct LiveBranchChangesEnvelopeMessage: Decodable {
    let requestID: String
    let deviceID: String
    let threadID: String
    let ciphertext: String

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case deviceID = "device_id"
        case threadID = "thread_id"
        case ciphertext
    }
}

nonisolated struct BranchChangesResultPlaintext: Decodable {
    let summary: FileChangeBundleSummary
    let filesSummary: [FileChangeFileSummary]
    let transferID: String
    let sizeBytes: Int?
    let baseRef: String?

    enum CodingKeys: String, CodingKey {
        case summary
        case filesSummary = "files_summary"
        case transferID = "transfer_id"
        case sizeBytes = "size_bytes"
        case baseRef = "base_ref"
    }
}

nonisolated struct BranchChangesFailedPlaintext: Decodable {
    let error: String
}

nonisolated struct LiveThreadArchiveResultMessage: Decodable {
    let requestID: String
    let deviceID: String
    let threadID: String
    let error: String?

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case deviceID = "device_id"
        case threadID = "thread_id"
        case error
    }
}

nonisolated struct LiveApprovalRequestMessage: Decodable {
    let deviceID: String
    let approvalID: String
    let threadID: String
    let approvalType: String
    let ciphertext: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
        case approvalID = "approval_id"
        case threadID = "thread_id"
        case approvalType = "approval_type"
        case ciphertext
    }
}

nonisolated struct LiveApprovalResponseMessage: Encodable {
    let kind: String
    let approvalID: String
    let ciphertext: String

    enum CodingKeys: String, CodingKey {
        case kind
        case approvalID = "approval_id"
        case ciphertext
    }
}

nonisolated struct LiveApprovalSyncMessage: Decodable {
    let approvalID: String
    let threadID: String
    let approvalType: String
    let status: String

    enum CodingKeys: String, CodingKey {
        case approvalID = "approval_id"
        case threadID = "thread_id"
        case approvalType = "approval_type"
        case status
    }
}

nonisolated struct LiveApprovalResponseFailedMessage: Decodable {
    let approvalID: String
    let error: String

    enum CodingKeys: String, CodingKey {
        case approvalID = "approval_id"
        case error
    }
}

nonisolated struct LiveUserInputAnswerMessage: Codable {
    let answers: [String]
}

nonisolated struct ApprovalResponsePlaintext: Encodable {
    let approvalID: String
    let decision: String
    let grantScope: ApprovalGrantScope?

    enum CodingKeys: String, CodingKey {
        case approvalID = "approval_id"
        case decision
        case grantScope = "grant_scope"
    }
}

nonisolated struct UserInputResponsePlaintext: Encodable {
    let requestID: String
    let answers: [String: LiveUserInputAnswerMessage]

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case answers
    }
}

nonisolated struct LiveUserInputQuestionOptionMessage: Codable {
    let label: String
    let description: String
}

nonisolated struct LiveUserInputQuestionMessage: Codable {
    let questionID: String
    let header: String
    let prompt: String
    let options: [LiveUserInputQuestionOptionMessage]
    let isOther: Bool
    let isSecret: Bool

    enum CodingKeys: String, CodingKey {
        case questionID = "question_id"
        case header
        case prompt
        case options
        case isOther = "is_other"
        case isSecret = "is_secret"
    }
}

nonisolated struct LiveUserInputRequestMessage: Decodable {
    let deviceID: String
    let requestID: String
    let threadID: String
    let status: String
    let ciphertext: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
        case requestID = "request_id"
        case threadID = "thread_id"
        case status
        case ciphertext
    }
}

nonisolated struct UserInputRequestPlaintext: Decodable {
    let questions: [LiveUserInputQuestionMessage]
}

nonisolated struct LiveUserInputResponseMessage: Encodable {
    let kind: String
    let ciphertext: String

    enum CodingKeys: String, CodingKey {
        case kind
        case ciphertext
    }
}

nonisolated struct LiveUserInputSyncMessage: Decodable {
    let requestID: String
    let threadID: String
    let status: String

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case threadID = "thread_id"
        case status
    }
}
