import CryptoKit
import Foundation

nonisolated enum PayloadCryptoDirection: String, Sendable {
    case iosToAgent = "ios_to_agent"
    case agentToIOS = "agent_to_ios"
}

nonisolated struct PayloadCryptoContext: Sendable {
    let privateKeyData: Data
    let peerPublicKey: String
    let bindingID: String
}

/// Encrypts and decrypts pair-scoped business payloads shared by iOS and the Rust gateway.
nonisolated enum PayloadCryptoService {
    private static let version = 1
    private static let algorithm = "X25519-HKDF-SHA256-CHACHA20POLY1305"

    static func additionalData(_ fields: [(String, String?)]) -> Data {
        fields
            .map { key, value in "\(key)=\(value ?? "")" }
            .joined(separator: "\n")
            .data(using: .utf8) ?? Data()
    }

    static func encrypt(
        plaintext: Data,
        context: PayloadCryptoContext,
        direction: PayloadCryptoDirection,
        additionalData: Data
    ) throws -> String {
        let key = try symmetricKey(context: context, direction: direction)
        let nonce = ChaChaPoly.Nonce()
        let sealed = try ChaChaPoly.seal(
            plaintext,
            using: key,
            nonce: nonce,
            authenticating: additionalData
        )
        let nonceData = nonce.withUnsafeBytes { Data($0) }
        let ciphertext = sealed.ciphertext + sealed.tag
        let envelope = PayloadCryptoEnvelope(
            version: version,
            alg: algorithm,
            kid: context.bindingID,
            nonce: nonceData.base64EncodedString(),
            ciphertext: ciphertext.base64EncodedString()
        )
        let data = try JSONEncoder().encode(envelope)
        guard let encoded = String(data: data, encoding: .utf8) else {
            throw AppModelError.invalidPairPayload
        }
        return encoded
    }

    static func decrypt(
        envelope encodedEnvelope: String,
        context: PayloadCryptoContext,
        direction: PayloadCryptoDirection,
        additionalData: Data
    ) throws -> Data {
        let envelope = try JSONDecoder().decode(
            PayloadCryptoEnvelope.self,
            from: Data(encodedEnvelope.utf8)
        )
        guard envelope.version == version,
              envelope.alg == algorithm,
              envelope.kid == context.bindingID,
              let nonceData = Data(base64Encoded: envelope.nonce),
              let combined = Data(base64Encoded: envelope.ciphertext)
        else {
            throw AppModelError.invalidPairPayload
        }
        let nonce = try ChaChaPoly.Nonce(data: nonceData)
        guard combined.count >= 16 else {
            throw AppModelError.invalidPairPayload
        }
        let ciphertext = combined.dropLast(16)
        let tag = combined.suffix(16)
        let sealed = try ChaChaPoly.SealedBox(
            nonce: nonce,
            ciphertext: ciphertext,
            tag: tag
        )
        return try ChaChaPoly.open(
            sealed,
            using: symmetricKey(context: context, direction: direction),
            authenticating: additionalData
        )
    }

    private static func symmetricKey(
        context: PayloadCryptoContext,
        direction: PayloadCryptoDirection
    ) throws -> SymmetricKey {
        let privateKey = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: context.privateKeyData)
        let peerPublicKey = try Curve25519.KeyAgreement.PublicKey(
            rawRepresentation: decodePrefixed(context.peerPublicKey, prefix: "x25519:")
        )
        let sharedSecret = try privateKey.sharedSecretFromKeyAgreement(with: peerPublicKey)
        return sharedSecret.hkdfDerivedSymmetricKey(
            using: SHA256.self,
            salt: Data(context.bindingID.utf8),
            sharedInfo: Data("niuma e2ee v1:\(direction.rawValue)".utf8),
            outputByteCount: 32
        )
    }

    private static func decodePrefixed(_ value: String, prefix: String) throws -> Data {
        let encoded = value.hasPrefix(prefix) ? String(value.dropFirst(prefix.count)) : value
        guard let data = Data(base64Encoded: encoded) else {
            throw AppModelError.invalidPairPayload
        }
        return data
    }
}

private nonisolated struct PayloadCryptoEnvelope: Codable {
    let version: Int
    let alg: String
    let kid: String
    let nonce: String
    let ciphertext: String
}
