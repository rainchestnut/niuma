import ImageIO
import SwiftUI
import UIKit

struct ExternalImageAttachmentStrip: View {
    @Environment(AppModel.self) private var appModel

    let parts: [ContentPart]
    let alignment: Alignment

    var body: some View {
        HStack(spacing: 8) {
            ForEach(Array(parts.enumerated()), id: \.offset) { _, part in
                ExternalImageAttachmentThumbnail(
                    part: part,
                    localAttachment: part.transferID.flatMap { appModel.localAttachments[$0] },
                    localData: part.transferID.flatMap { appModel.localAttachmentData(forTransferID: $0) }
                )
            }
        }
        .frame(maxWidth: .infinity, alignment: alignment)
    }
}

/// Renders image attachments outside the message bubble as a compact thumbnail
/// while keeping a comfortable tap target for opening the full-size preview.
private struct ExternalImageAttachmentThumbnail: View {
    @Environment(AppModel.self) private var appModel

    let part: ContentPart
    let localAttachment: LocalAttachmentPayload?
    let localData: Data?

    @State private var image: UIImage?
    @State private var didFail = false
    @State private var isShowingPreview = false

    private let thumbnailSize: CGFloat = 48
    private let thumbnailCornerRadius: CGFloat = 8
    private let tapTargetSize: CGFloat = 56

    var body: some View {
        Group {
            if let image {
                Button {
                    isShowingPreview = true
                } label: {
                    thumbnailImage(image)
                }
                .buttonStyle(.plain)
                .frame(width: tapTargetSize, height: tapTargetSize)
                .contentShape(Rectangle())
                .accessibilityHint(L10n.string("image.preview.view_full", language: appModel.appLanguage))
                .accessibilityIdentifier("thread-external-image-thumbnail")
            } else {
                Image(systemName: didFail ? "photo.badge.exclamationmark" : "photo")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(NiumaPalette.info)
                    .frame(width: thumbnailSize, height: thumbnailSize)
                    .background(
                        RoundedRectangle(cornerRadius: thumbnailCornerRadius, style: .continuous)
                            .fill(NiumaPalette.raisedCard)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: thumbnailCornerRadius, style: .continuous)
                            .stroke(NiumaPalette.border, lineWidth: 0.8)
                    )
                    .frame(width: tapTargetSize, height: tapTargetSize)
            }
        }
        .accessibilityLabel(part.fileName ?? part.alt ?? L10n.string("image.attachment", language: appModel.appLanguage))
        .fullScreenCover(isPresented: $isShowingPreview) {
            FullscreenImageAttachmentPreview(
                part: part,
                localAttachment: localAttachment,
                localData: localData,
                fallbackImage: image
            )
        }
        .task(id: loadIdentifier) {
            await loadImage()
        }
    }

    private func thumbnailImage(_ image: UIImage) -> some View {
        Image(uiImage: image)
            .resizable()
            .scaledToFill()
            .frame(width: thumbnailSize, height: thumbnailSize)
            .clipShape(RoundedRectangle(cornerRadius: thumbnailCornerRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: thumbnailCornerRadius, style: .continuous)
                    .stroke(NiumaPalette.border, lineWidth: 0.8)
            )
            .shadow(color: .black.opacity(0.08), radius: 3, x: 0, y: 1)
    }

    private var loadIdentifier: String {
        localAttachment?.localRelativePath ?? "empty"
    }

    @MainActor
    private func loadImage() async {
        image = nil
        didFail = false
        let localData = localData
        let decoded = await Task.detached(priority: .utility) {
            if let data = localData {
                return DataURLImageDecoder.decodeData(data, maxPixelDimension: 480)
            }
            return nil
        }.value
        guard !Task.isCancelled else { return }
        if let decoded {
            image = UIImage(cgImage: decoded)
        } else {
            didFail = true
        }
    }
}

/// Full-screen image viewer used by the detached thread attachment thumbnail.
private struct FullscreenImageAttachmentPreview: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss

    let part: ContentPart
    let localAttachment: LocalAttachmentPayload?
    let localData: Data?
    let fallbackImage: UIImage?

    @State private var image: UIImage?
    @State private var didFail = false

    var body: some View {
        ZStack(alignment: .topTrailing) {
            Color.black.ignoresSafeArea()

            Group {
                if let image = image ?? fallbackImage {
                    Image(uiImage: image)
                        .resizable()
                        .scaledToFit()
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                        .accessibilityLabel(part.fileName ?? part.alt ?? L10n.string("image.preview.full", language: appModel.appLanguage))
                        .accessibilityIdentifier("thread-fullscreen-image-preview")
                } else if didFail {
                    ContentUnavailableView(
                        L10n.string("image.preview.unavailable", language: appModel.appLanguage),
                        systemImage: "photo.badge.exclamationmark"
                    )
                        .foregroundStyle(.white)
                        .accessibilityIdentifier("thread-fullscreen-image-preview")
                } else {
                    ProgressView()
                        .tint(.white)
                        .controlSize(.large)
                        .accessibilityIdentifier("thread-fullscreen-image-preview")
                }
            }
            .padding(.horizontal, 16)

            Button {
                dismiss()
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 42, height: 42)
                    .background(.black.opacity(0.48), in: Circle())
            }
            .accessibilityLabel(L10n.string("image.preview.close", language: appModel.appLanguage))
            .accessibilityIdentifier("thread-fullscreen-image-close")
            .padding(.top, 18)
            .padding(.trailing, 16)
        }
        .task(id: loadIdentifier) {
            await loadImage()
        }
    }

    private var loadIdentifier: String {
        localAttachment?.localRelativePath ?? "empty"
    }

    @MainActor
    private func loadImage() async {
        image = nil
        didFail = false
        let localData = localData
        let decoded = await Task.detached(priority: .utility) {
            if let data = localData {
                return DataURLImageDecoder.decodeData(data, maxPixelDimension: 3_000)
            }
            return nil
        }.value
        guard !Task.isCancelled else { return }
        if let decoded {
            image = UIImage(cgImage: decoded)
        } else if fallbackImage == nil {
            didFail = true
        }
    }
}

struct ContentPartView: View {
    @Environment(AppModel.self) private var appModel

    let item: ContentPartRenderItem
    let localAttachment: LocalAttachmentPayload?
    let localData: Data?

    @State private var isShowingFileChangeDetail = false

    private var part: ContentPart {
        item.part
    }

    var body: some View {
        switch part.kind {
        case .text:
            MarkdownMessageText(blocks: item.markdownBlocks)
        case .fileRef:
            if part.isImageFile, localData != nil {
                InlineImagePartView(
                    rawData: localData,
                    title: part.fileName ?? part.alt ?? part.fileFallbackLabel(for: appModel.appLanguage),
                    subtitle: attachmentSubtitle(fallback: part.mimeType ?? "image"),
                    language: appModel.appLanguage
                )
            } else {
                let fallbackLabel = part.fileFallbackLabel(for: appModel.appLanguage)
                AttachmentPartCard(
                    iconName: part.fileIconName,
                    title: part.fileName ?? part.alt ?? fallbackLabel,
                    subtitle: attachmentSubtitle(
                        fallback: part.transferID.map { "transfer \($0)" } ?? (part.mimeType ?? fallbackLabel)
                    )
                )
            }
        case .fileChangeSummary:
            FileChangeSummaryCard(
                part: part,
                hasDetail: localData != nil,
                openDetail: { isShowingFileChangeDetail = true }
            )
            .fullScreenCover(isPresented: $isShowingFileChangeDetail) {
                FileChangeDetailSheet(part: part, rawData: localData)
            }
        }
    }

    /// Builds a short attachment status string from the unified local attachment cache.
    private func attachmentSubtitle(fallback: String) -> String {
        guard let localAttachment else { return fallback }
        let label = L10n.transferDirectionLabel(localAttachment.direction, language: appModel.appLanguage)
        if let sizeBytes = localAttachment.sizeBytes {
            return "\(label) \(ByteCountFormatter.string(fromByteCount: Int64(sizeBytes), countStyle: .file))"
        }
        return label
    }
}

private struct FileChangeSummaryCard: View {
    @Environment(AppModel.self) private var appModel

    let part: ContentPart
    let hasDetail: Bool
    let openDetail: () -> Void

    var body: some View {
        Button(action: openDetail) {
            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 10) {
                    Image(systemName: "doc.text.magnifyingglass")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(NiumaPalette.info)
                        .frame(width: 30, height: 30)
                        .background(Circle().fill(NiumaPalette.infoSoft))

                    VStack(alignment: .leading, spacing: 2) {
                        Text(filesChangedText(part.files ?? 0))
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(NiumaPalette.ink)
                        HStack(spacing: 8) {
                            Text("+\(part.additions ?? 0)")
                                .foregroundStyle(.green)
                            Text("-\(part.deletions ?? 0)")
                                .foregroundStyle(.red)
                            if !hasDetail {
                                Text(L10n.string("details.syncing", language: appModel.appLanguage))
                                    .foregroundStyle(NiumaPalette.mutedInk)
                            }
                        }
                        .font(.caption2.weight(.semibold))
                    }

                    Spacer(minLength: 0)
                    Image(systemName: "arrow.up.forward")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(NiumaPalette.mutedInk)
                }

                VStack(alignment: .leading, spacing: 6) {
                    ForEach(summaryRows.prefix(4), id: \.path) { file in
                        HStack(spacing: 8) {
                            Text(file.path)
                                .font(.caption2)
                                .foregroundStyle(NiumaPalette.ink)
                                .lineLimit(1)
                            Spacer(minLength: 8)
                            Text("+\(file.additions)")
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.green)
                            Text("-\(file.deletions)")
                                .font(.caption2.weight(.semibold))
                                .foregroundStyle(.red)
                        }
                    }
                    if summaryRows.count > 4 {
                        Text(moreFilesText(summaryRows.count - 4))
                            .font(.caption2)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    }
                }
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 9)
            .background(
                RoundedRectangle(cornerRadius: 14, style: .continuous)
                    .fill(NiumaPalette.canvas.opacity(0.52))
            )
        }
        .buttonStyle(.plain)
    }

    private var summaryRows: [FileChangeFileSummary] {
        part.filesSummary ?? []
    }

    private func filesChangedText(_ count: Int) -> String {
        L10n.string(count == 1 ? "files.changed.one" : "files.changed.other", language: appModel.appLanguage, count)
    }

    private func moreFilesText(_ count: Int) -> String {
        L10n.string(count == 1 ? "files.more.one" : "files.more.other", language: appModel.appLanguage, count)
    }
}

struct FileChangeDetailSheet: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss

    let part: ContentPart
    let rawData: Data?

    private let decoder = JSONDecoder()

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    FileChangeSummaryHeader(part: part)
                    if let bundle {
                        ForEach(bundle.files) { file in
                            FileChangeDiffFileView(file: file)
                        }
                    } else {
                        ForEach(part.filesSummary ?? [], id: \.path) { file in
                            FileChangeFallbackFileRow(file: file)
                        }
                        if rawData == nil {
                            Text(L10n.string("details.syncing.retry", language: appModel.appLanguage))
                                .font(.footnote)
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                    }
                }
                .padding(16)
            }
            .niumaScreenBackground()
            .navigationTitle(L10n.string("file_changes.title", language: appModel.appLanguage))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarTrailing) {
                    Button(L10n.string("common.done", language: appModel.appLanguage)) { dismiss() }
                }
            }
        }
    }

    private var bundle: FileChangeDiffBundle? {
        guard let rawData else { return nil }
        return try? decoder.decode(FileChangeDiffBundle.self, from: rawData)
    }
}

private struct FileChangeSummaryHeader: View {
    @Environment(AppModel.self) private var appModel

    let part: ContentPart

    var body: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 4) {
                Text(filesChangedText(part.files ?? 0))
                    .font(.headline)
                    .foregroundStyle(NiumaPalette.ink)
                Text("+\(part.additions ?? 0) -\(part.deletions ?? 0)")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(NiumaPalette.mutedInk)
            }
            Spacer()
        }
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
    }

    private func filesChangedText(_ count: Int) -> String {
        L10n.string(count == 1 ? "files.changed.one" : "files.changed.other", language: appModel.appLanguage, count)
    }
}

private struct FileChangeFallbackFileRow: View {
    let file: FileChangeFileSummary

    var body: some View {
        HStack(spacing: 8) {
            Text(file.path)
                .font(.footnote.weight(.semibold))
                .foregroundStyle(NiumaPalette.ink)
                .lineLimit(2)
            Spacer(minLength: 8)
            Text("+\(file.additions)")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.green)
            Text("-\(file.deletions)")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.red)
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
    }
}

private struct FileChangeDiffFileView: View {
    let file: FileChangeDiffFile

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Text(file.path)
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                    .lineLimit(2)
                Spacer(minLength: 8)
                Text("+\(file.additions)")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.green)
                Text("-\(file.deletions)")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(.red)
            }

            if file.hunks.isEmpty {
                RawDiffText(rawDiff: file.rawDiff)
            } else {
                ForEach(file.hunks) { hunk in
                    VStack(alignment: .leading, spacing: 0) {
                        Text("@@ -\(hunk.oldStart),\(hunk.oldLines) +\(hunk.newStart),\(hunk.newLines) @@")
                            .font(.caption2.monospaced())
                            .foregroundStyle(NiumaPalette.mutedInk)
                            .padding(.horizontal, 8)
                            .padding(.vertical, 6)
                        ForEach(hunk.lines) { line in
                            DiffLineRow(line: line)
                        }
                    }
                    .clipShape(RoundedRectangle(cornerRadius: 10, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 10, style: .continuous)
                            .stroke(NiumaPalette.border, lineWidth: 0.8)
                    )
                }
            }
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
    }
}

private struct DiffLineRow: View {
    let line: FileChangeDiffLine

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 8) {
            Text(lineNumberText)
                .font(.caption2.monospaced())
                .foregroundStyle(NiumaPalette.mutedInk)
                .frame(width: 58, alignment: .trailing)
            Text(prefix)
                .font(.caption2.monospaced().weight(.bold))
                .foregroundStyle(prefixColor)
                .frame(width: 10)
            Text(line.content)
                .font(.caption2.monospaced())
                .foregroundStyle(NiumaPalette.ink)
                .frame(maxWidth: .infinity, alignment: .leading)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 3)
        .background(rowColor)
    }

    private var lineNumberText: String {
        let old = line.oldLine.map(String.init) ?? "-"
        let new = line.newLine.map(String.init) ?? "-"
        return "\(old) \(new)"
    }

    private var prefix: String {
        switch line.kind {
        case "add": return "+"
        case "delete": return "-"
        default: return " "
        }
    }

    private var prefixColor: Color {
        switch line.kind {
        case "add": return .green
        case "delete": return .red
        default: return NiumaPalette.mutedInk
        }
    }

    private var rowColor: Color {
        switch line.kind {
        case "add": return Color.green.opacity(0.12)
        case "delete": return Color.red.opacity(0.12)
        default: return Color.clear
        }
    }
}

private struct RawDiffText: View {
    let rawDiff: String

    var body: some View {
        ScrollView(.horizontal, showsIndicators: true) {
            Text(rawDiff)
                .font(.caption2.monospaced())
                .foregroundStyle(NiumaPalette.ink)
                .padding(10)
        }
        .background(
            RoundedRectangle(cornerRadius: 10, style: .continuous)
                .fill(NiumaPalette.neutralSoft.opacity(0.6))
        )
    }
}

private struct AttachmentPartCard: View {
    let iconName: String
    let title: String
    let subtitle: String

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: iconName)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(NiumaPalette.info)
                .frame(width: 30, height: 30)
                .background(Circle().fill(NiumaPalette.infoSoft))
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                    .lineLimit(1)
                Text(subtitle)
                    .font(.caption2)
                    .foregroundStyle(NiumaPalette.mutedInk)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: 14, style: .continuous)
                .fill(NiumaPalette.canvas.opacity(0.52))
        )
    }
}

private struct InlineImagePartView: View {
    let rawData: Data?
    let title: String
    let subtitle: String
    let language: AppLanguage

    @State private var image: UIImage?
    @State private var didFail = false

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
                    .clipShape(RoundedRectangle(cornerRadius: 14, style: .continuous))
                    .overlay(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .stroke(NiumaPalette.border, lineWidth: 0.8)
                    )
            } else {
                AttachmentPartCard(
                    iconName: "photo",
                    title: title,
                    subtitle: didFail ? subtitle : L10n.string("attachment.loading", language: language, subtitle)
                )
            }
        }
        .task(id: loadIdentifier) {
            await loadImage()
        }
    }

    private var loadIdentifier: String {
        rawData.map { "raw-\($0.count)-\($0.hashValue)" } ?? "empty"
    }

    @MainActor
    private func loadImage() async {
        image = nil
        didFail = false
        guard rawData != nil else {
            didFail = true
            return
        }

        let decoded = await Task.detached(priority: .utility) {
            if let rawData {
                return DataURLImageDecoder.decodeData(rawData, maxPixelDimension: 1_600)
            }
            return nil
        }.value
        guard !Task.isCancelled else { return }
        if let decoded {
            image = UIImage(cgImage: decoded)
        } else {
            didFail = true
        }
    }
}

private enum DataURLImageDecoder {
    nonisolated static func decodeData(_ data: Data, maxPixelDimension: CGFloat) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil) else {
            return nil
        }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: max(1, Int(maxPixelDimension))
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}
