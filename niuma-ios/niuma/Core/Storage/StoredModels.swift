import Foundation
import SwiftData

/// SwiftData record for a desktop agent paired with the current iOS device.
@Model
final class StoredAgent {
    @Attribute(.unique) var agentID: String
    var bindingID: String
    var agentEncryptionPublicKey: String
    var displayName: String
    var fingerprint: String
    var pairedAt: Date
    var sessionToken: String?
    var isOnline: Bool
    var isSelected: Bool

    init(agent: PairedAgent, isSelected: Bool) {
        self.agentID = agent.agentID
        self.bindingID = agent.bindingID
        self.agentEncryptionPublicKey = agent.agentEncryptionPublicKey
        self.displayName = agent.displayName
        self.fingerprint = agent.fingerprint
        self.pairedAt = agent.pairedAt
        self.sessionToken = agent.sessionToken
        self.isOnline = agent.isOnline
        self.isSelected = isSelected
    }

    /// Updates stored desktop-agent metadata and the selected-agent flag.
    func update(agent: PairedAgent, isSelected: Bool) {
        bindingID = agent.bindingID
        agentEncryptionPublicKey = agent.agentEncryptionPublicKey
        displayName = agent.displayName
        fingerprint = agent.fingerprint
        pairedAt = agent.pairedAt
        sessionToken = agent.sessionToken
        isOnline = agent.isOnline
        self.isSelected = isSelected
    }

    /// Converts the stored desktop-agent row back into the domain payload.
    func toDomain() -> PairedAgent {
        PairedAgent(
            bindingID: bindingID,
            agentID: agentID,
            agentEncryptionPublicKey: agentEncryptionPublicKey,
            displayName: displayName,
            fingerprint: fingerprint,
            pairedAt: pairedAt,
            sessionToken: sessionToken,
            isOnline: isOnline
        )
    }
}

/// SwiftData record for Codex project metadata streamed from the desktop gateway.
@Model
final class StoredProject {
    @Attribute(.unique) var projectID: String
    var agentID: String
    var projectName: String
    var updatedAt: Date

    init(project: ProjectSummary) {
        self.projectID = project.projectID
        self.agentID = project.agentID
        self.projectName = project.projectName
        self.updatedAt = project.updatedAt
    }

    /// Updates the stored project metadata from a fresh server summary.
    func update(project: ProjectSummary) {
        agentID = project.agentID
        projectName = project.projectName
        updatedAt = project.updatedAt
    }

    /// Converts stored or decoded data into the domain model.
    func toDomain() -> ProjectSummary {
        ProjectSummary(projectID: projectID, agentID: agentID, projectName: projectName, updatedAt: updatedAt)
    }
}

/// SwiftData record for a Codex thread, its list metadata, and the mobile replay cursor.
@Model
final class StoredThread {
    @Attribute(.unique) var threadID: String
    var projectID: String
    var agentID: String
    var title: String
    var statusRawValue: String
    var lastCheckpointSeen: String?
    var cursor: Int
    var checkpoint: String?
    var updatedAt: Date

    init(thread: ThreadSummary, cursor: Int = 0, checkpoint: String? = nil) {
        self.threadID = thread.threadID
        self.projectID = thread.projectID
        self.agentID = thread.agentID
        self.title = thread.title
        self.statusRawValue = thread.status.rawValue
        self.lastCheckpointSeen = thread.lastCheckpointSeen
        self.cursor = cursor
        self.checkpoint = checkpoint
        self.updatedAt = thread.updatedAt
    }

    /// Updates the stored thread metadata while preserving the highest known cursor.
    func update(thread: ThreadSummary) {
        projectID = thread.projectID
        agentID = thread.agentID
        title = thread.title
        statusRawValue = thread.status.rawValue
        lastCheckpointSeen = thread.lastCheckpointSeen
        updatedAt = thread.updatedAt
    }

    /// Converts stored or decoded data into the domain model.
    func toDomain() -> ThreadSummary {
        ThreadSummary(
            threadID: threadID,
            projectID: projectID,
            agentID: agentID,
            title: title,
            status: ThreadStatus(rawValue: statusRawValue) ?? .unknown,
            lastCheckpointSeen: lastCheckpointSeen ?? checkpoint,
            updatedAt: updatedAt
        )
    }
}

/// SwiftData record for one Codex-assigned message entry.
@Model
final class StoredThreadEntry {
    @Attribute(.unique) var storageID: String
    var threadID: String
    var entryID: String
    var roleRawValue: String
    var type: String
    var phase: String?
    var text: String
    var createdAt: Date
    var seq: Int
    var contentPartsData: Data?

    init(threadID: String, entry: ThreadEntry, contentPartsData: Data?) {
        self.storageID = Self.storageID(threadID: threadID, entryID: entry.id)
        self.threadID = threadID
        self.entryID = entry.id
        self.roleRawValue = entry.role.rawValue
        self.type = entry.type
        self.phase = entry.phase
        self.text = entry.text
        self.createdAt = entry.createdAt
        self.seq = entry.seq
        self.contentPartsData = contentPartsData
    }

    /// Builds the unique SwiftData key for one thread entry.
    static func storageID(threadID: String, entryID: String) -> String {
        "\(threadID)::\(entryID)"
    }

    /// Updates the stored message entry and serialized content-parts payload.
    func update(entry: ThreadEntry, contentPartsData: Data?) {
        roleRawValue = entry.role.rawValue
        type = entry.type
        phase = entry.phase
        text = entry.text
        createdAt = entry.createdAt
        seq = entry.seq
        self.contentPartsData = contentPartsData
    }
}

/// SwiftData record mapping a transfer id to this device's local attachment file.
@Model
final class StoredLocalAttachment {
    @Attribute(.unique) var transferID: String
    var directionRawValue: String
    var sourceDeviceID: String
    var targetDeviceID: String
    var storedAt: Date
    var fileName: String?
    var mimeType: String?
    var sizeBytes: Int?
    var localRelativePath: String

    init(payload: LocalAttachmentPayload) {
        self.transferID = payload.transferID
        self.directionRawValue = payload.direction.rawValue
        self.sourceDeviceID = payload.sourceDeviceID
        self.targetDeviceID = payload.targetDeviceID
        self.storedAt = payload.storedAt
        self.fileName = payload.fileName
        self.mimeType = payload.mimeType
        self.sizeBytes = payload.sizeBytes
        self.localRelativePath = payload.localRelativePath
    }

    /// Updates the stored attachment file mapping and metadata after send or receive.
    func update(payload: LocalAttachmentPayload) {
        directionRawValue = payload.direction.rawValue
        sourceDeviceID = payload.sourceDeviceID
        targetDeviceID = payload.targetDeviceID
        storedAt = payload.storedAt
        fileName = payload.fileName
        mimeType = payload.mimeType
        sizeBytes = payload.sizeBytes
        localRelativePath = payload.localRelativePath
    }

    /// Converts stored or decoded data into the domain model.
    func toDomain() -> LocalAttachmentPayload {
        LocalAttachmentPayload(
            transferID: transferID,
            direction: TransferDirection(rawValue: directionRawValue) ?? .agentToIOS,
            sourceDeviceID: sourceDeviceID,
            targetDeviceID: targetDeviceID,
            storedAt: storedAt,
            fileName: fileName,
            mimeType: mimeType,
            sizeBytes: sizeBytes,
            localRelativePath: localRelativePath
        )
    }
}
