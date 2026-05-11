import Foundation

extension AppModel {
    func threads(for projectID: String) -> [ThreadSummary] {
        (threadsByProject[projectID] ?? []).filter { $0.status != .archived }
    }

    /// Finds a project summary by project id.
    func project(for projectID: String) -> ProjectSummary? {
        projects.first(where: { $0.projectID == projectID })
    }

    /// Returns the in-memory timeline for SwiftUI rendering without mutating observed state.
    func timelineSnapshot(for threadID: String) -> ThreadTimeline? {
        guard var timeline = timelines[threadID] else {
            guard let transientEntries = transientOutgoingEntries[threadID], !transientEntries.isEmpty else {
                return nil
            }
            var transientTimeline = ThreadTimeline(
                threadID: threadID,
                ackSeq: 0,
                checkpoint: nil,
                entries: transientEntries
            )
            transientTimeline.sortForTimelineDisplay()
            return transientTimeline
        }
        mergeTransientEntries(into: &timeline, threadID: threadID)
        return timeline
    }

    /// Prepends one older page, using five earlier user messages as the page size.
    @discardableResult
    func loadPreviousThreadWindow(threadID: String) async -> Bool {
        guard let currentTimeline = timelines[threadID],
              let oldestUserSeq = currentTimeline.entries
                .filter({ $0.role == .user })
                .map(\.seq)
                .min()
        else {
            return false
        }

        let previousTimeline = await threadSyncPipeline.loadPreviousThreadWindow(
            threadID: threadID,
            beforeUserSeq: oldestUserSeq
        )
        guard !previousTimeline.entries.isEmpty else { return false }

        var combined = currentTimeline
        combined.ackSeq = previousTimeline.ackSeq
        combined.checkpoint = previousTimeline.checkpoint
        var existingIDs = Set(combined.entries.map(\.id))
        combined.entries.insert(
            contentsOf: previousTimeline.entries.filter { existingIDs.insert($0.id).inserted },
            at: 0
        )
        combined.sortForTimelineDisplay()
        mergeTransientEntries(into: &combined, threadID: threadID)
        timelines[threadID] = combined
        return true
    }

    /// Returns the current refresh state for a detail screen.
    func refreshStatus(for threadID: String) -> ThreadRefreshStatus {
        threadRefreshStates[threadID] ?? .idle()
    }

    func mergeTransientEntries(into timeline: inout ThreadTimeline, threadID: String) {
        if let transientEntries = transientOutgoingEntries[threadID], !transientEntries.isEmpty {
            var existingIDs = Set(timeline.entries.map(\.id))
            for entry in transientEntries where existingIDs.insert(entry.id).inserted {
                timeline.entries.append(entry)
            }
            timeline.sortForTimelineDisplay()
        }
    }

    /// Looks up a thread summary across all project buckets.
    func threadSummary(for threadID: String) -> ThreadSummary? {
        threadsByProject.values
            .flatMap { $0 }
            .first(where: { $0.threadID == threadID })
    }

    /// Counts pending approvals attached to a thread.
    func pendingApprovalCount(for threadID: String) -> Int {
        approvals.filter { $0.status == .pending && $0.threadID == threadID }.count
    }

    /// Counts pending user-input requests attached to a thread.
    func pendingUserInputCount(for threadID: String) -> Int {
        userInputRequests.filter { $0.threadID == threadID && ($0.status == .pending || $0.status == .failed) }.count
    }

    /// Calculates list ordering priority from pending user action state.
    func attentionPriority(for thread: ThreadSummary) -> Int {
        let pendingApprovals = pendingApprovalCount(for: thread.threadID)
        if pendingApprovals > 0 { return 4 }
        let pendingInputs = pendingUserInputCount(for: thread.threadID)
        if pendingInputs > 0 { return 4 }

        switch thread.status {
        case .waitingApproval:
            return 3
        case .running, .threadStarted:
            return 2
        case .failed, .systemError:
            return 1
        case .notLoaded, .created, .idle, .pending, .completed, .cancelled, .closed, .archived, .unknown:
            return 0
        }
    }
}
