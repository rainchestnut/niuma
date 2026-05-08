import Foundation

extension AppModel {
    func startTask(projectID: String, threadID: String, prompt: String, attachments: [OutgoingAttachment] = []) async {
        do {
            try await performTaskStart(
                projectID: projectID,
                threadID: threadID,
                prompt: prompt,
                attachments: attachments
            )
        } catch {
            runtimeState = .failed
            pendingError = error.localizedDescription
        }
    }

    /// Reads a locally cached attachment by transfer id for SwiftUI rendering.
    /// - Parameters:
    ///   - transferID: SHA-256 transfer id used as the local cache key.
    /// - Returns: Attachment bytes when the local file mapping is present and readable.
    func localAttachmentData(forTransferID transferID: String) -> Data? {
        guard let payload = localAttachments[transferID] else {
            return nil
        }
        return dataStore.localAttachmentData(payload)
    }

    /// Sends the first prompt for a new desktop Codex task and lets Codex mint the thread id.
    func startNewTask(projectID: String, prompt: String) async throws {
        let normalizedPrompt = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !projectID.isEmpty else {
            throw AppModelError.missingProjectSelection
        }
        guard !normalizedPrompt.isEmpty else {
            throw AppModelError.missingPrompt
        }

        try await performTaskStart(
            projectID: projectID,
            threadID: nil,
            prompt: normalizedPrompt,
            attachments: []
        )
    }

    /// Starts a mobile-originated Codex task.
    func performTaskStart(
        projectID: String,
        threadID: String?,
        prompt: String,
        attachments: [OutgoingAttachment]
    ) async throws {
        guard !prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty else { return }
        guard let identity, let selectedAgent else { return }
        let controller = try requireController()
        let sessionToken = try await authenticate(identity: identity, agent: selectedAgent)
        let contentParts = try await buildContentParts(
            prompt: prompt,
            attachments: attachments,
            identity: identity,
            agent: selectedAgent,
            sessionToken: sessionToken
        )
        let transientThreadID: String?
        let transientEntryID: String?
        if let threadID {
            let inserted = insertTransientUserPrompt(
                threadID: threadID,
                projectID: projectID,
                agentID: selectedAgent.agentID,
                prompt: prompt,
                contentParts: contentParts
            )
            transientThreadID = inserted.threadID
            transientEntryID = inserted.entryID
        } else {
            transientThreadID = nil
            transientEntryID = nil
        }
        runtimeState = .submitting
        do {
            try await withTimeout(realtimeSendTimeout) { [self] in
                try await controller.sendTaskStart(
                    request: TaskStartRequestData(
                        deviceID: identity.deviceID,
                        agentID: selectedAgent.agentID,
                        bindingID: selectedAgent.bindingID,
                        agentEncryptionPublicKey: selectedAgent.agentEncryptionPublicKey,
                        projectID: projectID,
                        threadID: threadID,
                        prompt: prompt,
                        contentParts: contentParts,
                        model: self.selectedModelID
                    )
                )
            }
        } catch {
            if let transientThreadID, let transientEntryID {
                removeTransientUserPrompt(threadID: transientThreadID, entryID: transientEntryID)
            }
            throw error
        }
        runtimeState = .streaming
    }

    /// Builds the payload-blind content-parts array sent to desktop Codex.
    func buildContentParts(
        prompt: String,
        attachments: [OutgoingAttachment],
        identity: LocalDeviceIdentity,
        agent: PairedAgent,
        sessionToken: String
    ) async throws -> [ContentPart] {
        var parts: [ContentPart] = []
        let text = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        if !text.isEmpty {
            parts.append(ContentPart(kind: .text, text: text))
        }
        for attachment in attachments {
            parts.append(
                try await uploadAttachmentReference(
                    attachment,
                    identity: identity,
                    agent: agent,
                    sessionToken: sessionToken
                )
            )
        }
        return parts
    }

    /// Uploads an attachment as an encrypted transfer reference instead of inline Base64.
    func uploadAttachmentReference(
        _ attachment: OutgoingAttachment,
        identity: LocalDeviceIdentity,
        agent: PairedAgent,
        sessionToken: String
    ) async throws -> ContentPart {
        let controller = try requireController()
        controller.updateSessionToken(sessionToken)
        let cryptoContext = try identityService.makePayloadCryptoContext(
            peerPublicKey: agent.agentEncryptionPublicKey,
            bindingID: agent.bindingID
        )
        let encryptedEnvelope = try PayloadCryptoService.encrypt(
            plaintext: attachment.data,
            context: cryptoContext,
            direction: .iosToAgent,
            additionalData: Self.transferAdditionalData(
                direction: .iosToAgent,
                sourceDeviceID: identity.deviceID,
                targetDeviceID: agent.agentID
            )
        )
        guard let encryptedBody = encryptedEnvelope.data(using: .utf8) else {
            throw AppModelError.invalidPairPayload
        }
        let transferID = sha256Hex(encryptedBody)
        let ensured = try await controller.ensureTransfer(
            transferID: transferID,
            request: TransferEnsureRequestData(
                sourceDeviceID: identity.deviceID,
                targetDeviceID: agent.agentID,
                direction: .iosToAgent,
                encryptedSizeBytes: encryptedBody.count
            )
        )
        guard ensured.transferID == transferID else {
            throw AppModelError.transferIdentifierMismatch
        }
        if ensured.needsUpload {
            _ = try await controller.uploadTransfer(
                transferID: transferID,
                body: encryptedBody,
                deviceID: identity.deviceID
            )
        }
        let localRelativePath = try dataStore.saveLocalAttachmentFile(
            transferID: transferID,
            fileName: attachment.fileName,
            data: attachment.data
        )
        let part = ContentPart(
            kind: .fileRef,
            transferID: transferID,
            fileType: attachment.fileType,
            fileName: attachment.fileName,
            mimeType: attachment.mimeType,
            sizeBytes: attachment.data.count,
            alt: attachment.alt ?? attachment.fileName
        )
        let payload = LocalAttachmentPayload(
            transferID: transferID,
            direction: .iosToAgent,
            sourceDeviceID: identity.deviceID,
            targetDeviceID: agent.agentID,
            storedAt: .now,
            fileName: attachment.fileName,
            mimeType: attachment.mimeType,
            sizeBytes: attachment.data.count,
            localRelativePath: localRelativePath
        )
        localAttachments[transferID] = payload
        dataStore.upsertLocalAttachment(payload)
        return part
    }

    /// Starts the single session-detail refresh flow used by initial entry and manual refresh.
}
