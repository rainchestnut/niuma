import Foundation

struct ThreadProcessGroup {
    let id: String
    let durationSeconds: Int?
    let entries: [ThreadMessageRenderItem]
}

struct ThreadRenderSnapshot {
    let threadID: String
    let entryCount: Int
    let lastEntryID: String?
    let items: [ThreadRenderItem]

    static func empty(threadID: String) -> ThreadRenderSnapshot {
        ThreadRenderSnapshot(
            threadID: threadID,
            entryCount: 0,
            lastEntryID: nil,
            items: []
        )
    }

    /// Converts loaded thread entries into stable rows before SwiftUI renders them.
    static func make(from timeline: ThreadTimeline) -> ThreadRenderSnapshot {
        ThreadRenderSnapshot(
            threadID: timeline.threadID,
            entryCount: timeline.entries.count,
            lastEntryID: timeline.entries.last?.id,
            items: ThreadRenderItem.group(timeline.entries)
        )
    }
}

struct ThreadMessageRenderItem: Identifiable {
    let id: String
    let entry: ThreadEntry
    let fallbackMarkdownBlocks: [MarkdownBlock]
    let contentParts: [ContentPartRenderItem]

    init(entry: ThreadEntry) {
        self.id = entry.id
        self.entry = entry
        if let contentParts = entry.contentParts, !contentParts.isEmpty {
            self.contentParts = contentParts.enumerated().map { offset, part in
                ContentPartRenderItem(
                    id: "\(entry.id)-part-\(offset)-\(part.kind.rawValue)",
                    part: part,
                    idPrefix: "\(entry.id)-part-\(offset)"
                )
            }
            self.fallbackMarkdownBlocks = []
        } else {
            self.contentParts = []
            self.fallbackMarkdownBlocks = MarkdownBlock.parse(entry.text, idPrefix: "\(entry.id)-text")
        }
    }
}

struct ContentPartRenderItem: Identifiable {
    let id: String
    let part: ContentPart
    let markdownBlocks: [MarkdownBlock]

    init(id: String, part: ContentPart, idPrefix: String) {
        self.id = id
        self.part = part
        self.markdownBlocks = part.kind == .text
            ? MarkdownBlock.parse(part.text ?? "", idPrefix: idPrefix)
            : []
    }
}

enum ThreadRenderItem: Identifiable {
    case processGroup(ThreadProcessGroup)
    case message(ThreadMessageRenderItem)

    var id: String {
        switch self {
        case .processGroup(let group):
            return group.id
        case .message(let item):
            return item.id
        }
    }

    static func group(_ entries: [ThreadEntry]) -> [ThreadRenderItem] {
        var items: [ThreadRenderItem] = []
        var processEntries: [ThreadMessageRenderItem] = []
        var pendingFinalFileChanges: [ThreadMessageRenderItem] = []

        func flushProcessEntries() {
            guard !processEntries.isEmpty else { return }
            items.append(
                .processGroup(
                    ThreadProcessGroup(
                        id: processGroupID(for: processEntries),
                        durationSeconds: processDurationSeconds(for: processEntries),
                        entries: processEntries
                    )
                )
            )
            processEntries.removeAll(keepingCapacity: true)
        }

        func flushPendingFileChanges() {
            guard !pendingFinalFileChanges.isEmpty else { return }
            for entry in pendingFinalFileChanges {
                items.append(.message(entry))
            }
            pendingFinalFileChanges.removeAll(keepingCapacity: true)
        }

        for entry in entries {
            let item = ThreadMessageRenderItem(entry: entry)
            switch entry.timelineKind {
            case .fileChangeSummary:
                pendingFinalFileChanges.append(item)
            case .process:
                processEntries.append(item)
            case .message:
                flushProcessEntries()
                items.append(.message(item))
                if entry.endsAssistantTurn {
                    flushPendingFileChanges()
                }
            }
        }
        flushProcessEntries()
        flushPendingFileChanges()
        return items
    }

    private static func processGroupID(for entries: [ThreadMessageRenderItem]) -> String {
        "process-\(entries.first?.id ?? "start")-\(entries.last?.id ?? "end")-\(entries.count)"
    }

    private static func processDurationSeconds(for entries: [ThreadMessageRenderItem]) -> Int? {
        guard let first = entries.first?.entry.createdAt,
              let last = entries.last?.entry.createdAt else {
            return nil
        }
        let seconds = max(Int(last.timeIntervalSince(first)), 0)
        return seconds > 0 ? seconds : nil
    }
}

enum ThreadTimelineRow: Identifiable {
    case processGroup(ThreadProcessGroup)
    case message(ThreadMessageRenderItem)
    case approval(ApprovalSummary)
    case userInput(UserInputRequestSummary)

    var id: String {
        switch self {
        case .processGroup(let group):
            return group.id
        case .message(let item):
            return item.id
        case .approval(let approval):
            return "approval-\(approval.approvalID)"
        case .userInput(let request):
            return "user-input-\(request.requestID)"
        }
    }

    /// Merges blocking interaction prompts into the same chronological stream as messages.
    static func merge(
        items: [ThreadRenderItem],
        approvals: [ApprovalSummary],
        userInputs: [UserInputRequestSummary]
    ) -> [ThreadTimelineRow] {
        var sortItems: [ThreadTimelineSortItem] = items.enumerated().map { offset, item in
            let row = ThreadTimelineRow(item)
            return ThreadTimelineSortItem(
                row: row,
                date: row.sortDate,
                priority: row.sortPriority,
                sourceOffset: offset
            )
        }

        sortItems.append(
            contentsOf: approvals.enumerated().map { offset, approval in
                let row = ThreadTimelineRow.approval(approval)
                return ThreadTimelineSortItem(
                    row: row,
                    date: row.sortDate,
                    priority: row.sortPriority,
                    sourceOffset: items.count + offset
                )
            }
        )

        sortItems.append(
            contentsOf: userInputs.enumerated().map { offset, request in
                let row = ThreadTimelineRow.userInput(request)
                return ThreadTimelineSortItem(
                    row: row,
                    date: row.sortDate,
                    priority: row.sortPriority,
                    sourceOffset: items.count + approvals.count + offset
                )
            }
        )

        return sortItems.sorted { left, right in
            if left.date != right.date {
                return left.date < right.date
            }
            if left.priority != right.priority {
                return left.priority < right.priority
            }
            return left.sourceOffset < right.sourceOffset
        }
        .map(\.row)
    }

    private init(_ item: ThreadRenderItem) {
        switch item {
        case .processGroup(let group):
            self = .processGroup(group)
        case .message(let item):
            self = .message(item)
        }
    }

    private var sortDate: Date {
        switch self {
        case .processGroup(let group):
            return group.entries.last?.entry.createdAt ?? .distantPast
        case .message(let item):
            return item.entry.createdAt
        case .approval(let approval):
            return approval.updatedAt
        case .userInput(let request):
            return request.updatedAt
        }
    }

    private var sortPriority: Int {
        switch self {
        case .processGroup, .message:
            return 0
        case .approval, .userInput:
            return 1
        }
    }
}

private enum ThreadEntryTimelineKind {
    case message
    case process
    case fileChangeSummary
}

private struct ThreadTimelineSortItem {
    let row: ThreadTimelineRow
    let date: Date
    let priority: Int
    let sourceOffset: Int
}

private extension ThreadEntry {
    var timelineKind: ThreadEntryTimelineKind {
        if isFileChangeSummaryEntry {
            return .fileChangeSummary
        }
        if role == .user || type == "userMessage" {
            return .message
        }
        if endsAssistantTurn {
            return .message
        }
        return .process
    }

    var endsAssistantTurn: Bool {
        type == "agentMessage" && phase == "final_answer"
    }

    var isFileChangeSummaryEntry: Bool {
        contentParts?.contains { $0.kind == .fileChangeSummary } == true
    }
}
