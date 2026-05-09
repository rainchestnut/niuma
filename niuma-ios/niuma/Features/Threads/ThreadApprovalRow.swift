import Foundation
import SwiftUI

struct ApprovalTimelineRow: View {
    @Environment(AppModel.self) private var appModel

    let approval: ApprovalSummary

    private var presentation: ApprovalPresentation {
        ApprovalPresentation(approval: approval, language: appModel.appLanguage)
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 0) {
            NavigationLink {
                ApprovalDetailView(approval: approval)
            } label: {
                VStack(alignment: .leading, spacing: 9) {
                    HStack(alignment: .top, spacing: 10) {
                        Image(systemName: presentation.iconName)
                            .font(.system(size: 16, weight: .semibold))
                            .foregroundStyle(NiumaPalette.warning)
                            .frame(width: 30, height: 30)
                            .background(Circle().fill(NiumaPalette.card.opacity(0.86)))

                        VStack(alignment: .leading, spacing: 4) {
                            HStack(spacing: 7) {
                                Text(L10n.string("approval.needs", language: appModel.appLanguage))
                                    .font(.footnote.weight(.semibold))
                                    .foregroundStyle(NiumaPalette.ink)
                                Text(presentation.typeTitle)
                                    .font(.caption2.weight(.semibold))
                                    .foregroundStyle(NiumaPalette.warning)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 4)
                                    .background(Capsule().fill(NiumaPalette.card.opacity(0.74)))
                            }

                            Text(presentation.actionText)
                                .font(presentation.actionMonospaced ? .footnote.monospaced() : .footnote)
                                .foregroundStyle(NiumaPalette.ink)
                                .fixedSize(horizontal: false, vertical: true)

                            if let reasonText = presentation.reasonText {
                                Text(reasonText)
                                    .font(.caption)
                                    .foregroundStyle(NiumaPalette.mutedInk)
                                    .fixedSize(horizontal: false, vertical: true)
                            }
                        }

                        Image(systemName: "chevron.right")
                            .font(.caption.weight(.bold))
                            .foregroundStyle(NiumaPalette.mutedInk)
                            .padding(.top, 6)
                    }

                    Text(DateFormatting.timeAndDate.string(from: approval.updatedAt))
                        .font(.caption2)
                        .foregroundStyle(NiumaPalette.mutedInk)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 10)
                .background(
                    RoundedRectangle(cornerRadius: 18, style: .continuous)
                        .fill(NiumaPalette.warningSoft)
                )
            }
            .buttonStyle(.plain)
            .frame(maxWidth: .infinity, alignment: .leading)

            Spacer(minLength: 34)
        }
    }
}
