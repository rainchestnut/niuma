import Foundation
import OSLog

enum LiveRealtimeEventDecoder {
    nonisolated static func decode(
        from payload: String,
        agentID: String,
        cryptoContext: PayloadCryptoContext,
        decoder: JSONDecoder,
        logger: Logger
    ) throws -> RealtimeEvent {
        let data = Data(payload.utf8)
        let envelope = try decoder.decode(LiveRealtimeEnvelope.self, from: data)
        logger.debug("mobile_ws_receive kind=\(envelope.kind, privacy: .public)")
        switch envelope.kind {
        case "project_sync":
            let sync = try decoder.decode(LiveProjectSyncMessage.self, from: data)
            return .projectSync(
                ProjectSummary(
                    projectID: sync.projectID,
                    agentID: sync.agentID ?? agentID,
                    projectName: sync.projectName,
                    updatedAt: sync.updatedAt.map(Date.init(timeIntervalSince1970:)) ?? .now
                )
            )
        case "thread_sync":
            let sync = try decoder.decode(LiveThreadSyncMessage.self, from: data)
            return .threadSync(
                ThreadSummary(
                    threadID: sync.threadID,
                    projectID: sync.projectID ?? conversationProjectID,
                    agentID: sync.agentID ?? agentID,
                    title: sync.title,
                    status: sync.status,
                    lastCheckpointSeen: sync.lastCheckpointSeen,
                    currentBranch: sync.currentBranch,
                    updatedAt: sync.updatedAt.map(Date.init(timeIntervalSince1970:)) ?? .now
                )
            )
        case "thread_sync_completed":
            let completion = try decoder.decode(LiveThreadSyncCompletedMessage.self, from: data)
            logger.info("mobile_ws_thread_sync_completed_decode thread_id=\(completion.threadID, privacy: .public) cursor=\(completion.cursor, privacy: .public) entry_count=\(completion.entryCount, privacy: .public) checkpoint_present=\((completion.checkpoint != nil), privacy: .public)")
            // This is the explicit end marker for one replay batch. The mobile
            // app uses it to stop loading state and persist the authoritative
            // cursor; it does not infer completion from websocket quiet time.
            return .threadSyncCompleted(
                ThreadSyncCompletion(
                    threadID: completion.threadID,
                    cursor: completion.cursor,
                    checkpoint: completion.checkpoint,
                    entryCount: completion.entryCount
                )
            )
        case "thread_sync_failed":
            let failure = try decoder.decode(LiveThreadSyncFailedMessage.self, from: data)
            logger.error("mobile_ws_thread_sync_failed_decode thread_id=\(failure.threadID, privacy: .public) cursor=\(failure.cursor, privacy: .public) error=\(failure.error, privacy: .public)")
            return .threadSyncFailed(
                ThreadSyncFailure(
                    threadID: failure.threadID,
                    cursor: failure.cursor,
                    checkpoint: failure.checkpoint,
                    error: failure.error
                )
            )
        case "model_sync":
            let sync = try decoder.decode(LiveModelSyncMessage.self, from: data)
            return .modelSync(
                ModelSyncState(
                    currentModel: sync.currentModel,
                    availableModels: sync.availableModels
                )
            )
        case "task_update":
            let update = try decoder.decode(LiveTaskUpdateMessage.self, from: data)
            if update.seq <= 3 || update.seq.isMultiple(of: 100) {
                logger.info("mobile_ws_task_update_progress thread_id=\(update.threadID, privacy: .public) seq=\(update.seq, privacy: .public) role=\((update.role ?? "assistant"), privacy: .public) type=\(update.type, privacy: .public) phase=\((update.phase ?? "nil"), privacy: .public) entry_id=\((update.entryID ?? "nil"), privacy: .public)")
            }
            // Terminal status is signaled separately via thread_sync, not by any
            // per-update flag.
            let plaintext = try PayloadCryptoService.decrypt(
                envelope: update.ciphertext,
                context: cryptoContext,
                direction: .agentToIOS,
                additionalData: taskUpdateAdditionalData(update, agentID: agentID)
            )
            let content = decodeCiphertextContent(plaintext, decoder: decoder)
            let role = ThreadEntryRole(rawValue: update.role ?? "assistant") ?? .assistant
            return .taskUpdate(
                RealtimeTaskUpdate(
                    threadID: update.threadID,
                    seq: update.seq,
                    checkpoint: update.checkpoint,
                    entry: ThreadEntry(
                        id: update.entryID ?? "\(update.threadID)-\(update.seq)",
                        role: role,
                        type: update.type,
                        phase: update.phase,
                        text: content.text,
                        createdAt: update.createdAt.map(Date.init(timeIntervalSince1970:)),
                        seq: update.seq,
                        contentParts: content.parts
                    )
                )
            )
        case "transfer_ready":
            let ready = try decoder.decode(LiveTransferReadyMessage.self, from: data)
            return .transferReady(
                TransferReady(
                    transferID: ready.transferID,
                    direction: ready.direction,
                    sourceDeviceID: ready.sourceDeviceID,
                    targetDeviceID: ready.targetDeviceID,
                    encryptedSizeBytes: ready.encryptedSizeBytes,
                    expiresAt: ready.expiresAt
                )
            )
        case "metadata_refresh_completed":
            let result = try decoder.decode(LiveMetadataRefreshCompletedMessage.self, from: data)
            return .metadataRefreshResult(
                MetadataRefreshResult(
                    requestID: result.requestID,
                    succeeded: true,
                    error: nil
                )
            )
        case "metadata_refresh_failed":
            let result = try decoder.decode(LiveMetadataRefreshFailedMessage.self, from: data)
            return .metadataRefreshResult(
                MetadataRefreshResult(
                    requestID: result.requestID,
                    succeeded: false,
                    error: result.error
                )
            )
        case "branch_changes_result":
            let envelope = try decoder.decode(LiveBranchChangesEnvelopeMessage.self, from: data)
            let plaintext = try PayloadCryptoService.decrypt(
                envelope: envelope.ciphertext,
                context: cryptoContext,
                direction: .agentToIOS,
                additionalData: branchChangesAdditionalData(envelope, kind: "branch_changes_result", agentID: agentID)
            )
            let result = try decoder.decode(BranchChangesResultPlaintext.self, from: plaintext)
            return .branchChangesResult(
                BranchChangesResult(
                    requestID: envelope.requestID,
                    threadID: envelope.threadID,
                    succeeded: true,
                    summary: result.summary,
                    filesSummary: result.filesSummary,
                    transferID: result.transferID,
                    sizeBytes: result.sizeBytes,
                    error: nil
                )
            )
        case "branch_changes_failed":
            let envelope = try decoder.decode(LiveBranchChangesEnvelopeMessage.self, from: data)
            let plaintext = try PayloadCryptoService.decrypt(
                envelope: envelope.ciphertext,
                context: cryptoContext,
                direction: .agentToIOS,
                additionalData: branchChangesAdditionalData(envelope, kind: "branch_changes_failed", agentID: agentID)
            )
            let result = try decoder.decode(BranchChangesFailedPlaintext.self, from: plaintext)
            return .branchChangesResult(
                BranchChangesResult(
                    requestID: envelope.requestID,
                    threadID: envelope.threadID,
                    succeeded: false,
                    summary: nil,
                    filesSummary: [],
                    transferID: nil,
                    sizeBytes: nil,
                    error: result.error
                )
            )
        case "thread_archive_result":
            let result = try decoder.decode(LiveThreadArchiveResultMessage.self, from: data)
            return .threadArchiveResult(
                ThreadArchiveResult(
                    requestID: result.requestID,
                    threadID: result.threadID,
                    succeeded: true,
                    error: nil
                )
            )
        case "thread_archive_failed":
            let result = try decoder.decode(LiveThreadArchiveResultMessage.self, from: data)
            return .threadArchiveResult(
                ThreadArchiveResult(
                    requestID: result.requestID,
                    threadID: result.threadID,
                    succeeded: false,
                    error: result.error
                )
            )
        case "thread_rename_result":
            let result = try decoder.decode(LiveThreadRenameResultMessage.self, from: data)
            return .threadRenameResult(
                ThreadRenameResult(
                    requestID: result.requestID,
                    threadID: result.threadID,
                    succeeded: true,
                    error: nil
                )
            )
        case "thread_rename_failed":
            let result = try decoder.decode(LiveThreadRenameResultMessage.self, from: data)
            return .threadRenameResult(
                ThreadRenameResult(
                    requestID: result.requestID,
                    threadID: result.threadID,
                    succeeded: false,
                    error: result.error
                )
            )
        case "approval_request":
            let approval = try decoder.decode(LiveApprovalRequestMessage.self, from: data)
            let approvalPlaintext = try PayloadCryptoService.decrypt(
                envelope: approval.ciphertext,
                context: cryptoContext,
                direction: .agentToIOS,
                additionalData: approvalRequestAdditionalData(approval, agentID: agentID)
            )
            let details = try decodedApprovalRequestDetails(from: approvalPlaintext)
            return .approvalRequest(
                ApprovalSummary(
                    approvalID: approval.approvalID,
                    threadID: approval.threadID,
                    agentID: agentID,
                    approvalType: approval.approvalType,
                    requestMethod: details.requestMethod,
                    paramsJSON: details.paramsJSON,
                    status: .pending,
                    updatedAt: .now
                )
            )
        case "approval_sync":
            let approval = try decoder.decode(LiveApprovalSyncMessage.self, from: data)
            return .approvalRequest(
                ApprovalSummary(
                    approvalID: approval.approvalID,
                    threadID: approval.threadID,
                    agentID: agentID,
                    approvalType: approval.approvalType,
                    requestMethod: nil,
                    paramsJSON: nil,
                    status: approval.status == "pending" ? .pending : .resolved,
                    updatedAt: .now
                )
            )
        case "approval_response_failed":
            let failure = try decoder.decode(LiveApprovalResponseFailedMessage.self, from: data)
            return .approvalResponseFailed(
                ApprovalResponseFailure(
                    approvalID: failure.approvalID,
                    error: failure.error
                )
            )
        case "user_input_request":
            let request = try decoder.decode(LiveUserInputRequestMessage.self, from: data)
            let questions = try decodedUserInputQuestions(
                request,
                agentID: agentID,
                cryptoContext: cryptoContext,
                decoder: decoder
            )
            return .userInputRequest(
                UserInputRequestSummary(
                    requestID: request.requestID,
                    threadID: request.threadID,
                    agentID: agentID,
                    questions: questions.map {
                        UserInputQuestion(
                            questionID: $0.questionID,
                            header: $0.header,
                            prompt: $0.prompt,
                            options: $0.options.map {
                                UserInputQuestionOption(label: $0.label, description: $0.description)
                            },
                            isOther: $0.isOther,
                            isSecret: $0.isSecret
                        )
                    },
                    status: request.status == "pending" ? .pending : .resolved,
                    updatedAt: .now
                )
            )
        case "user_input_sync":
            let request = try decoder.decode(LiveUserInputSyncMessage.self, from: data)
            return .userInputRequest(
                UserInputRequestSummary(
                    requestID: request.requestID,
                    threadID: request.threadID,
                    agentID: agentID,
                    questions: [],
                    status: request.status == "pending" ? .pending : .resolved,
                    updatedAt: .now
                )
            )
        case "user_input_response_failed":
            let failure = try decoder.decode(LiveUserInputResponseFailedMessage.self, from: data)
            return .userInputResponseFailed(
                UserInputResponseFailure(
                    requestID: failure.requestID,
                    error: failure.error
                )
            )
        default:
            throw NSError(domain: "NiumaRealtimeError", code: 0, userInfo: [
                NSLocalizedDescriptionKey: "unsupported realtime event: \(envelope.kind)"
            ])
        }
    }

    nonisolated static func decodeCiphertextContent(
        _ data: Data,
        decoder: JSONDecoder
    ) -> DecodedCiphertextContent {
        if let payload = try? decoder.decode(ContentPartsPayload.self, from: data) {
            return DecodedCiphertextContent(
                text: flattenContentParts(payload.contentParts),
                parts: payload.contentParts
            )
        }
        return DecodedCiphertextContent(
            text: String(data: data, encoding: .utf8) ?? "",
            parts: nil
        )
    }

    nonisolated static func taskStartAdditionalData(_ request: TaskStartRequestData) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", "task_start"),
            ("device_id", request.deviceID),
            ("agent_id", request.agentID),
            ("project_id", request.projectID),
            ("thread_id", request.threadID)
        ])
    }

    nonisolated static func taskUpdateAdditionalData(
        _ update: LiveTaskUpdateMessage,
        agentID: String
    ) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", "task_update"),
            ("device_id", update.deviceID),
            ("agent_id", agentID),
            ("thread_id", update.threadID),
            ("seq", String(update.seq)),
            ("role", update.role),
            ("type", update.type),
            ("project_id", update.projectID),
            ("entry_id", update.entryID)
        ])
    }

    nonisolated static func mobileResponseAdditionalData(
        kind: String,
        deviceID: String,
        agentID: String?
    ) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", kind),
            ("device_id", deviceID),
            ("agent_id", agentID)
        ])
    }

    nonisolated static func taskProgressPushAdditionalData(
        deviceID: String,
        agentID: String
    ) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", "task_progress_push"),
            ("device_id", deviceID),
            ("agent_id", agentID)
        ])
    }

    nonisolated static func approvalRequestAdditionalData(
        _ request: LiveApprovalRequestMessage,
        agentID: String
    ) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", "approval_request"),
            ("device_id", request.deviceID),
            ("agent_id", agentID),
            ("approval_id", request.approvalID),
            ("thread_id", request.threadID),
            ("approval_type", request.approvalType)
        ])
    }

    nonisolated static func branchChangesAdditionalData(
        _ request: LiveBranchChangesEnvelopeMessage,
        kind: String,
        agentID: String
    ) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", kind),
            ("device_id", request.deviceID),
            ("agent_id", agentID),
            ("request_id", request.requestID),
            ("thread_id", request.threadID)
        ])
    }

    nonisolated static func userInputRequestAdditionalData(
        _ request: LiveUserInputRequestMessage,
        agentID: String
    ) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", "user_input_request"),
            ("device_id", request.deviceID),
            ("agent_id", agentID),
            ("request_id", request.requestID),
            ("thread_id", request.threadID),
            ("status", request.status)
        ])
    }

    nonisolated static func decodedUserInputQuestions(
        _ request: LiveUserInputRequestMessage,
        agentID: String,
        cryptoContext: PayloadCryptoContext,
        decoder: JSONDecoder
    ) throws -> [LiveUserInputQuestionMessage] {
        let plaintext = try PayloadCryptoService.decrypt(
            envelope: request.ciphertext,
            context: cryptoContext,
            direction: .agentToIOS,
            additionalData: userInputRequestAdditionalData(request, agentID: agentID)
        )
        return try decoder.decode(UserInputRequestPlaintext.self, from: plaintext).questions
    }

    nonisolated static func decodedApprovalRequestDetails(
        from plaintext: Data
    ) throws -> (requestMethod: String?, paramsJSON: String?) {
        guard let object = try JSONSerialization.jsonObject(with: plaintext) as? [String: Any] else {
            throw AppModelError.invalidPairPayload
        }
        return (
            requestMethod: object["method"] as? String,
            paramsJSON: try prettyJSONString(object["params"])
        )
    }

    nonisolated static func prettyJSONString(_ value: Any?) throws -> String? {
        guard let value else { return nil }
        guard JSONSerialization.isValidJSONObject(value) else {
            return String(describing: value)
        }
        let data = try JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys])
        return String(data: data, encoding: .utf8)
    }

    nonisolated static func flattenContentParts(_ parts: [ContentPart]) -> String {
        parts.map { part in
            switch part.kind {
            case .text:
                return part.text ?? ""
            case .fileRef:
                let name = part.fileName ?? part.alt ?? part.transferID ?? "attachment"
                if part.fileType == "image" || part.mimeType?.hasPrefix("image/") == true {
                    return String(format: String(localized: "content_part.image.preview", table: "Localizable"), name)
                }
                if part.fileType == "video" || part.mimeType?.hasPrefix("video/") == true {
                    return String(format: String(localized: "content_part.video.preview", table: "Localizable"), name)
                }
                return String(format: String(localized: "content_part.attachment.preview", table: "Localizable"), name)
            case .fileChangeSummary:
                return String(
                    format: String(localized: "content_part.file_changes.summary", table: "Localizable"),
                    part.files ?? 0,
                    part.additions ?? 0,
                    part.deletions ?? 0
                )
            }
        }
        .filter { !$0.isEmpty }
        .joined(separator: "\n")
    }
}
