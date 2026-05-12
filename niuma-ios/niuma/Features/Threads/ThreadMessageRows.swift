import Foundation
import SwiftUI

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

                        if let createdAt = entry.createdAt {
                            Text(DateFormatting.timeAndDate.string(from: createdAt))
                                .font(.caption2)
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                    }
                    .padding(.horizontal, 11)
                    .padding(.vertical, 9)
                    .background(
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .fill(backgroundColor)
                    )
                } else {
                    if let createdAt = entry.createdAt {
                        Text(DateFormatting.timeAndDate.string(from: createdAt))
                            .font(.caption2)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    }
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

struct ThreadEntryContentView: View {
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

enum ImagePartPlacement: Equatable {
    case inline
    case external
}

extension ThreadEntry {
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
    nonisolated var isImageFile: Bool {
        kind == .fileRef && (fileType == "image" || mimeType?.hasPrefix("image/") == true)
    }

    nonisolated var fileIconName: String {
        if isImageFile {
            return "photo"
        }
        if fileType == "video" || mimeType?.hasPrefix("video/") == true {
            return "film"
        }
        return "doc"
    }

    nonisolated func fileFallbackLabel(for language: AppLanguage) -> String {
        L10n.fileFallbackLabel(for: self, language: language)
    }
}
