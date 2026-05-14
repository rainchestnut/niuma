import Foundation
import OSLog

extension AppModel {
    func authenticate(identity: LocalDeviceIdentity, agent: PairedAgent) async throws -> String {
        let controller = try requireController()
        // The current FastAPI server keeps session tokens in memory only, so any
        // server restart invalidates previously persisted mobile tokens. Always
        // re-run challenge/verify here to keep the control-plane reads and the
        // WebSocket handshake aligned with the live server process.
        let challenge = try await controller.issueChallenge(deviceID: identity.deviceID)
        let verifyPayload = try identityService.makeVerifyRequest(
            deviceID: identity.deviceID,
            challengeID: challenge.challengeID,
            challenge: challenge.challenge
        )
        let response = try await controller.verify(request: verifyPayload)
        controller.updateSessionToken(response.sessionToken)
        updateAgentToken(agentID: agent.agentID, sessionToken: response.sessionToken)
        return response.sessionToken
    }

    func ensurePairingIdentity() throws -> LocalDeviceIdentity {
        if let identity {
            return identity
        }
        let resolved = try identityService.ensureIdentity(deviceName: DeviceIdentityService.defaultDeviceName)
        self.identity = resolved
        return resolved
    }

    /// Returns the active server controller after the user has configured a Niuma Server address.
    func requireController() throws -> any NiumaControlling {
        guard let controller else {
            throw AppModelError.serverNotConfigured
        }
        return controller
    }

    /// Opens the authenticated WebSocket and continuously consumes server events for the selected agent.
    func connectRealtime(deviceID: String, agentID: String, sessionToken: String) async {
        realtimeTask?.cancel()
        realtimeTask = nil
        do {
            let controller = try requireController()
            guard let agent = pairedAgents.first(where: { $0.agentID == agentID }) else {
                throw AppModelError.invalidPairPayload
            }
            let stream = try await controller.connectRealtime(
                deviceID: deviceID,
                agent: agent,
                sessionToken: sessionToken
            )
            connectionState = .connected
            realtimeTask = Task {
                do {
                    for try await event in stream {
                        await handle(event)
                    }
                    if !Task.isCancelled {
                        await failActiveRefreshes(error: localized("realtime.error.closed_refresh_incomplete"))
                        connectionState = .degraded
                        updateAgentOnline(agentID: agentID, isOnline: false)
                    }
                } catch {
                    if !Task.isCancelled {
                        let isTransientDisconnect = isTransientRealtimeDisconnect(error)
                        await failActiveRefreshes(
                            error: localized("realtime.error.interrupted_refresh_incomplete"),
                            presentsAlert: !isTransientDisconnect
                        )
                        connectionState = .degraded
                        updateAgentOnline(agentID: agentID, isOnline: false)
                        if !isTransientDisconnect {
                            pendingError = error.localizedDescription
                        }
                    }
                }
            }
        } catch {
            connectionState = .degraded
            if !isTransientRealtimeDisconnect(error) {
                pendingError = error.localizedDescription
            }
        }
    }

    /// Bounds websocket sends that can otherwise hang after the server has reset
    /// a socket but URLSession has not surfaced the failure to the awaiting task.
    func withTimeout<T>(
        _ timeout: Duration,
        operation: @escaping () async throws -> T
    ) async throws -> T {
        try await withThrowingTaskGroup(of: T.self) { group in
            defer { group.cancelAll() }
            group.addTask {
                try await operation()
            }
            group.addTask { [self] in
                try await Task.sleep(for: timeout)
                await markRealtimeOperationTimedOut()
                throw AppModelError.realtimeOperationStalled
            }

            guard let result = try await group.next() else {
                throw AppModelError.realtimeOperationStalled
            }
            return result
        }
    }

    func markRealtimeOperationTimedOut() {
        controller?.disconnectRealtime()
        connectionState = .degraded
    }

    func ensureRealtimeConnected(
        deviceID: String,
        agentID: String,
        sessionToken: String,
        forceReconnect: Bool = false
    ) async {
        if !forceReconnect, connectionState == .connected, realtimeTask != nil {
            return
        }
        await connectRealtime(deviceID: deviceID, agentID: agentID, sessionToken: sessionToken)
    }

    /// Authenticates, ensures a realtime WebSocket, sends one request, and
    /// retries once when iOS is resuming from a suspended socket.
    func sendRealtimeRequestWithRecovery(
        identity: LocalDeviceIdentity,
        agent: PairedAgent,
        operationLabel: String,
        forceReconnect: Bool,
        afterAuthentication: ((_ controller: any NiumaControlling, _ sessionToken: String) async throws -> Void)? = nil,
        request: @escaping (_ controller: any NiumaControlling) async throws -> Void
    ) async throws {
        do {
            try await sendRealtimeRequest(
                identity: identity,
                agent: agent,
                operationLabel: operationLabel,
                forceReconnect: forceReconnect,
                afterAuthentication: afterAuthentication,
                request: request
            )
        } catch {
            guard isTransientRealtimeDisconnect(error) else {
                throw error
            }
            logger.info("realtime_request_transient_disconnect_retry operation=\(operationLabel, privacy: .public)")
            controller?.disconnectRealtime()
            connectionState = .retrying
            try await sendRealtimeRequest(
                identity: identity,
                agent: agent,
                operationLabel: operationLabel,
                forceReconnect: true,
                afterAuthentication: afterAuthentication,
                request: request
            )
        }
    }

    /// Performs one authenticated realtime send after the caller has chosen
    /// whether to reuse or rebuild the current WebSocket.
    func sendRealtimeRequest(
        identity: LocalDeviceIdentity,
        agent: PairedAgent,
        operationLabel: String,
        forceReconnect: Bool,
        afterAuthentication: ((_ controller: any NiumaControlling, _ sessionToken: String) async throws -> Void)?,
        request: @escaping (_ controller: any NiumaControlling) async throws -> Void
    ) async throws {
        let controller = try requireController()
        let sessionToken = try await authenticate(identity: identity, agent: agent)
        controller.updateSessionToken(sessionToken)
        try await afterAuthentication?(controller, sessionToken)
        await ensureRealtimeConnected(
            deviceID: identity.deviceID,
            agentID: agent.agentID,
            sessionToken: sessionToken,
            forceReconnect: forceReconnect
        )
        guard connectionState == .connected else {
            logger.error("realtime_request_not_connected operation=\(operationLabel, privacy: .public) connection=\(String(describing: self.connectionState), privacy: .public)")
            throw AppModelError.realtimeNotConnected
        }
        try await withTimeout(realtimeSendTimeout) {
            try await request(controller)
        }
    }

    /// Background suspension and simulator app switching should degrade the
    /// badge state without presenting a blocking error dialog to the user.
    func isTransientRealtimeDisconnect(_ error: Error) -> Bool {
        if let appError = error as? AppModelError {
            switch appError {
            case .realtimeOperationStalled:
                return true
            default:
                break
            }
        }
        let nsError = error as NSError
        if nsError.domain == NSURLErrorDomain {
            switch URLError.Code(rawValue: nsError.code) {
            case .networkConnectionLost, .notConnectedToInternet, .cancelled, .timedOut:
                return true
            default:
                break
            }
        }
        if nsError.domain == NSPOSIXErrorDomain {
            // Darwin POSIX socket errors commonly surfaced by URLSession after
            // iOS suspends a WebSocket while the device is locked.
            switch nsError.code {
            case 32, 53, 54, 57, 60:
                return true
            default:
                break
            }
        }
        let message = error.localizedDescription.lowercased()
        return message.contains("socket is not connected")
            || message.contains("connection lost")
            || message.contains("network connection was lost")
            || message.contains("software caused connection abort")
            || message.contains("connection abort")
            || message.contains("connection reset")
            || message.contains("broken pipe")
            || message.contains("cancelled")
            || message.contains("closed")
            || message.contains("\u{8F6F}\u{4EF6}\u{5BFC}\u{81F4}\u{8FDE}\u{63A5}\u{4E2D}\u{6B62}")
            || message.contains("\u{8FDE}\u{63A5}\u{4E2D}\u{6B62}")
            || message.contains("\u{8FDE}\u{63A5}\u{5DF2}\u{4E2D}\u{65AD}")
    }

    /// Downloads an encrypted server relay transfer once and stores it for local rendering.
    func receiveTransferIfNeeded(_ ready: TransferReady) async {
        guard ready.direction == .agentToIOS, let identity else { return }
        if let payload = localAttachments[ready.transferID],
           dataStore.localAttachmentData(payload) != nil {
            transferDownloadStates[ready.transferID] = .ready
            _ = try? await controller?.ackTransfer(
                transferID: ready.transferID,
                request: TransferAckRequestData(receiverDeviceID: identity.deviceID)
            )
            return
        }
        if transferDownloadStates[ready.transferID] == .downloading {
            return
        }
        transferDownloadStates[ready.transferID] = .downloading

        do {
            let controller = try requireController()
            let encryptedData = try await controller.downloadTransfer(
                transferID: ready.transferID,
                deviceID: identity.deviceID
            )
            guard sha256Hex(encryptedData) == ready.transferID else {
                throw AppModelError.transferChecksumMismatch
            }
            guard let sourceAgent = pairedAgents.first(where: { $0.agentID == ready.sourceDeviceID }) else {
                throw AppModelError.invalidPairPayload
            }
            let cryptoContext = try identityService.makePayloadCryptoContext(
                peerPublicKey: sourceAgent.agentEncryptionPublicKey,
                bindingID: sourceAgent.bindingID
            )
            let envelope = String(data: encryptedData, encoding: .utf8) ?? ""
            let data = try PayloadCryptoService.decrypt(
                envelope: envelope,
                context: cryptoContext,
                direction: .agentToIOS,
                additionalData: Self.transferAdditionalData(
                    direction: .agentToIOS,
                    sourceDeviceID: ready.sourceDeviceID,
                    targetDeviceID: ready.targetDeviceID
                )
            )
            let localRelativePath = try dataStore.saveLocalAttachmentFile(
                transferID: ready.transferID,
                fileName: nil,
                data: data
            )
            let payload = LocalAttachmentPayload(
                transferID: ready.transferID,
                direction: ready.direction,
                sourceDeviceID: ready.sourceDeviceID,
                targetDeviceID: ready.targetDeviceID,
                storedAt: .now,
                fileName: nil,
                mimeType: nil,
                sizeBytes: ready.encryptedSizeBytes,
                localRelativePath: localRelativePath
            )
            localAttachments[ready.transferID] = payload
            dataStore.upsertLocalAttachment(payload)
            transferDownloadStates[ready.transferID] = .ready
            _ = try await controller.ackTransfer(
                transferID: ready.transferID,
                request: TransferAckRequestData(receiverDeviceID: identity.deviceID)
            )
        } catch {
            transferDownloadStates[ready.transferID] = .failed(error.localizedDescription)
            pendingError = error.localizedDescription
        }
    }

    /// Routes one realtime event to local persistence or observed UI state.
    func handle(_ event: RealtimeEvent) async {
        switch event {
        case .taskUpdate(let update):
            logger.info("realtime_task_update thread_id=\(update.threadID, privacy: .public) seq=\(update.seq, privacy: .public) role=\(update.entry.role.rawValue, privacy: .public) type=\(update.entry.type, privacy: .public) phase=\((update.entry.phase ?? "nil"), privacy: .public) entry_id=\(update.entry.id, privacy: .public)")
            threadSyncPipeline.submit(.taskUpdate(update))

        case .transferReady(let ready):
            await receiveTransferIfNeeded(ready)

        case .metadataRefreshResult(let result):
            logger.info("metadata_refresh_result request_id=\(result.requestID, privacy: .public) succeeded=\(result.succeeded, privacy: .public)")
            if result.succeeded, let threadIDs = result.threadIDs, let agentID = selectedAgent?.agentID {
                reconcileThreadSnapshot(threadIDs: Set(threadIDs), agentID: agentID)
            } else if !result.succeeded {
                pendingError = result.error ?? "metadata refresh failed"
            }

        case .branchChangesResult(let result):
            logger.info("branch_changes_result request_id=\(result.requestID, privacy: .public) thread_id=\(result.threadID, privacy: .public) succeeded=\(result.succeeded, privacy: .public)")
            branchChangesByThread[result.threadID] = result
            if result.succeeded {
                Task { await ensureBranchChangeBundleDownloaded(result) }
            } else {
                pendingError = result.error ?? "branch changes failed"
            }

        case .threadArchiveResult(let result):
            logger.info("thread_archive_result request_id=\(result.requestID, privacy: .public) thread_id=\(result.threadID, privacy: .public) succeeded=\(result.succeeded, privacy: .public)")
            archivingThreadIDs.remove(result.threadID)
            if !result.succeeded {
                pendingError = result.error ?? "thread archive failed"
            }

        case .threadRenameResult(let result):
            logger.info("thread_rename_result request_id=\(result.requestID, privacy: .public) thread_id=\(result.threadID, privacy: .public) succeeded=\(result.succeeded, privacy: .public)")
            renamingThreadIDs.remove(result.threadID)
            if !result.succeeded {
                pendingError = result.error ?? "thread rename failed"
            }

        case .taskQueueSync(let sync):
            if sync.queuedCount > 0 {
                queuedTaskCountsByThread[sync.threadID] = sync.queuedCount
            } else {
                queuedTaskCountsByThread.removeValue(forKey: sync.threadID)
            }

        case .taskStartResult(let result):
            if result.succeeded {
                materializePendingNewTaskPrompt(result)
            } else {
                if let requestID = result.requestID {
                    pendingNewTaskPrompts.removeValue(forKey: requestID)
                }
                pendingError = result.error ?? "task start failed"
            }

        case .taskSteerResult(let result):
            if !result.succeeded {
                pendingError = result.error ?? "task steer failed"
            }

        case .taskInterruptResult(let result):
            if result.succeeded {
                runtimeState = .idle
            } else {
                pendingError = result.error ?? "task interrupt failed"
            }

        case .approvalRequest(let approval):
            runtimeState = .waitingApproval
            if let index = approvals.firstIndex(where: { $0.approvalID == approval.approvalID }) {
                approvals[index] = ApprovalSummary(
                    approvalID: approval.approvalID,
                    threadID: approval.threadID,
                    agentID: approval.agentID,
                    approvalType: approval.approvalType,
                    requestMethod: approval.requestMethod ?? approvals[index].requestMethod,
                    paramsJSON: approval.paramsJSON ?? approvals[index].paramsJSON,
                    status: approval.status,
                    updatedAt: approval.updatedAt
                )
            } else {
                approvals.insert(approval, at: 0)
            }
            if approval.status == .resolved || approval.requestMethod != nil {
                approvalResponseFailures[approval.approvalID] = nil
            }

        case .approvalResponseFailed(let failure):
            approvalResponseFailures[failure.approvalID] = failure.error
            pendingError = failure.error

        case .userInputRequest(let request):
            let currentStatus: ApprovalStatus
            if let index = userInputRequests.firstIndex(where: { $0.requestID == request.requestID }) {
                userInputRequests[index] = mergedUserInputRequest(
                    existing: userInputRequests[index],
                    incoming: request
                )
                currentStatus = userInputRequests[index].status
            } else {
                userInputRequests.insert(request, at: 0)
                currentStatus = request.status
            }
            if currentStatus != .resolved {
                runtimeState = .waitingApproval
            }
            if let current = currentUserInputRequest(request.requestID),
               current.status == .resolved || (current.status == .pending && !request.questions.isEmpty) {
                userInputResponseFailures[request.requestID] = nil
            }

        case .userInputResponseFailed(let failure):
            updateUserInputStatus(requestID: failure.requestID, status: .failed)
            userInputResponseFailures[failure.requestID] = failure.error
            pendingError = failure.error

        case .deviceStatus(let agentID, let online):
            guard let index = pairedAgents.firstIndex(where: { $0.agentID == agentID }) else { return }
            pairedAgents[index].isOnline = online
            persistPairedAgents()

        case .projectSync(let project):
            if let index = projects.firstIndex(where: { $0.projectID == project.projectID }) {
                projects[index] = project
            } else {
                projects.insert(project, at: 0)
            }
            dataStore.upsertProject(project)

        case .threadSync(let thread):
            if thread.status == .archived {
                deleteArchivedThread(thread)
                return
            }
            upsertThreadSummary(thread)

        case .threadSyncCompleted(let completion):
            // Completion is the authoritative end-of-batch signal. The app does
            // not infer completion from the absence of updates or from timers.
            logger.info("thread_sync_completed thread_id=\(completion.threadID, privacy: .public) cursor=\(completion.cursor, privacy: .public) entry_count=\(completion.entryCount, privacy: .public) checkpoint_present=\((completion.checkpoint != nil), privacy: .public)")
            threadSyncPipeline.submit(.completed(completion))

        case .threadSyncFailed(let failure):
            logger.error("thread_sync_failed thread_id=\(failure.threadID, privacy: .public) cursor=\(failure.cursor, privacy: .public) error=\(failure.error, privacy: .public)")
            threadSyncPipeline.submit(.failed(failure))

        case .modelSync(let state):
            applyModelSync(state)
        }
    }

    /// Keeps a mobile-local response from being moved back to pending by a
    /// duplicate app-server elicitation replay for the same request id.
    private func mergedUserInputRequest(
        existing: UserInputRequestSummary,
        incoming: UserInputRequestSummary
    ) -> UserInputRequestSummary {
        let regression = isUserInputStatusRegression(existing: existing.status, incoming: incoming.status)
        return UserInputRequestSummary(
            requestID: incoming.requestID,
            threadID: regression ? existing.threadID : incoming.threadID,
            agentID: regression ? existing.agentID : incoming.agentID,
            questions: incoming.questions.isEmpty ? existing.questions : incoming.questions,
            status: mergedUserInputStatus(existing: existing.status, incoming: incoming.status),
            updatedAt: regression ? existing.updatedAt : incoming.updatedAt
        )
    }

    private func mergedUserInputStatus(
        existing: ApprovalStatus,
        incoming: ApprovalStatus
    ) -> ApprovalStatus {
        if existing == .resolved || incoming == .resolved {
            return .resolved
        }
        if isUserInputStatusRegression(existing: existing, incoming: incoming) {
            return existing
        }
        return incoming
    }

    private func isUserInputStatusRegression(
        existing: ApprovalStatus,
        incoming: ApprovalStatus
    ) -> Bool {
        (existing == .submitting || existing == .failed || existing == .resolved) && incoming == .pending
    }

    /// Inserts an optimistic local user prompt that will be replaced by desktop replay.
    /// - Returns: Local thread and entry ids for rollback if WebSocket send fails.
}
