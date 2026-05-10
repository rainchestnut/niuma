import Foundation

extension AppModel {
    func insertTransientUserPrompt(
        threadID: String,
        projectID: String,
        agentID: String,
        prompt: String,
        contentParts: [ContentPart]
    ) -> (threadID: String, entryID: String) {
        let timeline = timelines[threadID] ?? ThreadTimeline(threadID: threadID, ackSeq: 0, checkpoint: nil, entries: [])
        let nextSeq = max(timeline.ackSeq, timeline.entries.map(\.seq).max() ?? 0) + 1
        let entryID = "transient-\(UUID().uuidString.lowercased())"
        let entry = ThreadEntry(
            id: entryID,
            role: .user,
            type: "userMessage",
            text: contentPartsTextPreview(contentParts, fallback: prompt),
            createdAt: .now,
            seq: nextSeq,
            contentParts: contentParts.isEmpty ? nil : contentParts
        )
        transientOutgoingEntries[threadID, default: []].append(entry)
        upsertThreadSummary(
            ThreadSummary(
                threadID: threadID,
                projectID: projectID,
                agentID: agentID,
                title: prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? threadID : prompt,
                status: .running,
                lastCheckpointSeen: timeline.checkpoint,
                currentBranch: threadSummary(for: threadID)?.currentBranch,
                updatedAt: .now
            )
        )
        return (threadID, entryID)
    }

    /// Removes a local optimistic user prompt after a matching desktop-confirmed entry arrives.
    /// - Returns: Removed local-only entry ids so any already-copied UI timeline can be cleaned.
    func removeTransientUserPrompt(threadID: String, matching canonicalEntry: ThreadEntry) -> Set<String> {
        guard canonicalEntry.role == .user,
              var candidates = transientOutgoingEntries[threadID],
              !candidates.isEmpty else {
            return []
        }
        var removedIDs: Set<String> = []
        candidates.removeAll { candidate in
            let shouldRemove = candidate.matchesCanonicalUserPrompt(canonicalEntry)
            if shouldRemove {
                removedIDs.insert(candidate.id)
            }
            return shouldRemove
        }
        if candidates.isEmpty {
            transientOutgoingEntries.removeValue(forKey: threadID)
        } else {
            transientOutgoingEntries[threadID] = candidates
        }
        return removedIDs
    }

    /// Removes local-only user prompts superseded by any canonical Codex user row.
    func removeTransientUserPrompts(threadID: String, matching canonicalEntries: [ThreadEntry]) {
        for canonicalEntry in canonicalEntries where canonicalEntry.role == .user {
            _ = removeTransientUserPrompt(threadID: threadID, matching: canonicalEntry)
        }
    }

    /// Removes one local optimistic user prompt when sending fails before desktop receives it.
    func removeTransientUserPrompt(threadID: String, entryID: String) {
        transientOutgoingEntries[threadID]?.removeAll { $0.id == entryID }
        if transientOutgoingEntries[threadID]?.isEmpty == true {
            transientOutgoingEntries.removeValue(forKey: threadID)
        }
    }

    /// Persists the model selected from the mobile UI.
    func selectModel(_ modelID: String) {
        selectedModelID = modelID
        storage.set(modelID, forKey: StorageKey.selectedModelID.rawValue)
    }

    /// Persists the reasoning effort selected from the shared composer menu.
    func selectReasoningEffort(_ effort: ReasoningEffort) {
        selectedReasoningEffort = effort
        storage.set(effort.rawValue, forKey: StorageKey.selectedReasoningEffort.rawValue)
    }

    /// Applies the desktop app-server model list and current model to mobile state.
    func applyModelSync(_ state: ModelSyncState) {
        availableModels = state.availableModels
        if let current = state.currentModel, !current.isEmpty {
            selectedModelID = current
            storage.set(current, forKey: StorageKey.selectedModelID.rawValue)
        } else if selectedModelID == nil, let first = state.availableModels.first {
            selectedModelID = first
            storage.set(first, forKey: StorageKey.selectedModelID.rawValue)
        }
    }

    /// Builds the visible fallback text for rich content parts in optimistic UI.
    func contentPartsTextPreview(_ parts: [ContentPart], fallback: String) -> String {
        var lines: [String] = []
        let trimmedFallback = fallback.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmedFallback.isEmpty {
            lines.append(trimmedFallback)
        }
        for part in parts where part.kind != .text {
            let name = part.fileName ?? part.alt ?? part.transferID ?? "attachment"
            switch part.kind {
            case .fileRef:
                lines.append("[\(fileLabel(for: part)): \(name)]")
            case .text:
                break
            case .fileChangeSummary:
                lines.append(
                    L10n.string(
                        "content_part.file_changes.summary",
                        language: appLanguage,
                        part.files ?? 0,
                        part.additions ?? 0,
                        part.deletions ?? 0
                    )
                )
            }
        }
        return lines.joined(separator: "\n")
    }

    func fileLabel(for part: ContentPart) -> String {
        if part.fileType == "image" || part.mimeType?.hasPrefix("image/") == true {
            return L10n.string("file.kind.image", language: appLanguage)
        }
        if part.fileType == "video" || part.mimeType?.hasPrefix("video/") == true {
            return L10n.string("file.kind.video", language: appLanguage)
        }
        return L10n.string("file.kind.file", language: appLanguage)
    }

    /// Updates the in-memory thread index used by visible session lists.
}
