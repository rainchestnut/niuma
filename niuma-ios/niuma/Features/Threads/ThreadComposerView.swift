import SwiftUI

struct ComposerPill: View {
    let title: String

    var body: some View {
        Text(title)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(NiumaPalette.mutedInk)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Capsule().fill(NiumaPalette.raisedCard))
    }
}

struct ComposerAttachmentStrip: View {
    let attachments: [OutgoingAttachment]
    let onRemove: (OutgoingAttachment) -> Void

    var body: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 8) {
                ForEach(attachments) { attachment in
                    HStack(spacing: 8) {
                        Image(systemName: iconName(for: attachment.fileType))
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(NiumaPalette.info)
                        Text(attachment.fileName)
                            .font(.caption2.weight(.medium))
                            .foregroundStyle(NiumaPalette.ink)
                            .lineLimit(1)
                        Button {
                            onRemove(attachment)
                        } label: {
                            Image(systemName: "xmark")
                                .font(.caption2.weight(.bold))
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                        .buttonStyle(.plain)
                    }
                    .padding(.horizontal, 10)
                    .padding(.vertical, 7)
                    .background(Capsule().fill(NiumaPalette.raisedCard.opacity(0.84)))
                }
            }
            .padding(.horizontal, 2)
        }
    }

    private func iconName(for fileType: String) -> String {
        switch fileType {
        case "image":
            return "photo"
        case "video":
            return "film"
        default:
            return "doc"
        }
    }
}
