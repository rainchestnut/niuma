import Foundation

extension AppModel {
    func selectAgent(_ agentID: String) async {
        selectedAgentID = agentID
        persistSelection()
        await refresh()
    }

    /// Pairs with a desktop agent using scanned QR JSON.
    func pairWithScannedPayload(_ raw: String) async -> Bool {
        pendingError = nil
        do {
            let payload = try PairCodePayloadDecoder.decode(raw)
            return await pair(payload: payload)
        } catch {
            deviceState = .pairFailed
            pendingError = error.localizedDescription
            return false
        }
    }

    /// Pairs with the local desktop gateway dashboard payload.
    func pairWithDesktopGateway() async -> Bool {
        pendingError = nil
        do {
            let controller = try requireController()
            let payload = try await controller.fetchDesktopPairingPayload()
            return await pair(payload: payload)
        } catch {
            deviceState = .pairFailed
            pendingError = error.localizedDescription
            return false
        }
    }

    /// Completes pairing from a decoded desktop payload.
    func pair(payload: PairCodePayload) async -> Bool {
        do {
            let controller = try requireController()
            let identity = try ensurePairingIdentity()
            deviceState = .pairing
            let response = try await controller.confirmPairing(
                request: try identityService.makePairConfirmRequest(
                    identity: identity,
                    payload: payload,
                    pushToken: storedPushToken
                )
            )
            let pairedAgent = PairedAgent(
                bindingID: response.bindingID,
                agentID: payload.agentID,
                agentEncryptionPublicKey: payload.agentEncryptionPublicKey,
                displayName: payload.agentName,
                fingerprint: payload.fingerprint,
                pairedAt: .now,
                sessionToken: nil
            )
            upsertPairedAgent(pairedAgent)
            selectedAgentID = pairedAgent.agentID
            persistSelection()
            deviceState = .paired
            await refresh()
            return true
        } catch {
            deviceState = .pairFailed
            pendingError = error.localizedDescription
            return false
        }
    }

    func removePairedAgent(_ agent: PairedAgent) async {
        pendingError = nil
        let previousState = deviceState
        deviceState = .pairing
        do {
            let controller = try requireController()
            let identity = try ensurePairingIdentity()
            let sessionToken = try await authenticate(identity: identity, agent: agent)
            controller.updateSessionToken(sessionToken)
            updateAgentToken(agentID: agent.agentID, sessionToken: sessionToken)
            let response = try await controller.revokePairing(
                request: PairRevokeRequestData(
                    deviceID: identity.deviceID,
                    agentID: agent.agentID
                )
            )
            guard response.revoked else {
                throw AppModelError.pairingRevocationFailed
            }
            removeLocalPairedAgent(agent.agentID)
            if let nextAgent = selectedAgent {
                selectedAgentID = nextAgent.agentID
                persistSelection()
                await refresh()
            } else {
                selectedAgentID = nil
                persistSelection()
                realtimeTask?.cancel()
                realtimeTask = nil
                connectionState = .disconnected
                runtimeState = .idle
                visibleThreadID = nil
                approvals = []
                approvalResponseFailures = [:]
                userInputRequests = []
                userInputResponseFailures = [:]
                projects = []
                threadsByProject = [:]
                timelines = [:]
                dataStore.resetAll()
                deviceState = .unpaired
            }
        } catch {
            deviceState = previousState
            pendingError = error.localizedDescription
        }
    }

    /// Sends a prompt into an existing session and optionally attaches files.
}
