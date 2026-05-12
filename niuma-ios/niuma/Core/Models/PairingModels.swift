import CryptoKit
import Foundation

nonisolated struct PairCodePayload: Codable, Hashable {
    let agentID: String
    let pairToken: String
    let agentName: String
    let expiresAt: Int
    let agentSigningPublicKey: String
    let agentEncryptionPublicKey: String
    let agentPairingPublicKey: String
    let agentSigningKeyFingerprint: String
    let agentEncryptionKeyFingerprint: String
    let signature: String

    var fingerprint: String { agentEncryptionKeyFingerprint }

    // JSON keys here MUST match the desktop gateway QR payload.
    enum CodingKeys: String, CodingKey {
        case agentID = "agent_id"
        case pairToken = "pair_token"
        case agentName = "agent_name"
        case expiresAt = "expires_at"
        case agentSigningPublicKey = "agent_signing_public_key"
        case agentEncryptionPublicKey = "agent_encryption_public_key"
        case agentPairingPublicKey = "agent_pairing_public_key"
        case agentSigningKeyFingerprint = "agent_signing_key_fingerprint"
        case agentEncryptionKeyFingerprint = "agent_encryption_key_fingerprint"
        case signature
    }
}

/// Decodes a raw QR-string into a `PairCodePayload`. Used by both the camera
/// scanner callback and the manual pairing-info entry point.
nonisolated enum PairCodePayloadDecoder {
    /// Decodes a scanned QR string into a typed pair-code payload.
    static func decode(_ raw: String) throws -> PairCodePayload {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let data = trimmed.data(using: .utf8) else {
            throw AppModelError.invalidPairPayload
        }
        let payload: PairCodePayload
        do {
            payload = try JSONDecoder().decode(PairCodePayload.self, from: data)
        } catch {
            throw AppModelError.invalidPairPayload
        }
        guard !payload.agentID.isEmpty,
              !payload.pairToken.isEmpty,
              !payload.agentName.isEmpty,
              !payload.agentSigningPublicKey.isEmpty,
              !payload.agentEncryptionPublicKey.isEmpty,
              !payload.agentPairingPublicKey.isEmpty,
              !payload.agentSigningKeyFingerprint.isEmpty,
              !payload.agentEncryptionKeyFingerprint.isEmpty,
              !payload.signature.isEmpty,
              payload.expiresAt > Int(Date.now.timeIntervalSince1970),
              verifySignature(payload)
        else {
            throw AppModelError.invalidPairPayload
        }
        return payload
    }

    /// Verifies that the QR payload was signed by the included desktop signing key.
    private static func verifySignature(_ payload: PairCodePayload) -> Bool {
        do {
            let publicKey = try Curve25519.Signing.PublicKey(
                rawRepresentation: decodePrefixed(payload.agentSigningPublicKey, prefix: "ed25519:")
            )
            let signature = try decodePrefixed(payload.signature, prefix: "ed25519:")
            let digest = sha256Hex(
                "\(payload.agentID):\(payload.agentName):\(payload.pairToken):\(payload.expiresAt):\(payload.agentSigningPublicKey):\(payload.agentEncryptionPublicKey):\(payload.agentPairingPublicKey)"
            )
            return publicKey.isValidSignature(signature, for: Data(digest.utf8))
        } catch {
            return false
        }
    }

    private static func decodePrefixed(_ value: String, prefix: String) throws -> Data {
        let encoded = value.hasPrefix(prefix) ? String(value.dropFirst(prefix.count)) : value
        guard let data = Data(base64Encoded: encoded) else {
            throw AppModelError.invalidPairPayload
        }
        return data
    }

    private static func sha256Hex(_ value: String) -> String {
        SHA256.hash(data: Data(value.utf8)).map { String(format: "%02x", $0) }.joined()
    }
}
