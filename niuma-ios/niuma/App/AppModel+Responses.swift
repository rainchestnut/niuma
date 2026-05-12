import Foundation

extension AppModel {
    func resolveApproval(
        _ approval: ApprovalSummary,
        decision: ApprovalDecision,
        grantScope: ApprovalGrantScope? = nil
    ) async throws {
        guard let identity else { throw AppModelError.missingDeviceIdentity }
        guard let selectedAgent else { throw AppModelError.missingAgentBinding }
        guard currentApproval(approval.approvalID)?.status == .pending else {
            throw AppModelError.approvalNotPending
        }
        do {
            let controller = try requireController()
            _ = try await authenticate(identity: identity, agent: selectedAgent)
            approvalResponseFailures[approval.approvalID] = nil
            try await controller.respondToApproval(
                request: ApprovalDecisionRequestData(
                    deviceID: identity.deviceID,
                    agentID: selectedAgent.agentID,
                    bindingID: selectedAgent.bindingID,
                    agentEncryptionPublicKey: selectedAgent.agentEncryptionPublicKey,
                    approvalID: approval.approvalID,
                    decision: decision,
                    grantScope: grantScope
                )
            )
        } catch {
            pendingError = error.localizedDescription
            throw error
        }
    }

    func currentApproval(_ approvalID: String) -> ApprovalSummary? {
        approvals.first { $0.approvalID == approvalID }
    }

    func approvalFailureMessage(for approvalID: String) -> String? {
        approvalResponseFailures[approvalID]
    }

    func userInputFailureMessage(for requestID: String) -> String? {
        userInputResponseFailures[requestID]
    }

    func respondToUserInput(_ request: UserInputRequestSummary, answers: [String: [String]]) async throws {
        guard let identity else { throw AppModelError.missingDeviceIdentity }
        guard let selectedAgent else { throw AppModelError.missingAgentBinding }
        guard let currentRequest = currentUserInputRequest(request.requestID),
              currentRequest.status == .pending || currentRequest.status == .failed
        else {
            throw AppModelError.userInputNotPending
        }
        do {
            let controller = try requireController()
            _ = try await authenticate(identity: identity, agent: selectedAgent)
            userInputResponseFailures[request.requestID] = nil
            updateUserInputStatus(requestID: request.requestID, status: .submitting)
            try await controller.respondToUserInput(
                request: UserInputResponseRequestData(
                    deviceID: identity.deviceID,
                    agentID: selectedAgent.agentID,
                    bindingID: selectedAgent.bindingID,
                    agentEncryptionPublicKey: selectedAgent.agentEncryptionPublicKey,
                    requestID: request.requestID,
                    answers: answers
                )
            )
        } catch {
            updateUserInputStatus(requestID: request.requestID, status: .failed)
            userInputResponseFailures[request.requestID] = error.localizedDescription
            pendingError = error.localizedDescription
            throw error
        }
    }

    func currentUserInputRequest(_ requestID: String) -> UserInputRequestSummary? {
        userInputRequests.first { $0.requestID == requestID }
    }

    func updateUserInputStatus(requestID: String, status: ApprovalStatus) {
        guard let index = userInputRequests.firstIndex(where: { $0.requestID == requestID }) else {
            return
        }
        userInputRequests[index].status = status
    }

    func dismissError() {
        pendingError = nil
    }

    /// Restores the realtime channel after iOS brings the app back from a
    /// backgrounded state, where WebSocket keepalive can legitimately time out.
    func resumeAfterActivation() async {
        guard hasBootstrapped, let identity, let selectedAgent else { return }
        guard deviceState == .paired || deviceState == .pairFailed else { return }
        let sessionToken: String
        do {
            sessionToken = try await authenticate(identity: identity, agent: selectedAgent)
        } catch {
            if isTransientRealtimeDisconnect(error) {
                connectionState = .retrying
            } else {
                pendingError = error.localizedDescription
            }
            return
        }
        connectionState = .retrying
        await ensureRealtimeConnected(
            deviceID: identity.deviceID,
            agentID: selectedAgent.agentID,
            sessionToken: sessionToken,
            forceReconnect: true
        )
    }
}
