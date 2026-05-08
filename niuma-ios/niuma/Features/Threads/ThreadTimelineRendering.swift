import SwiftUI

struct ThreadProcessGroup {
    let id: String
    let title: String
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

    /// Converts the current visible timeline window into stable, pre-parsed rows for display.
    ///
    /// Paging belongs to `ThreadSyncPipeline`; this layer only groups and parses
    /// the already-loaded entries so SwiftUI receives stable row identities.
    static func make(from timeline: ThreadTimeline) -> ThreadRenderSnapshot {
        return ThreadRenderSnapshot(
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

    /// Groups raw timeline entries into renderable message and process rows.
    static func group(_ entries: [ThreadEntry]) -> [ThreadRenderItem] {
        var items: [ThreadRenderItem] = []
        var processEntries: [ThreadMessageRenderItem] = []
        var pendingFinalFileChanges: [ThreadMessageRenderItem] = []

        /// Flushes accumulated process entries into one collapsed timeline row.
        func flushProcessEntries() {
            guard !processEntries.isEmpty else { return }
            items.append(
                .processGroup(
                    ThreadProcessGroup(
                        id: processGroupID(for: processEntries),
                        title: processTitle(for: processEntries),
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
            if entry.isFileChangeSummaryEntry {
                pendingFinalFileChanges.append(item)
                continue
            }

            if entry.isCodexProcessItem {
                processEntries.append(item)
                continue
            }

            flushProcessEntries()
            items.append(.message(item))
            if entry.type == "agentMessage", entry.phase == "final_answer" {
                flushPendingFileChanges()
            }
        }
        flushProcessEntries()
        flushPendingFileChanges()
        return items
    }

    private static func processGroupID(for entries: [ThreadMessageRenderItem]) -> String {
        "process-\(entries.first?.id ?? "start")-\(entries.last?.id ?? "end")-\(entries.count)"
    }

    private static func processTitle(for entries: [ThreadMessageRenderItem]) -> String {
        guard let first = entries.first?.entry.createdAt,
              let last = entries.last?.entry.createdAt else {
            return "已处理"
        }
        let seconds = max(Int(last.timeIntervalSince(first)), 0)
        guard seconds > 0 else {
            return "已处理"
        }
        if seconds >= 60 {
            return "已处理 \(seconds / 60)m \(seconds % 60)s"
        }
        return "已处理 \(seconds)s"
    }
}

private extension ThreadEntry {
    /// True when the raw Codex app-server item is process output rather than a
    /// user message or final assistant answer.
    var isCodexProcessItem: Bool {
        if type == "userMessage" {
            return false
        }
        if type == "agentMessage", phase == "final_answer" {
            return false
        }
        return true
    }

    var isFileChangeSummaryEntry: Bool {
        contentParts?.contains { $0.kind == .fileChangeSummary } == true
    }
}

struct ProcessGroupRow: View {
    let group: ThreadProcessGroup

    @State private var isExpanded = false

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                Button {
                    withAnimation(.easeInOut(duration: 0.16)) {
                        isExpanded.toggle()
                    }
                } label: {
                    HStack(spacing: 8) {
                        Text(group.title)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(NiumaPalette.mutedInk)
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(NiumaPalette.mutedInk)
                            .rotationEffect(.degrees(isExpanded ? 90 : 0))
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 9)
                    .background(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .fill(NiumaPalette.raisedCard.opacity(0.72))
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if isExpanded {
                    VStack(alignment: .leading, spacing: 10) {
                        ForEach(group.entries) { item in
                            ThreadEntryContentView(item: item)
                                .padding(.leading, 2)
                        }
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 10)
                    .background(
                        RoundedRectangle(cornerRadius: 16, style: .continuous)
                            .fill(NiumaPalette.neutralSoft.opacity(0.68))
                    )
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Spacer(minLength: 34)
        }
    }
}

struct ThreadEntryRow: View {
    let item: ThreadMessageRenderItem

    private var entry: ThreadEntry {
        item.entry
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            if entry.role == .user { Spacer(minLength: 34) }

            VStack(alignment: rowAlignment, spacing: 6) {
                if !entry.externalImageParts.isEmpty {
                    ExternalImageAttachmentStrip(
                        parts: entry.externalImageParts,
                        alignment: frameAlignment
                    )
                }

                if entry.hasBubbleContentExcludingImages {
                    VStack(alignment: .leading, spacing: 6) {
                        ThreadEntryContentView(item: item, imagePlacement: .external)

                        Text(DateFormatting.timeAndDate.string(from: entry.createdAt))
                            .font(.caption2)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    }
                    .padding(.horizontal, 11)
                    .padding(.vertical, 9)
                    .background(
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .fill(backgroundColor)
                    )
                } else {
                    Text(DateFormatting.timeAndDate.string(from: entry.createdAt))
                        .font(.caption2)
                        .foregroundStyle(NiumaPalette.mutedInk)
                }
            }
            .frame(maxWidth: .infinity, alignment: frameAlignment)

            if entry.role != .user { Spacer(minLength: 34) }
        }
    }

    private var rowAlignment: HorizontalAlignment {
        entry.role == .user ? .trailing : .leading
    }

    private var frameAlignment: Alignment {
        entry.role == .user ? .trailing : .leading
    }

    private var backgroundColor: Color {
        switch entry.role {
        case .user:
            return NiumaPalette.infoSoft
        case .assistant:
            return NiumaPalette.raisedCard
        case .system:
            return NiumaPalette.neutralSoft
        case .approval:
            return NiumaPalette.warningSoft
        }
    }
}

private extension ThreadEntry {
    var externalImageParts: [ContentPart] {
        contentParts?.filter(\.isImageFile) ?? []
    }

    var hasBubbleContentExcludingImages: Bool {
        guard let contentParts, !contentParts.isEmpty else {
            return !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        }
        return contentParts.contains { part in
            switch part.kind {
            case .text:
                return !(part.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            case .fileRef:
                return !part.isImageFile
            case .fileChangeSummary:
                return true
            }
        }
    }

}

extension ContentPart {
    var isImageFile: Bool {
        kind == .fileRef && (fileType == "image" || mimeType?.hasPrefix("image/") == true)
    }

    var fileIconName: String {
        if isImageFile {
            return "photo"
        }
        if fileType == "video" || mimeType?.hasPrefix("video/") == true {
            return "film"
        }
        return "doc"
    }

    var fileFallbackLabel: String {
        if isImageFile {
            return "图片"
        }
        if fileType == "video" || mimeType?.hasPrefix("video/") == true {
            return "视频"
        }
        return "文件"
    }
}


private struct ThreadEntryContentView: View {
    @Environment(AppModel.self) private var appModel

    let item: ThreadMessageRenderItem
    var imagePlacement: ImagePartPlacement = .inline

    @ViewBuilder
    var body: some View {
        if !item.contentParts.isEmpty {
            let renderableParts = imagePlacement == .external
                ? item.contentParts.filter { !$0.part.isImageFile }
                : item.contentParts
            if !renderableParts.isEmpty {
                VStack(alignment: .leading, spacing: 8) {
                    ForEach(renderableParts) { partItem in
                        ContentPartView(
                            item: partItem,
                            localAttachment: partItem.part.transferID.flatMap { appModel.localAttachments[$0] },
                            localData: partItem.part.transferID.flatMap { appModel.localAttachmentData(forTransferID: $0) }
                        )
                    }
                }
            }
        } else {
            MarkdownMessageText(blocks: item.fallbackMarkdownBlocks)
        }
    }
}

private enum ImagePartPlacement: Equatable {
    case inline
    case external
}
