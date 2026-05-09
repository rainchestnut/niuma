import Foundation
import OSLog
import SwiftData

nonisolated enum ThreadSyncPipelineEvent: @unchecked Sendable {
    case taskUpdate(RealtimeTaskUpdate)
    case completed(ThreadSyncCompletion)
    case failed(ThreadSyncFailure)
}

nonisolated enum ThreadSyncPipelineResult: @unchecked Sendable {
    case syncCompleted(ThreadTimeline)
    case syncFailed(threadID: String, phase: ThreadRefreshPhase, error: String, timeline: ThreadTimeline)
}

/// Serial consumer for thread sync events from the realtime websocket.
///
/// Message records are persisted as soon as they arrive. UI-facing timeline
/// reloads happen only when the desktop gateway sends a terminal marker.
final class ThreadSyncPipeline {
    let results: AsyncStream<[ThreadSyncPipelineResult]>

    private let worker: ThreadSyncPipelineWorker
    private let eventContinuation: AsyncStream<ThreadSyncPipelineEvent>.Continuation
    private let resultContinuation: AsyncStream<[ThreadSyncPipelineResult]>.Continuation
    private let workerTask: Task<Void, Never>

    /// Opens the pipeline and starts the background sync-event consumer.
    init() throws {
        let eventChannel = AsyncStream.makeStream(
            of: ThreadSyncPipelineEvent.self,
            bufferingPolicy: .unbounded
        )
        let resultChannel = AsyncStream.makeStream(
            of: [ThreadSyncPipelineResult].self,
            bufferingPolicy: .unbounded
        )
        let worker = try ThreadSyncPipelineWorker(resultContinuation: resultChannel.continuation)

        self.worker = worker
        self.eventContinuation = eventChannel.continuation
        self.resultContinuation = resultChannel.continuation
        self.results = resultChannel.stream
        self.workerTask = Task {
            await worker.consume(eventChannel.stream)
        }
    }

    deinit {
        eventContinuation.finish()
        resultContinuation.finish()
        workerTask.cancel()
    }

    /// Builds the production pipeline used by `AppModel`.
    static func live() -> ThreadSyncPipeline {
        do {
            return try ThreadSyncPipeline()
        } catch {
            fatalError("Failed to open Niuma thread sync pipeline: \(error)")
        }
    }

    /// Submits one realtime event without blocking the websocket reader on SwiftData work.
    func submit(_ event: ThreadSyncPipelineEvent) {
        eventContinuation.yield(event)
    }

    /// Clears worker-owned thread replay records after a full app data reset.
    func resetLocalThreadState() async {
        await worker.resetLocalThreadState()
    }

    /// Loads one older visible window before the current oldest visible user row.
    func loadPreviousThreadWindow(
        threadID: String,
        beforeUserSeq: Int,
        userLimit: Int = 5
    ) async -> ThreadTimeline {
        await worker.loadPreviousThreadWindow(
            threadID: threadID,
            beforeUserSeq: beforeUserSeq,
            userLimit: userLimit
        )
    }

    /// Returns the persisted replay cursor without consulting the visible UI window.
    func loadThreadSyncState(threadID: String) async -> (cursor: Int, checkpoint: String?) {
        await worker.loadThreadSyncState(threadID: threadID)
    }
}

/// Owns the SwiftData context used to process queued thread-sync events.
private actor ThreadSyncPipelineWorker {
    private let container: ModelContainer
    private let context: ModelContext
    private let encoder = JSONEncoder()
    private let decoder = JSONDecoder()
    private let resultContinuation: AsyncStream<[ThreadSyncPipelineResult]>.Continuation
    private let logger = Logger(subsystem: "com.rainchestnut.niuma", category: "thread-sync-pipeline")

    /// Opens a dedicated SwiftData context for sync ingestion.
    init(resultContinuation: AsyncStream<[ThreadSyncPipelineResult]>.Continuation) throws {
        self.container = try NiumaModelStore.makeContainer()
        self.context = ModelContext(container)
        self.resultContinuation = resultContinuation
        encoder.dateEncodingStrategy = .iso8601
        decoder.dateDecodingStrategy = .iso8601
    }

    /// Drains the pipeline stream one event at a time.
    func consume(_ events: AsyncStream<ThreadSyncPipelineEvent>) async {
        for await event in events {
            let results = process(event)
            guard !results.isEmpty else { continue }
            resultContinuation.yield(results)
        }
    }

    /// Processes one queued realtime sync event.
    private func process(_ event: ThreadSyncPipelineEvent) -> [ThreadSyncPipelineResult] {
        switch event {
        case .taskUpdate(let update):
            return append(update)

        case .completed(let completion):
            return commit(completion)

        case .failed(let failure):
            return fail(threadID: failure.threadID, phase: .failed, error: failure.error)
        }
    }

    /// Removes thread rows and entries held by the pipeline's dedicated SwiftData context.
    func resetLocalThreadState() {
        for entry in fetchAll(StoredThreadEntry.self) {
            context.delete(entry)
        }
        for thread in fetchAll(StoredThread.self) {
            context.delete(thread)
        }
        save()
    }

    /// Loads the latest visible window: five user turns plus following records.
    func loadLatestThreadWindow(threadID: String, userLimit: Int = 5) -> ThreadTimeline {
        let userEntries = fetchUserEntries(threadID: threadID, beforeSeq: nil, limit: userLimit)
        guard let minUserSeq = userEntries.map(\.seq).min() else {
            return emptyTimeline(threadID: threadID)
        }
        return timeline(
            threadID: threadID,
            entries: fetchEntries(threadID: threadID, minSeq: minUserSeq, maxSeqExclusive: nil)
        )
    }

    /// Loads one older visible window before the current oldest visible user row.
    func loadPreviousThreadWindow(
        threadID: String,
        beforeUserSeq: Int,
        userLimit: Int = 5
    ) -> ThreadTimeline {
        let userEntries = fetchUserEntries(threadID: threadID, beforeSeq: beforeUserSeq, limit: userLimit)
        guard let minUserSeq = userEntries.map(\.seq).min() else {
            return emptyTimeline(threadID: threadID)
        }
        return timeline(
            threadID: threadID,
            entries: fetchEntries(threadID: threadID, minSeq: minUserSeq, maxSeqExclusive: beforeUserSeq)
        )
    }

    /// Returns the persisted replay cursor without consulting the visible UI window.
    func loadThreadSyncState(threadID: String) -> (cursor: Int, checkpoint: String?) {
        let thread = storedThread(threadID)
        return (thread?.cursor ?? 0, thread?.checkpoint ?? thread?.lastCheckpointSeen)
    }

    private func append(_ update: RealtimeTaskUpdate) -> [ThreadSyncPipelineResult] {
        upsertEntry(update.entry, threadID: update.threadID)
        save()
        logger.info("thread_sync_pipeline_stored thread_id=\(update.threadID, privacy: .public) seq=\(update.seq, privacy: .public) entry_id=\(update.entry.id, privacy: .public)")
        return []
    }

    private func commit(_ completion: ThreadSyncCompletion) -> [ThreadSyncPipelineResult] {
        completeThreadSync(
            threadID: completion.threadID,
            cursor: completion.cursor,
            checkpoint: completion.checkpoint
        )
        let timeline = loadLatestThreadWindow(threadID: completion.threadID)
        logger.info("thread_sync_pipeline_completed thread_id=\(completion.threadID, privacy: .public) entry_count=\(completion.entryCount, privacy: .public) cursor=\(completion.cursor, privacy: .public)")
        return [.syncCompleted(timeline)]
    }

    private func fail(threadID: String, phase: ThreadRefreshPhase, error: String) -> [ThreadSyncPipelineResult] {
        let timeline = loadLatestThreadWindow(threadID: threadID)
        logger.error("thread_sync_pipeline_failed thread_id=\(threadID, privacy: .public) phase=\(phase.rawValue, privacy: .public) error=\(error, privacy: .public)")
        return [.syncFailed(threadID: threadID, phase: phase, error: error, timeline: timeline)]
    }

    private func emptyTimeline(threadID: String) -> ThreadTimeline {
        timeline(threadID: threadID, entries: [])
    }

    private func timeline(threadID: String, entries: [StoredThreadEntry]) -> ThreadTimeline {
        let thread = storedThread(threadID)
        return ThreadTimeline(
            threadID: threadID,
            ackSeq: thread?.cursor ?? 0,
            checkpoint: thread?.checkpoint ?? thread?.lastCheckpointSeen,
            entries: entries.map(toDomainEntry)
        )
    }

    private func upsertThread(_ thread: ThreadSummary) {
        if let stored = storedThread(thread.threadID) {
            stored.update(thread: thread)
        } else {
            context.insert(StoredThread(thread: thread))
        }
    }

    private func upsertEntry(_ entry: ThreadEntry, threadID: String) {
        let data = encodeContentParts(entry.contentParts)
        let storageID = StoredThreadEntry.storageID(threadID: threadID, entryID: entry.id)
        if let stored = storedEntry(storageID) {
            stored.update(entry: entry, contentPartsData: data)
        } else {
            context.insert(StoredThreadEntry(threadID: threadID, entry: entry, contentPartsData: data))
        }
    }

    private func completeThreadSync(threadID: String, cursor: Int, checkpoint: String?) {
        let thread = ensureStoredThread(threadID: threadID)
        let previousCursor = thread.cursor
        if cursor >= previousCursor {
            thread.cursor = cursor
        }
        if let checkpoint, cursor >= previousCursor {
            thread.checkpoint = checkpoint
        }
        save()
    }

    private func ensureStoredThread(threadID: String) -> StoredThread {
        if let thread = storedThread(threadID) {
            return thread
        }
        let fallback = ThreadSummary(
            threadID: threadID,
            projectID: "",
            agentID: "",
            title: threadID,
            status: .notLoaded,
            lastCheckpointSeen: nil,
            updatedAt: .now
        )
        let stored = StoredThread(thread: fallback)
        context.insert(stored)
        return stored
    }

    private func storedThread(_ threadID: String) -> StoredThread? {
        first(StoredThread.self, #Predicate { $0.threadID == threadID })
    }

    private func storedEntry(_ storageID: String) -> StoredThreadEntry? {
        first(StoredThreadEntry.self, #Predicate { $0.storageID == storageID })
    }

    private func fetchUserEntries(
        threadID: String,
        beforeSeq: Int?,
        limit: Int
    ) -> [StoredThreadEntry] {
        let userRole = ThreadEntryRole.user.rawValue
        let predicate: Predicate<StoredThreadEntry>
        if let beforeSeq {
            predicate = #Predicate {
                $0.threadID == threadID
                    && $0.roleRawValue == userRole
                    && $0.seq < beforeSeq
            }
        } else {
            predicate = #Predicate {
                $0.threadID == threadID
                    && $0.roleRawValue == userRole
            }
        }
        var descriptor = FetchDescriptor<StoredThreadEntry>(
            predicate: predicate,
            sortBy: [SortDescriptor(\.seq, order: .reverse)]
        )
        descriptor.fetchLimit = limit
        descriptor.includePendingChanges = true
        return (try? context.fetch(descriptor)) ?? []
    }

    private func fetchEntries(
        threadID: String,
        minSeq: Int,
        maxSeqExclusive: Int?
    ) -> [StoredThreadEntry] {
        let predicate: Predicate<StoredThreadEntry>
        if let maxSeqExclusive {
            predicate = #Predicate {
                $0.threadID == threadID
                    && $0.seq >= minSeq
                    && $0.seq < maxSeqExclusive
            }
        } else {
            predicate = #Predicate {
                $0.threadID == threadID
                    && $0.seq >= minSeq
            }
        }
        var descriptor = FetchDescriptor<StoredThreadEntry>(
            predicate: predicate,
            sortBy: [SortDescriptor(\.seq), SortDescriptor(\.createdAt)]
        )
        descriptor.includePendingChanges = true
        return (try? context.fetch(descriptor)) ?? []
    }

    private func first<T: PersistentModel>(_ model: T.Type, _ predicate: Predicate<T>) -> T? {
        var descriptor = FetchDescriptor<T>(predicate: predicate)
        descriptor.fetchLimit = 1
        descriptor.includePendingChanges = true
        return try? context.fetch(descriptor).first
    }

    private func fetchAll<T: PersistentModel>(_ model: T.Type) -> [T] {
        var descriptor = FetchDescriptor<T>()
        descriptor.includePendingChanges = true
        return (try? context.fetch(descriptor)) ?? []
    }

    private func toDomainEntry(_ stored: StoredThreadEntry) -> ThreadEntry {
        ThreadEntry(
            id: stored.entryID,
            role: ThreadEntryRole(rawValue: stored.roleRawValue) ?? .assistant,
            type: stored.type,
            phase: stored.phase,
            text: stored.text,
            createdAt: stored.createdAt,
            seq: stored.seq,
            contentParts: decodeContentParts(stored.contentPartsData)
        )
    }

    private func encodeContentParts(_ parts: [ContentPart]?) -> Data? {
        guard let parts else { return nil }
        return try? encoder.encode(parts)
    }

    private func decodeContentParts(_ data: Data?) -> [ContentPart]? {
        guard let data else { return nil }
        return try? decoder.decode([ContentPart].self, from: data)
    }

    private func save() {
        do {
            try context.save()
        } catch {
            assertionFailure("Failed to save Niuma thread sync pipeline: \(error)")
        }
    }
}
