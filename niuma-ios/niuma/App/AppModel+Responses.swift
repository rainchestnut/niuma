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
            approvalResponseFailures[approval.approvalID] = nil
            try await sendRealtimeRequestWithRecovery(
                identity: identity,
                agent: selectedAgent,
                operationLabel: "approval_response",
                forceReconnect: connectionState != .connected
            ) { controller in
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
            }
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
            userInputResponseFailures[request.requestID] = nil
            updateUserInputStatus(requestID: request.requestID, status: .submitting)
            try await sendRealtimeRequestWithRecovery(
                identity: identity,
                agent: selectedAgent,
                operationLabel: "user_input_response",
                forceReconnect: connectionState != .connected
            ) { controller in
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
            }
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
        guard hasBootstrapped, identity != nil, selectedAgent != nil else { return }
        guard deviceState == .paired || deviceState == .pairFailed else { return }
        shouldMaintainRealtimeConnection = true
        if connectionState == .connected, realtimeTask != nil {
            return
        }
        connectionState = .retrying
        startRealtimeReconnect(reason: "activation")
    }
}
