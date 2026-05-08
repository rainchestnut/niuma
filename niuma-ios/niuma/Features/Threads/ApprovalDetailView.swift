import SwiftUI

/// Detail screen for an approval request attached to a thread timeline.
struct ApprovalDetailView: View {
    @Environment(AppModel.self) private var appModel
    let approval: ApprovalSummary

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                SurfaceCard(title: "审批信息") {
                    VStack(alignment: .leading, spacing: 12) {
                        detailRow(title: "审批 ID", value: approval.approvalID, monospaced: true)
                        detailRow(title: "线程 ID", value: approval.threadID, monospaced: true)
                        detailRow(title: "类型", value: approval.approvalType)
                        if let requestMethod = approval.requestMethod {
                            detailRow(title: "方法", value: requestMethod, monospaced: true)
                        }
                    }
                }

                if let paramsJSON = approval.paramsJSON, !paramsJSON.isEmpty {
                    SurfaceCard(title: "参数") {
                        ScrollView(.horizontal, showsIndicators: false) {
                            Text(paramsJSON)
                                .font(.footnote.monospaced())
                                .foregroundStyle(NiumaPalette.ink)
                                .textSelection(.enabled)
                                .frame(maxWidth: .infinity, alignment: .leading)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }

                SurfaceCard(title: "操作") {
                    VStack(spacing: 12) {
                        Button("Allow") {
                            Task {
                                await appModel.resolveApproval(approval, decision: .allow)
                            }
                        }
                        .buttonStyle(NiumaPrimaryButtonStyle())

                        Button("Reject", role: .destructive) {
                            Task {
                                await appModel.resolveApproval(approval, decision: .reject)
                            }
                        }
                        .font(.headline)
                        .frame(maxWidth: .infinity)
                        .padding(.horizontal, 18)
                        .padding(.vertical, 14)
                        .background(NiumaPalette.criticalSoft, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
                        .foregroundStyle(NiumaPalette.critical)
                    }
                }
            }
            .padding()
        }
        .niumaScreenBackground()
        .navigationTitle("审批详情")
    }

    private func detailRow(title: String, value: String, monospaced: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption.weight(.semibold))
                .foregroundStyle(NiumaPalette.mutedInk)
            Text(value)
                .font(monospaced ? .footnote.monospaced() : .body)
                .foregroundStyle(NiumaPalette.ink)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(NiumaPalette.border, lineWidth: 1)
        )
    }
}
