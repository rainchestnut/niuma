import Foundation

struct ChallengeRequestData: Encodable {
    let deviceID: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
    }
}

struct ChallengeResponseData: Decodable {
    let challengeID: String
    let challenge: String
    let expiresAt: Int

    enum CodingKeys: String, CodingKey {
        case challengeID = "challenge_id"
        case challenge
        case expiresAt = "expires_at"
    }
}

struct VerifyRequestData: Encodable {
    let deviceID: String
    let challengeID: String
    let timestamp: Int
    let nonce: String
    let requestDigest: String
    let signature: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
        case challengeID = "challenge_id"
        case timestamp
        case nonce
        case requestDigest = "request_digest"
        case signature
    }
}

struct VerifyResponseData: Decodable {
    let verified: Bool
    let sessionToken: String

    enum CodingKeys: String, CodingKey {
        case verified
        case sessionToken = "session_token"
    }
}

struct PairConfirmRequestData: Encodable {
    let deviceID: String
    let deviceName: String
    let publicKey: String
    let osType: String?
    let pushToken: String?
    let agentID: String
    let pairToken: String
    let agentPairingPublicKey: String
    let encryptedHandshake: String
    let signature: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
        case deviceName = "device_name"
        case publicKey = "public_key"
        case osType = "os_type"
        case pushToken = "push_token"
        case agentID = "agent_id"
        case pairToken = "pair_token"
        case agentPairingPublicKey = "agent_pairing_public_key"
        case encryptedHandshake = "encrypted_handshake"
        case signature
    }
}

struct PairConfirmResponseData: Decodable {
    let bindingID: String
    let status: String
    let agentAck: AgentPairAckData

    enum CodingKeys: String, CodingKey {
        case bindingID = "binding_id"
        case status
        case agentAck = "agent_ack"
    }
}

struct AgentPairAckData: Decodable {
    let bindingID: String
    let deviceID: String
    let agentID: String
    let ackStatus: String

    enum CodingKeys: String, CodingKey {
        case bindingID = "binding_id"
        case deviceID = "device_id"
        case agentID = "agent_id"
        case ackStatus = "ack_status"
    }
}

struct PairRevokeRequestData: Encodable {
    let deviceID: String
    let agentID: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
        case agentID = "agent_id"
    }
}

struct PairRevokeResponseData: Decodable {
    let revoked: Bool
}

struct PushTokenUpdateRequestData: Encodable {
    let deviceID: String
    let pushToken: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
        case pushToken = "push_token"
    }
}

struct PushTokenUpdateResponseData: Decodable {
    let updated: Bool
}


struct TaskStartRequestData: Encodable {
    let deviceID: String
    let agentID: String
    let bindingID: String
    let agentEncryptionPublicKey: String
    let projectID: String
    let threadID: String?
    let prompt: String
    let contentParts: [ContentPart]?
    let model: String?
}

struct MetadataRefreshRequestData: Encodable {
    let requestID: String
    let deviceID: String

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case deviceID = "device_id"
    }
}

struct BranchChangesRequestData: Encodable {
    let requestID: String
    let deviceID: String
    let threadID: String
    let baseRef: String?

    enum CodingKeys: String, CodingKey {
        case requestID = "request_id"
        case deviceID = "device_id"
        case threadID = "thread_id"
        case baseRef = "base_ref"
    }
}

struct ResumeThreadRequestData: Encodable {
    let threadID: String
    let cursor: Int
    let checkpoint: String?

    enum CodingKeys: String, CodingKey {
        case threadID = "thread_id"
        case cursor
        case checkpoint
    }
}

struct ApprovalDecisionRequestData: Encodable {
    let deviceID: String
    let agentID: String
    let bindingID: String
    let agentEncryptionPublicKey: String
    let approvalID: String
    let decision: ApprovalDecision
}

struct UserInputResponseRequestData: Encodable {
    let deviceID: String
    let agentID: String
    let bindingID: String
    let agentEncryptionPublicKey: String
    let requestID: String
    let answers: [String: [String]]
}

struct TransferEnsureRequestData: Encodable {
    let sourceDeviceID: String
    let targetDeviceID: String
    let direction: TransferDirection
    let encryptedSizeBytes: Int

    enum CodingKeys: String, CodingKey {
        case sourceDeviceID = "source_device_id"
        case targetDeviceID = "target_device_id"
        case direction
        case encryptedSizeBytes = "encrypted_size_bytes"
    }
}

struct TransferEnsureResponseData: Decodable {
    let transferID: String
    let expiresAt: Int
    let needsUpload: Bool

    enum CodingKeys: String, CodingKey {
        case transferID = "transfer_id"
        case expiresAt = "expires_at"
        case needsUpload = "needs_upload"
    }
}

struct TransferUploadResponseData: Decodable {
    let uploaded: Bool
    let expiresAt: Int

    enum CodingKeys: String, CodingKey {
        case uploaded
        case expiresAt = "expires_at"
    }
}

struct TransferAckRequestData: Encodable {
    let receiverDeviceID: String

    enum CodingKeys: String, CodingKey {
        case receiverDeviceID = "receiver_device_id"
    }
}

struct TransferAckResponseData: Decodable {
    let acknowledged: Bool
}
