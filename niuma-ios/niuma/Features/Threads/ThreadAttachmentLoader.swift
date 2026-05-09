import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// Converts user-selected media and imported files into outbound task attachments.
enum ThreadAttachmentLoader {
    /// Builds one attachment from PhotosUI media while preserving the broad media category.
    static func photoAttachment(from item: PhotosPickerItem, nextIndex: Int) async throws -> OutgoingAttachment? {
        guard let data = try await item.loadTransferable(type: Data.self) else { return nil }
        let type = item.supportedContentTypes.first(where: {
            $0.conforms(to: .movie) || $0.conforms(to: .image)
        }) ?? .jpeg
        let ext = type.preferredFilenameExtension ?? "jpg"
        let isVideo = type.conforms(to: .movie)
        let fileName = "\(isVideo ? "video" : "image")-\(nextIndex).\(ext)"
        let fileType = isVideo ? "video" : "image"
        return OutgoingAttachment(
            fileType: fileType,
            fileName: fileName,
            mimeType: type.preferredMIMEType ?? (isVideo ? "video/quicktime" : "image/jpeg"),
            data: data,
            alt: fileName
        )
    }

    /// Reads one imported document URL under its security-scoped access window.
    static func fileAttachment(from url: URL) throws -> OutgoingAttachment {
        let hasAccess = url.startAccessingSecurityScopedResource()
        defer {
            if hasAccess {
                url.stopAccessingSecurityScopedResource()
            }
        }
        let data = try Data(contentsOf: url)
        let resourceValues = try? url.resourceValues(forKeys: [.contentTypeKey])
        let type = resourceValues?.contentType ?? UTType(filenameExtension: url.pathExtension) ?? .data
        return OutgoingAttachment(
            fileType: attachmentFileType(for: type),
            fileName: url.lastPathComponent.isEmpty ? "attachment" : url.lastPathComponent,
            mimeType: type.preferredMIMEType ?? "application/octet-stream",
            data: data,
            alt: url.lastPathComponent
        )
    }

    /// Maps platform content types to the transfer categories understood by Gateway.
    private static func attachmentFileType(for type: UTType) -> String {
        if type.conforms(to: .movie) {
            return "video"
        }
        if type.conforms(to: .image) {
            return "image"
        }
        return "file"
    }
}
