import CryptoKit
import Foundation
import UIKit

struct DeviceIdentityService {
    static let defaultDeviceName = UIDevice.current.name

    private let storage: KeyValueStore
    private let secretStore: SecretStore
    private let deviceIDKey = "device.identity.id"
    private let publicKeyKey = "device.identity.publicKey"
    private let privateKeyKey = "device.identity.privateKey"
    private let encryptionPublicKeyKey = "device.identity.encryptionPublicKey"
    private let encryptionPrivateKeyKey = "device.identity.encryptionPrivateKey"
    private let displayNameKey = "device.identity.displayName"

    init(storage: KeyValueStore, secretStore: SecretStore) {
        self.storage = storage
        self.secretStore = secretStore
    }

    /// Ensures Identity.
    func ensureIdentity(deviceName: String) throws -> LocalDeviceIdentity {
        if let deviceID = storage.string(forKey: deviceIDKey),
           let publicKey = storage.string(forKey: publicKeyKey),
           let encryptionPublicKey = storage.string(forKey: encryptionPublicKeyKey),
           let displayName = storage.string(forKey: displayNameKey),
           secretStore.data(forKey: privateKeyKey) != nil,
           secretStore.data(forKey: encryptionPrivateKeyKey) != nil {
            return LocalDeviceIdentity(
                deviceID: deviceID,
                publicKey: publicKey,
                encryptionPublicKey: encryptionPublicKey,
                displayName: displayName
            )
        }

        let signingPrivateKey = Curve25519.Signing.PrivateKey()
        let publicKey = "ed25519:\(signingPrivateKey.publicKey.rawRepresentation.base64EncodedString())"
        let encryptionPrivateKey = Curve25519.KeyAgreement.PrivateKey()
        let encryptionPublicKey = "x25519:\(encryptionPrivateKey.publicKey.rawRepresentation.base64EncodedString())"
        let deviceID = "ios-\(UUID().uuidString.lowercased())"

        try secretStore.set(signingPrivateKey.rawRepresentation, forKey: privateKeyKey)
        try secretStore.set(encryptionPrivateKey.rawRepresentation, forKey: encryptionPrivateKeyKey)
        storage.set(deviceID, forKey: deviceIDKey)
        storage.set(publicKey, forKey: publicKeyKey)
        storage.set(encryptionPublicKey, forKey: encryptionPublicKeyKey)
        storage.set(deviceName, forKey: displayNameKey)

        return LocalDeviceIdentity(
            deviceID: deviceID,
            publicKey: publicKey,
            encryptionPublicKey: encryptionPublicKey,
            displayName: deviceName
        )
    }

    /// Deletes the local device identity and its private keys so the next pair creates a fresh device.
    func resetIdentity() throws {
        storage.set(nil, forKey: deviceIDKey)
        storage.set(nil, forKey: publicKeyKey)
        storage.set(nil, forKey: encryptionPublicKeyKey)
        storage.set(nil, forKey: displayNameKey)
        try secretStore.removeAll()
    }

    func makeVerifyRequest(deviceID: String, challengeID: String, challenge: String) throws -> VerifyRequestData {
        let timestamp = Int(Date.now.timeIntervalSince1970)
        let nonce = UUID().uuidString.lowercased()
        let digest = Self.authDigest(
            deviceID: deviceID,
            challengeID: challengeID,
            challenge: challenge,
            timestamp: timestamp,
            nonce: nonce
        )
        let signature = try sign(message: digest)
        return VerifyRequestData(
            deviceID: deviceID,
            challengeID: challengeID,
            timestamp: timestamp,
            nonce: nonce,
            requestDigest: digest,
            signature: signature
        )
    }

    func makePairConfirmRequest(
        identity: LocalDeviceIdentity,
        payload: PairCodePayload,
        pushToken: String?
    ) throws -> PairConfirmRequestData {
        let encryptedHandshake = try makeEncryptedPairHandshake(identity: identity, payload: payload)
        let signature = try sign(
            message: Self.pairDigest(
                deviceID: identity.deviceID,
                agentID: payload.agentID,
                pairToken: payload.pairToken,
                agentPairingPublicKey: payload.agentPairingPublicKey,
                encryptedHandshake: encryptedHandshake
            )
        )
        return PairConfirmRequestData(
            deviceID: identity.deviceID,
            deviceName: identity.displayName,
            publicKey: identity.publicKey,
            osType: "iOS",
            pushToken: pushToken,
            agentID: payload.agentID,
            pairToken: payload.pairToken,
            agentPairingPublicKey: payload.agentPairingPublicKey,
            encryptedHandshake: encryptedHandshake,
            signature: signature
        )
    }

    func makeTaskStartSignature(
        deviceID: String,
        agentID: String,
        projectID: String,
        threadID: String?,
        ciphertext: String
    ) throws -> String {
        let digest = Self.taskStartDigest(
            deviceID: deviceID,
            agentID: agentID,
            projectID: projectID,
            threadID: threadID,
            ciphertext: ciphertext
        )
        return try sign(message: digest)
    }

    /// Builds the pair-scoped crypto context used for business payload encryption.
    func makePayloadCryptoContext(peerPublicKey: String, bindingID: String) throws -> PayloadCryptoContext {
        guard let rawPrivateKey = secretStore.data(forKey: encryptionPrivateKeyKey) else {
            throw AppModelError.missingPrivateKey
        }
        return PayloadCryptoContext(
            privateKeyData: rawPrivateKey,
            peerPublicKey: peerPublicKey,
            bindingID: bindingID
        )
    }

    /// Signs a digest string with the locally stored private key.
    private func sign(message: String) throws -> String {
        guard let rawPrivateKey = secretStore.data(forKey: privateKeyKey) else {
            throw AppModelError.missingPrivateKey
        }
        let privateKey = try Curve25519.Signing.PrivateKey(rawRepresentation: rawPrivateKey)
        let signature = try privateKey.signature(for: Data(message.utf8))
        return "ed25519:\(signature.base64EncodedString())"
    }

    private static func authDigest(
        deviceID: String,
        challengeID: String,
        challenge: String,
        timestamp: Int,
        nonce: String
    ) -> String {
        sha256Hex("\(deviceID):\(challengeID):\(challenge):\(timestamp):\(nonce)")
    }

    private static func pairDigest(
        deviceID: String,
        agentID: String,
        pairToken: String,
        agentPairingPublicKey: String,
        encryptedHandshake: String
    ) -> String {
        sha256Hex("\(deviceID):\(agentID):\(pairToken):\(agentPairingPublicKey):\(encryptedHandshake)")
    }

    private static func taskStartDigest(
        deviceID: String,
        agentID: String,
        projectID: String,
        threadID: String?,
        ciphertext: String
    ) -> String {
        sha256Hex("\(deviceID):\(agentID):\(projectID):\(threadID ?? ""):\(ciphertext)")
    }

    private static func sha256Hex(_ value: String) -> String {
        let digest = SHA256.hash(data: Data(value.utf8))
        return digest.map { String(format: "%02x", $0) }.joined()
    }

    private func makeEncryptedPairHandshake(identity: LocalDeviceIdentity, payload: PairCodePayload) throws -> String {
        guard let rawPrivateKey = secretStore.data(forKey: encryptionPrivateKeyKey) else {
            throw AppModelError.missingPrivateKey
        }
        let privateKey = try Curve25519.KeyAgreement.PrivateKey(rawRepresentation: rawPrivateKey)
        let agentPairingKey = try Curve25519.KeyAgreement.PublicKey(
            rawRepresentation: Self.decodePrefixed(payload.agentPairingPublicKey, prefix: "x25519:")
        )
        let sharedSecret = try privateKey.sharedSecretFromKeyAgreement(with: agentPairingKey)
        let symmetricKey = sharedSecret.hkdfDerivedSymmetricKey(
            using: SHA256.self,
            salt: Data(payload.pairToken.utf8),
            sharedInfo: Data("niuma pairing handshake".utf8),
            outputByteCount: 32
        )
        let plaintext = try JSONEncoder().encode(
            PairHandshakePlaintext(
                deviceID: identity.deviceID,
                iosEncryptionPublicKey: identity.encryptionPublicKey
            )
        )
        let nonce = ChaChaPoly.Nonce()
        let sealed = try ChaChaPoly.seal(plaintext, using: symmetricKey, nonce: nonce)
        let nonceData = nonce.withUnsafeBytes { Data($0) }
        let ciphertext = sealed.ciphertext + sealed.tag
        return try String(
            data: JSONEncoder().encode(
                PairHandshakeEnvelope(
                    iosEncryptionPublicKey: identity.encryptionPublicKey,
                    nonce: nonceData.base64EncodedString(),
                    ciphertext: ciphertext.base64EncodedString()
                )
            ),
            encoding: .utf8
        ).orThrow(AppModelError.invalidPairPayload)
    }

    private static func decodePrefixed(_ value: String, prefix: String) throws -> Data {
        let encoded = value.hasPrefix(prefix) ? String(value.dropFirst(prefix.count)) : value
        guard let data = Data(base64Encoded: encoded) else {
            throw AppModelError.invalidPairPayload
        }
        return data
    }
}

private struct PairHandshakePlaintext: Encodable {
    let deviceID: String
    let iosEncryptionPublicKey: String

    enum CodingKeys: String, CodingKey {
        case deviceID = "device_id"
        case iosEncryptionPublicKey = "ios_encryption_public_key"
    }
}

private struct PairHandshakeEnvelope: Encodable {
    let iosEncryptionPublicKey: String
    let nonce: String
    let ciphertext: String

    enum CodingKeys: String, CodingKey {
        case iosEncryptionPublicKey = "ios_encryption_public_key"
        case nonce
        case ciphertext
    }
}

private extension Optional where Wrapped == String {
    func orThrow(_ error: Error) throws -> String {
        guard let value = self else {
            throw error
        }
        return value
    }
}
