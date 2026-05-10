import Foundation
import SwiftData

nonisolated enum NiumaModelStore {
    /// Builds the single SwiftData schema used by UI-facing and sync-pipeline contexts.
    static func makeContainer() throws -> ModelContainer {
        do {
            return try buildContainer()
        } catch {
            // The app is still in testing. A schema conflict means local test
            // state is obsolete, so remove it and recreate a clean store.
            deleteLocalStoreFiles()
            deleteLocalAttachmentFiles()
            return try buildContainer()
        }
    }

    /// Builds the SwiftData container using the app's default store location.
    private static func buildContainer() throws -> ModelContainer {
        try ModelContainer(
            for: StoredAgent.self,
            StoredProject.self,
            StoredThread.self,
            StoredThreadEntry.self,
            StoredLocalAttachment.self,
            configurations: ModelConfiguration()
        )
    }

    /// Removes default SwiftData SQLite files when the test schema changes.
    private static func deleteLocalStoreFiles() {
        guard let appSupport = try? FileManager.default.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: false
        ) else {
            return
        }
        for fileName in ["default.store", "default.store-shm", "default.store-wal"] {
            try? FileManager.default.removeItem(at: appSupport.appendingPathComponent(fileName))
        }
    }

    /// Removes attachment files whose SwiftData mappings were discarded.
    private static func deleteLocalAttachmentFiles() {
        guard let directory = try? NiumaDataStore.makeLocalAttachmentsDirectory(create: false) else {
            return
        }
        try? FileManager.default.removeItem(at: directory)
    }
}

/// App-local persistence facade for Niuma mobile state.
///
/// Metadata and local attachment records remain on the main actor. Thread
/// message entries are owned by `ThreadSyncPipeline` so replay batches have one
/// persistence path and one terminal commit point.
@MainActor
final class NiumaDataStore {
    private let container: ModelContainer
    private let context: ModelContext
    private let fileManager: FileManager
    private let localAttachmentsDirectory: URL

    /// Opens the live on-disk store.
    init() throws {
        let fileManager = FileManager.default
        self.fileManager = fileManager
        self.localAttachmentsDirectory = try Self.makeLocalAttachmentsDirectory(fileManager: fileManager)
        self.container = try NiumaModelStore.makeContainer()
        self.context = ModelContext(container)
    }

    /// Builds the production store used by the app container.
    static func live() -> NiumaDataStore {
        do {
            return try NiumaDataStore()
        } catch {
            fatalError("Failed to open Niuma SwiftData store: \(error)")
        }
    }

    /// Deletes all SwiftData-backed mobile state while leaving app settings in UserDefaults.
    func resetAll() {
        for entry in fetchAll(StoredThreadEntry.self) {
            context.delete(entry)
        }
        for attachment in fetchAll(StoredLocalAttachment.self) {
            context.delete(attachment)
        }
        for thread in fetchAll(StoredThread.self) {
            context.delete(thread)
        }
        for project in fetchAll(StoredProject.self) {
            context.delete(project)
        }
        for agent in fetchAll(StoredAgent.self) {
            context.delete(agent)
        }
        try? fileManager.removeItem(at: localAttachmentsDirectory)
        try? fileManager.createDirectory(at: localAttachmentsDirectory, withIntermediateDirectories: true)
        save()
    }

    /// Loads paired agents and resolves the currently selected agent from the DB.
    func loadAgents() -> (agents: [PairedAgent], selectedAgentID: String?) {
        let stored = fetchAll(StoredAgent.self).sorted(by: { $0.pairedAt > $1.pairedAt })
        return (stored.map { $0.toDomain() }, stored.first(where: \.isSelected)?.agentID)
    }

    func replaceAgents(_ agents: [PairedAgent], selectedAgentID: String?) {
        let keep = Set(agents.map(\.agentID))
        for stored in fetchAll(StoredAgent.self) where !keep.contains(stored.agentID) {
            context.delete(stored)
        }
        for agent in agents {
            upsertAgent(agent, isSelected: agent.agentID == selectedAgentID, saveImmediately: false)
        }
        save()
    }

    func selectAgent(_ selectedAgentID: String?) {
        for stored in fetchAll(StoredAgent.self) {
            stored.isSelected = stored.agentID == selectedAgentID
        }
        save()
    }

    func upsertAgent(_ agent: PairedAgent, isSelected: Bool, saveImmediately: Bool = true) {
        if let stored = storedAgent(agent.agentID) {
            stored.update(agent: agent, isSelected: isSelected)
        } else {
            context.insert(StoredAgent(agent: agent, isSelected: isSelected))
        }
        if isSelected {
            for stored in fetchAll(StoredAgent.self) where stored.agentID != agent.agentID {
                stored.isSelected = false
            }
        }
        if saveImmediately {
            save()
        }
    }

    func loadProjects() -> [ProjectSummary] {
        fetchAll(StoredProject.self)
            .map { $0.toDomain() }
    }

    /// Inserts or updates one project summary row.
    func upsertProject(_ project: ProjectSummary, saveImmediately: Bool = true) {
        if let stored = storedProject(project.projectID) {
            stored.update(project: project)
        } else {
            context.insert(StoredProject(project: project))
        }
        if saveImmediately {
            save()
        }
    }

    /// Loads threads grouped by project using the store's default fetch order.
    func loadThreadsByProject() -> [String: [ThreadSummary]] {
        Dictionary(grouping: loadThreads(), by: \.projectID)
    }

    /// Clears cached message history for a thread while preserving its list row.
    func resetThreadHistory(threadID: String) {
        deleteThreadLocalHistory(threadID: threadID, shouldDeleteThread: false)
    }

    /// Deletes one thread projection, its message entries, and attachment files.
    func deleteThreadCascade(threadID: String) {
        deleteThreadLocalHistory(threadID: threadID, shouldDeleteThread: true)
    }

    /// Shared local-history deletion used by cache reset and archived-thread deletion.
    private func deleteThreadLocalHistory(threadID: String, shouldDeleteThread: Bool) {
        let threads = fetchAll(StoredThread.self).filter { $0.threadID == threadID }
        let threadIDs = Set(threads.map(\.threadID)).union([threadID])
        let allEntries = fetchAll(StoredThreadEntry.self)
        let deletedEntries = allEntries.filter { threadIDs.contains($0.threadID) }
        let keptEntries = allEntries.filter { !threadIDs.contains($0.threadID) }
        let removableTransferIDs = transferIDs(in: deletedEntries).subtracting(transferIDs(in: keptEntries))

        for entry in deletedEntries {
            context.delete(entry)
        }
        if shouldDeleteThread {
            for thread in threads {
                context.delete(thread)
            }
        } else {
            for thread in threads {
                thread.cursor = 0
                thread.checkpoint = nil
                thread.lastCheckpointSeen = nil
            }
        }
        for attachment in fetchAll(StoredLocalAttachment.self) where removableTransferIDs.contains(attachment.transferID) {
            deleteLocalAttachment(attachment)
        }
        save()
    }

    /// Inserts or updates one thread summary row.
    func upsertThread(_ thread: ThreadSummary, saveImmediately: Bool = true) {
        if let stored = storedThread(thread.threadID) {
            stored.update(thread: thread)
        } else {
            context.insert(StoredThread(thread: thread))
        }
        if saveImmediately {
            save()
        }
    }

    /// Saves attachment bytes under Application Support and returns the stable relative path.
    /// - Parameters:
    ///   - transferID: SHA-256 transfer id used as the directory key.
    ///   - fileName: Optional original display name used only for the final path component.
    ///   - data: Attachment bytes that should be available for future local rendering.
    /// - Returns: Relative path stored in SwiftData.
    func saveLocalAttachmentFile(transferID: String, fileName: String?, data: Data) throws -> String {
        let safeTransferID = Self.safePathComponent(transferID)
        let safeFileName = Self.safeFileName(fileName)
        let directory = localAttachmentsDirectory.appendingPathComponent(safeTransferID, isDirectory: true)
        try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        let fileURL = directory.appendingPathComponent(safeFileName, isDirectory: false)
        try data.write(to: fileURL, options: [.atomic])
        return "\(safeTransferID)/\(safeFileName)"
    }

    /// Reads locally cached attachment bytes for rendering without touching the server cache.
    /// - Parameters:
    ///   - payload: Attachment row containing the local relative path.
    /// - Returns: File bytes when the local copy still exists.
    func localAttachmentData(_ payload: LocalAttachmentPayload) -> Data? {
        guard let url = localAttachmentURL(relativePath: payload.localRelativePath) else {
            return nil
        }
        return try? Data(contentsOf: url)
    }

    /// Upserts a local attachment file mapping for sent and received media rendering.
    func upsertLocalAttachment(_ payload: LocalAttachmentPayload) {
        if let stored = storedLocalAttachment(payload.transferID) {
            stored.update(payload: payload)
        } else {
            context.insert(StoredLocalAttachment(payload: payload))
        }
        save()
    }

    /// Loads local attachment file mappings keyed by transfer id.
    func loadLocalAttachments() -> [String: LocalAttachmentPayload] {
        Dictionary(uniqueKeysWithValues: fetchAll(StoredLocalAttachment.self).map { ($0.transferID, $0.toDomain()) })
    }

    private func loadThreads() -> [ThreadSummary] {
        fetchAll(StoredThread.self).map { $0.toDomain() }
    }

    private func storedAgent(_ agentID: String) -> StoredAgent? {
        first(StoredAgent.self, #Predicate { $0.agentID == agentID })
    }

    private func storedProject(_ projectID: String) -> StoredProject? {
        first(StoredProject.self, #Predicate { $0.projectID == projectID })
    }

    private func storedThread(_ threadID: String) -> StoredThread? {
        first(StoredThread.self, #Predicate { $0.threadID == threadID })
    }

    private func storedLocalAttachment(_ transferID: String) -> StoredLocalAttachment? {
        first(StoredLocalAttachment.self, #Predicate { $0.transferID == transferID })
    }

    /// Builds the app-owned directory where attachment files are stored.
    /// - Parameters:
    ///   - fileManager: File manager used to resolve Application Support.
    /// - Returns: Directory URL for local attachment files.
    nonisolated fileprivate static func makeLocalAttachmentsDirectory(
        fileManager: FileManager = .default,
        create: Bool = true
    ) throws -> URL {
        let base = try fileManager.url(
            for: .applicationSupportDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: create
        )
        let directory = base
            .appendingPathComponent("Niuma", isDirectory: true)
            .appendingPathComponent("LocalAttachments", isDirectory: true)
        if create {
            try fileManager.createDirectory(at: directory, withIntermediateDirectories: true)
        }
        return directory
    }

    /// Resolves a stored relative path while rejecting path traversal.
    /// - Parameters:
    ///   - relativePath: Relative path stored in `StoredLocalAttachment`.
    /// - Returns: Absolute file URL inside `localAttachmentsDirectory`.
    private func localAttachmentURL(relativePath: String) -> URL? {
        let parts = relativePath.split(separator: "/").map(String.init)
        guard parts.count == 2,
              parts.allSatisfy({ !$0.isEmpty && !$0.contains("..") && !$0.contains("/") }) else {
            return nil
        }
        return localAttachmentsDirectory
            .appendingPathComponent(parts[0], isDirectory: true)
            .appendingPathComponent(parts[1], isDirectory: false)
    }

    /// Extracts transfer ids referenced by stored message content parts.
    private func transferIDs(in entries: [StoredThreadEntry]) -> Set<String> {
        let decoder = JSONDecoder()
        return entries.reduce(into: Set<String>()) { ids, entry in
            guard let data = entry.contentPartsData,
                  let parts = try? decoder.decode([ContentPart].self, from: data) else {
                return
            }
            for part in parts {
                if let transferID = part.transferID {
                    ids.insert(transferID)
                }
            }
        }
    }

    /// Deletes the SwiftData attachment row and its cached payload directory.
    private func deleteLocalAttachment(_ attachment: StoredLocalAttachment) {
        if let fileURL = localAttachmentURL(relativePath: attachment.localRelativePath) {
            try? fileManager.removeItem(at: fileURL)
            try? fileManager.removeItem(at: fileURL.deletingLastPathComponent())
        }
        context.delete(attachment)
    }

    /// Sanitizes path components used for locally cached transfer files.
    /// - Parameters:
    ///   - value: Raw path component supplied by transfer metadata.
    /// - Returns: Filesystem-safe ASCII component.
    private static func safePathComponent(_ value: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "._-"))
        let sanitized = value.unicodeScalars
            .map { allowed.contains($0) ? String($0) : "-" }
            .joined()
        let cleaned = sanitized.trimmingCharacters(in: CharacterSet(charactersIn: ".-"))
        return String((cleaned.isEmpty ? "file" : cleaned).prefix(128))
    }

    /// Builds a safe local filename while preserving the caller-provided extension when possible.
    /// - Parameters:
    ///   - fileName: Optional display filename from the attachment content part.
    /// - Returns: Safe filename for the local attachment directory.
    private static func safeFileName(_ fileName: String?) -> String {
        let lastPathComponent = URL(fileURLWithPath: fileName ?? "payload.bin").lastPathComponent
        let cleaned = safePathComponent(lastPathComponent)
        return cleaned.contains(".") ? cleaned : "\(cleaned).bin"
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

    /// Saves pending SwiftData changes and logs failures instead of crashing UI flow.
    private func save() {
        do {
            try context.save()
        } catch {
            assertionFailure("Failed to save Niuma SwiftData store: \(error)")
        }
    }
}
