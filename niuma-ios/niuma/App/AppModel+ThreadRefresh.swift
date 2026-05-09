import Foundation
import OSLog

extension AppModel {
    func refreshBranchChanges(threadID: String, baseRef: String? = nil) async {
        guard let identity, let selectedAgent else { return }
        do {
            let controller = try requireController()
            let sessionToken = try await authenticate(identity: identity, agent: selectedAgent)
            controller.updateSessionToken(sessionToken)
            await ensureRealtimeConnected(
                deviceID: identity.deviceID,
                agentID: selectedAgent.agentID,
                sessionToken: sessionToken,
                forceReconnect: connectionState != .connected
            )
            guard connectionState == .connected else {
                throw AppModelError.realtimeNotConnected
            }
            let requestID = UUID().uuidString.lowercased()
            try await withTimeout(realtimeSendTimeout) {
                try await controller.requestBranchChanges(
                    request: BranchChangesRequestData(
                        requestID: requestID,
                        deviceID: identity.deviceID,
                        threadID: threadID,
                        baseRef: baseRef
                    )
                )
            }
        } catch {
            pendingError = error.localizedDescription
        }
    }

    /// Ensures the branch-change diff bundle referenced by a result is available for local detail rendering.
    func ensureBranchChangeBundleDownloaded(_ result: BranchChangesResult) async {
        guard result.succeeded,
              let transferID = result.transferID,
              localAttachmentData(forTransferID: transferID) == nil,
              let identity,
              let selectedAgent else {
            return
        }
        await receiveTransferIfNeeded(
            TransferReady(
                transferID: transferID,
                direction: .agentToIOS,
                sourceDeviceID: selectedAgent.agentID,
                targetDeviceID: identity.deviceID,
                encryptedSizeBytes: result.sizeBytes ?? 0,
                expiresAt: 0
            )
        )
    }

    func refreshThreadDetails(threadID: String) async {
        guard let identity, let selectedAgent else { return }
        guard let thread = threadSummary(for: threadID) else {
            logger.warning("thread_refresh_missing_summary thread_id=\(threadID, privacy: .public)")
            pendingError = "未找到可刷新的会话详情"
            return
        }
        guard !refreshStatus(for: thread.threadID).isRefreshing else {
            logger.info("thread_refresh_already_running thread_id=\(thread.threadID, privacy: .public)")
            return
        }

        let refreshID = beginThreadRefresh(threadID: thread.threadID)
        startThreadRefreshTimeout(refreshID: refreshID, threadID: thread.threadID)

        logger.info("thread_refresh_start thread_id=\(thread.threadID, privacy: .public) agent_id=\(selectedAgent.agentID, privacy: .public) connection=\(String(describing: self.connectionState), privacy: .public)")
        do {
            let controller = try requireController()
            let sessionToken = try await authenticate(identity: identity, agent: selectedAgent)
            controller.updateSessionToken(sessionToken)
            await ensureRealtimeConnected(
                deviceID: identity.deviceID,
                agentID: selectedAgent.agentID,
                sessionToken: sessionToken,
                forceReconnect: connectionState != .connected
            )
            guard connectionState == .connected else {
                logger.error("thread_refresh_not_connected thread_id=\(thread.threadID, privacy: .public) connection=\(String(describing: self.connectionState), privacy: .public)")
                throw AppModelError.realtimeNotConnected
            }
            try await sendResumeThread(thread: thread)
            logger.info("thread_refresh_resume_sent thread_id=\(thread.threadID, privacy: .public)")
        } catch {
            let message = error.localizedDescription
            logger.error("thread_refresh_failed thread_id=\(thread.threadID, privacy: .public) error=\(message, privacy: .public)")
            if finishThreadRefresh(threadID: thread.threadID, phase: .failed, error: message, visibleEntries: nil) {
                runtimeState = .failed
                pendingError = message
            }
            controller?.disconnectRealtime()
            connectionState = .degraded
        }
    }

    /// Sends the current detail cursor/checkpoint to the desktop gateway.
    func sendResumeThread(thread: ThreadSummary) async throws {
        let syncState = await threadSyncPipeline.loadThreadSyncState(threadID: thread.threadID)
        let localEntryCount = timelines[thread.threadID]?.entries.count ?? 0
        logger.info("resume_thread_send thread_id=\(thread.threadID, privacy: .public) cursor=\(syncState.cursor, privacy: .public) checkpoint_present=\((syncState.checkpoint != nil), privacy: .public) local_entries=\(localEntryCount, privacy: .public)")
        let controller = try requireController()
        try await withTimeout(realtimeSendTimeout) {
            try await controller.resumeThread(
                request: ResumeThreadRequestData(
                    threadID: thread.threadID,
                    cursor: syncState.cursor,
                    checkpoint: syncState.checkpoint
                )
            )
        }
    }

    /// Marks a refresh as running and logs the explicit timeout boundary.
    func beginThreadRefresh(threadID: String) -> String {
        let refreshID = UUID().uuidString.lowercased()
        threadRefreshStates[threadID] = ThreadRefreshStatus(
            refreshID: refreshID,
            phase: .refreshing,
            error: nil
        )
        logger.info("thread_refresh_state thread_id=\(threadID, privacy: .public) refresh_id=\(refreshID, privacy: .public) phase=refreshing timeout_seconds=300")
        pendingError = nil
        return refreshID
    }

    /// Starts the 300-second detail-refresh guard for one thread.
    func startThreadRefreshTimeout(refreshID: String, threadID: String) {
        cancelThreadRefreshTimeout(threadID: threadID)
        let logger = Logger(subsystem: "com.rainchestnut.niuma", category: "realtime")
        threadRefreshTimeoutTasks[threadID] = Task.detached(priority: .utility) { [weak self] in
            do {
                try await Task.sleep(for: .seconds(300))
            } catch {
                return
            }
            guard !Task.isCancelled else { return }
            logger.error("thread_refresh_timeout_deadline_reached thread_id=\(threadID, privacy: .public) refresh_id=\(refreshID, privacy: .public) timeout_seconds=300")
            await self?.finishThreadRefreshIfTimedOut(refreshID: refreshID, threadID: threadID)
        }
    }

    /// Converts an over-deadline refresh into a terminal failure state.
    func finishThreadRefreshIfTimedOut(refreshID: String, threadID: String) {
        let status = refreshStatus(for: threadID)
        guard status.refreshID == refreshID, status.isRefreshing else { return }
        let error = "刷新超过 300 秒未收到完成或失败事件"
        if finishThreadRefresh(threadID: threadID, phase: .timedOut, error: error, visibleEntries: nil) {
            runtimeState = .failed
            pendingError = error
        }
    }

    /// Stops the timeout task for a refresh that already reached a terminal marker.
    func cancelThreadRefreshTimeout(threadID: String) {
        threadRefreshTimeoutTasks.removeValue(forKey: threadID)?.cancel()
    }

    /// Applies pipeline output to observed SwiftUI state on the main actor.
    func applyPipelineResults(_ results: [ThreadSyncPipelineResult]) {
        for result in results {
            switch result {
            case .syncCompleted(var timeline):
                removeTransientUserPrompts(threadID: timeline.threadID, matching: timeline.entries)
                mergeTransientEntries(into: &timeline, threadID: timeline.threadID)
                timelines[timeline.threadID] = timeline
                logger.info("thread_sync_state thread_id=\(timeline.threadID, privacy: .public) phase=succeeded visible_entries=\(timeline.entries.count, privacy: .public)")
                _ = finishThreadRefresh(
                    threadID: timeline.threadID,
                    phase: .succeeded,
                    error: nil,
                    visibleEntries: timeline.entries.count
                )
                runtimeState = .completed
                pendingError = nil

            case .syncFailed(let threadID, let phase, let error, var timeline):
                removeTransientUserPrompts(threadID: threadID, matching: timeline.entries)
                mergeTransientEntries(into: &timeline, threadID: threadID)
                timelines[threadID] = timeline
                if !finishThreadRefresh(threadID: threadID, phase: phase, error: error, visibleEntries: timeline.entries.count) {
                    logger.error("thread_sync_state thread_id=\(threadID, privacy: .public) phase=\(phase.rawValue, privacy: .public) error=\(error, privacy: .public)")
                }
                runtimeState = .failed
                pendingError = error
            }
        }
    }

    /// Finishes the active refresh immediately instead of queueing local
    /// terminal states behind websocket replay persistence.
    @discardableResult
    func finishThreadRefresh(
        threadID: String,
        phase: ThreadRefreshPhase,
        error: String?,
        visibleEntries: Int?
    ) -> Bool {
        let status = refreshStatus(for: threadID)
        guard let refreshID = status.refreshID, status.isRefreshing else { return false }
        cancelThreadRefreshTimeout(threadID: threadID)
        threadRefreshStates[threadID] = ThreadRefreshStatus(
            refreshID: refreshID,
            phase: phase,
            error: error
        )
        if phase == .timedOut {
            logger.error("thread_refresh_state thread_id=\(threadID, privacy: .public) refresh_id=\(refreshID, privacy: .public) phase=timed_out timeout_seconds=300 error=\(error ?? "", privacy: .public)")
        } else if let error {
            logger.error("thread_refresh_state thread_id=\(threadID, privacy: .public) refresh_id=\(refreshID, privacy: .public) phase=\(phase.rawValue, privacy: .public) error=\(error, privacy: .public)")
        } else if let visibleEntries {
            logger.info("thread_refresh_state thread_id=\(threadID, privacy: .public) refresh_id=\(refreshID, privacy: .public) phase=\(phase.rawValue, privacy: .public) visible_entries=\(visibleEntries, privacy: .public)")
        } else {
            logger.info("thread_refresh_state thread_id=\(threadID, privacy: .public) refresh_id=\(refreshID, privacy: .public) phase=\(phase.rawValue, privacy: .public)")
        }
        return true
    }

    /// Converts transport-level interruptions into terminal refresh UI states.
    func failActiveRefreshes(error: String) async {
        let activeThreadIDs = threadRefreshStates.compactMap { threadID, status in
            status.isRefreshing ? threadID : nil
        }
        guard !activeThreadIDs.isEmpty else { return }
        for threadID in activeThreadIDs {
            _ = finishThreadRefresh(threadID: threadID, phase: .failed, error: error, visibleEntries: nil)
        }
        runtimeState = .failed
        pendingError = error
    }
}
