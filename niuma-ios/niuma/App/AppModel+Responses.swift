import Foundation

extension AppModel {
    func resolveApproval(_ approval: ApprovalSummary, decision: ApprovalDecision) async {
        guard let identity, let selectedAgent else { return }
        do {
            let controller = try requireController()
            _ = try await authenticate(identity: identity, agent: selectedAgent)
            try await controller.respondToApproval(
                request: ApprovalDecisionRequestData(
                    deviceID: identity.deviceID,
                    agentID: selectedAgent.agentID,
                    bindingID: selectedAgent.bindingID,
                    agentEncryptionPublicKey: selectedAgent.agentEncryptionPublicKey,
                    approvalID: approval.approvalID,
                    decision: decision
                )
            )
            if let index = approvals.firstIndex(where: { $0.approvalID == approval.approvalID }) {
                approvals[index].status = .resolved
            }
        } catch {
            pendingError = error.localizedDescription
        }
    }

    func respondToUserInput(_ request: UserInputRequestSummary, answers: [String: [String]]) async {
        guard let identity, let selectedAgent else { return }
        do {
            let controller = try requireController()
            _ = try await authenticate(identity: identity, agent: selectedAgent)
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
            if let index = userInputRequests.firstIndex(where: { $0.requestID == request.requestID }) {
                userInputRequests[index].status = .resolved
            }
        } catch {
            pendingError = error.localizedDescription
        }
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
            pendingError = error.localizedDescription
            return
        }
        await ensureRealtimeConnected(
            deviceID: identity.deviceID,
            agentID: selectedAgent.agentID,
            sessionToken: sessionToken,
            forceReconnect: connectionState != .connected
        )
    }
}
