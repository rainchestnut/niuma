import Foundation

extension AppModel {
    func cacheThreadSummary(_ thread: ThreadSummary) {
        var threads = threadsByProject[thread.projectID] ?? []
        if let index = threads.firstIndex(where: { $0.threadID == thread.threadID }) {
            threads[index] = thread
        } else {
            threads.insert(thread, at: 0)
        }
        threads.sort(by: { $0.updatedAt > $1.updatedAt })
        threadsByProject[thread.projectID] = threads
    }

    /// Removes an archived Codex thread from all mobile-local projections.
    func deleteArchivedThread(_ thread: ThreadSummary) {
        for projectID in Array(threadsByProject.keys) {
            threadsByProject[projectID]?.removeAll { $0.threadID == thread.threadID }
        }
        clearThreadRuntimeState(threadID: thread.threadID)
        dataStore.deleteThreadCascade(threadID: thread.threadID)
        localAttachments = dataStore.loadLocalAttachments()
    }

    /// Clears only mobile-local history for a visible thread so the next detail
    /// entry replays from Codex app-server without removing the list row.
    func resetLocalHistory(for thread: ThreadSummary) {
        clearThreadRuntimeState(threadID: thread.threadID)
        dataStore.resetThreadHistory(threadID: thread.threadID)
        localAttachments = dataStore.loadLocalAttachments()
    }

    /// Clears in-memory timeline and refresh state for a local thread history reset.
    func clearThreadRuntimeState(threadID: String) {
        timelines.removeValue(forKey: threadID)
        threadRefreshStates.removeValue(forKey: threadID)
        transientOutgoingEntries.removeValue(forKey: threadID)
        branchChangesByThread.removeValue(forKey: threadID)
        threadRefreshTimeoutTasks.removeValue(forKey: threadID)?.cancel()
    }

    func upsertThreadSummary(_ thread: ThreadSummary, saveImmediately: Bool = true) {
        cacheThreadSummary(thread)
        dataStore.upsertThread(thread, saveImmediately: saveImmediately)
    }

    func upsertPairedAgent(_ agent: PairedAgent) {
        if let index = pairedAgents.firstIndex(where: { $0.agentID == agent.agentID }) {
            pairedAgents[index] = agent
        } else {
            pairedAgents.insert(agent, at: 0)
        }
        persistPairedAgents()
    }

    func updateAgentToken(agentID: String, sessionToken: String) {
        guard let index = pairedAgents.firstIndex(where: { $0.agentID == agentID }) else { return }
        pairedAgents[index].sessionToken = sessionToken
        persistPairedAgents()
    }

    func updateAgentOnline(agentID: String, isOnline: Bool) {
        guard let index = pairedAgents.firstIndex(where: { $0.agentID == agentID }) else { return }
        pairedAgents[index].isOnline = isOnline
        persistPairedAgents()
    }

    /// Removes LocalPairedAgent.
    func removeLocalPairedAgent(_ agentID: String) {
        pairedAgents.removeAll(where: { $0.agentID == agentID })
        if selectedAgentID == agentID {
            selectedAgentID = pairedAgents.first?.agentID
        }
        persistPairedAgents()
        persistSelection()
    }

    /// Persists paired agents and the selected agent id to SwiftData.
    func persistPairedAgents() {
        dataStore.replaceAgents(pairedAgents, selectedAgentID: selectedAgentID)
    }

    /// Persists only the selected agent id after selection changes.
    func persistSelection() {
        dataStore.selectAgent(selectedAgentID)
    }

    /// Detects the server reset case for the current local device identity.
    /// - Parameters:
    ///   - error: Transport error returned by the HTTP client.
    ///   - deviceID: Current local iOS identity being authenticated.
    /// - Returns: `true` when the live server explicitly rejected this device id as unknown.
    func isServerForgotCurrentDevice(_ error: Error, deviceID: String) -> Bool {
        guard let httpError = error as? NiumaHTTPError else { return false }
        guard httpError.statusCode == 404 || httpError.statusCode == 401 else { return false }
        return httpError.detail.contains("unknown device_id=\(deviceID)")
    }

    /// Clears stale paired state after the server database no longer contains this iOS identity.
    func resetPairingAfterServerIdentityLoss() {
        realtimeTask?.cancel()
        realtimeTask = nil
        controller?.disconnectRealtime()
        controller?.updateSessionToken(nil)
        for task in threadRefreshTimeoutTasks.values {
            task.cancel()
        }
        threadRefreshTimeoutTasks.removeAll()
        pairedAgents = []
        selectedAgentID = nil
        projects = []
        threadsByProject = [:]
        approvals = []
        approvalResponseFailures = [:]
        userInputRequests = []
        timelines = [:]
        threadRefreshStates = [:]
        transientOutgoingEntries = [:]
        localAttachments = [:]
        visibleThreadID = nil
        availableModels = []
        selectedModelID = nil
        storage.set(nil, forKey: StorageKey.selectedModelID.rawValue)
        dataStore.resetAll()
        deviceState = .unpaired
        connectionState = .disconnected
        runtimeState = .idle
    }

    /// Swaps the active transport endpoint and tears down server-scoped auth state.
    func replaceController(serverBaseURL: URL) {
        realtimeTask?.cancel()
        realtimeTask = nil
        controller?.disconnectRealtime()
        controller?.updateSessionToken(nil)
        for task in threadRefreshTimeoutTasks.values {
            task.cancel()
        }
        threadRefreshTimeoutTasks.removeAll()
        self.serverBaseURL = serverBaseURL
        controller = controllerFactory(serverBaseURL)
        connectionState = .disconnected
        runtimeState = .idle
    }

    static func transferAdditionalData(
        direction: PayloadCryptoDirection,
        sourceDeviceID: String,
        targetDeviceID: String
    ) -> Data {
        PayloadCryptoService.additionalData([
            ("kind", "transfer"),
            ("direction", direction.rawValue),
            ("source_device_id", sourceDeviceID),
            ("target_device_id", targetDeviceID)
        ])
    }
}
