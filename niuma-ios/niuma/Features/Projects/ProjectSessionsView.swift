import SwiftUI

struct ProjectSessionsView: View {
    @Environment(AppModel.self) private var appModel

    let project: ProjectSummary
    @State private var resetCandidate: ThreadSummary?
    @State private var isShowingResetConfirmation = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                VStack(alignment: .leading, spacing: 8) {
                    HStack(spacing: 14) {
                        Image(systemName: "folder")
                            .font(.system(size: 24, weight: .medium))
                            .foregroundStyle(NiumaPalette.info)
                        Text(project.projectName)
                            .font(.system(size: 28, weight: .bold))
                            .foregroundStyle(NiumaPalette.ink)
                        Spacer()
                        Button {
                            Task { await appModel.refresh() }
                        } label: {
                            Group {
                                if appModel.isRefreshing {
                                    ProgressView()
                                        .tint(NiumaPalette.ink)
                                } else {
                                    Image(systemName: "arrow.clockwise")
                                        .font(.system(size: 15, weight: .semibold))
                                }
                            }
                            .foregroundStyle(NiumaPalette.ink)
                            .frame(width: 38, height: 38)
                            .background(Circle().fill(NiumaPalette.raisedCard))
                        }
                        .buttonStyle(.plain)
                        .disabled(appModel.isRefreshing)
                    }

                    Text(project.projectID)
                        .font(.caption2.monospaced())
                        .foregroundStyle(NiumaPalette.mutedInk)
                        .textSelection(.enabled)
                }

                VStack(alignment: .leading, spacing: 12) {
                    Text("Session")
                        .font(.title3.weight(.semibold))
                        .foregroundStyle(NiumaPalette.ink)

                    if appModel.threads(for: project.projectID).isEmpty {
                        Text("当前项目还没有 session。")
                            .foregroundStyle(NiumaPalette.mutedInk)
                    } else {
                        VStack(spacing: 8) {
                            ForEach(appModel.threads(for: project.projectID)) { session in
                                SessionRow(
                                    project: project,
                                    session: session,
                                    onResetHistory: {
                                        resetCandidate = session
                                        isShowingResetConfirmation = true
                                    }
                                )
                            }
                        }
                    }
                }
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .padding(.bottom, 28)
        }
        .niumaScreenBackground()
        .navigationTitle(project.projectName)
        .navigationBarTitleDisplayMode(.inline)
        .alert("重置历史？", isPresented: $isShowingResetConfirmation) {
            Button("取消", role: .cancel) {
                resetCandidate = nil
            }
            Button("重置历史", role: .destructive) {
                if let resetCandidate {
                    appModel.resetLocalHistory(for: resetCandidate)
                }
                resetCandidate = nil
            }
        } message: {
            Text("这不会影响桌面 Codex 历史或服务端记录，只会清空本机缓存。重新进入详情会重新同步。")
        }
    }
}

private struct SessionRow: View {
    @State private var isShowingActions = false

    let project: ProjectSummary
    let session: ThreadSummary
    let onResetHistory: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            NavigationLink {
                ThreadView(project: project, session: session)
            } label: {
                SessionRowContent(session: session)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("project-session-row")

            Button {
                isShowingActions.toggle()
            } label: {
                Image(systemName: "ellipsis")
                    .font(.system(size: 13, weight: .semibold))
                    .foregroundStyle(NiumaPalette.mutedInk)
                    .frame(width: 28, height: 28)
                    .background(Circle().fill(NiumaPalette.neutralSoft))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Session 操作")
            .popover(
                isPresented: $isShowingActions,
                attachmentAnchor: .rect(.bounds),
                arrowEdge: .trailing
            ) {
                SessionActionList(
                    onResetHistory: {
                        isShowingActions = false
                        onResetHistory()
                    }
                )
                .frame(width: 146)
                .presentationCompactAdaptation(.popover)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.vertical, 14)
        .background(
            RoundedRectangle(cornerRadius: 20, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
        .contentShape(Rectangle())
    }
}

private struct SessionRowContent: View {
    @Environment(AppModel.self) private var appModel

    let session: ThreadSummary

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text(session.title)
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(NiumaPalette.ink)
                    .lineLimit(2)
                Spacer()
                if let statusBadge = session.status.compactBadge {
                    StatusBadge(title: statusBadge.0, tone: statusBadge.1)
                }
            }

            HStack(spacing: 8) {
                let pendingApprovals = appModel.pendingApprovalCount(for: session.threadID)
                if pendingApprovals > 0 {
                    StatusBadge(title: "\(pendingApprovals) 待审批", tone: .warning)
                }

                Text(DateFormatting.timeAndDate.string(from: session.updatedAt))
                    .font(.caption2)
                    .foregroundStyle(NiumaPalette.mutedInk)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct SessionActionList: View {
    let onResetHistory: () -> Void

    var body: some View {
        VStack(spacing: 0) {
            SessionActionRow(
                title: "重置历史",
                systemImage: "arrow.counterclockwise",
                role: .destructive,
                action: onResetHistory
            )
        }
        .padding(4)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(NiumaPalette.card)
                .overlay(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .stroke(NiumaPalette.border, lineWidth: 1)
                )
        )
    }
}

private struct SessionActionRow: View {
    let title: String
    let systemImage: String
    let role: ButtonRole?
    let action: () -> Void

    var body: some View {
        Button(role: role, action: action) {
            HStack(spacing: 8) {
                Image(systemName: systemImage)
                    .font(.system(size: 11, weight: .medium))
                    .frame(width: 14)
                Text(title)
                    .font(.system(size: 12, weight: .medium))
                Spacer()
            }
            .foregroundStyle(role == .destructive ? NiumaPalette.critical : NiumaPalette.ink)
            .padding(.horizontal, 7)
            .padding(.vertical, 7)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

struct BranchChangesSheet: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @State private var didRequest = false
    @State private var requestTimedOut = false

    let session: ThreadSummary

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if let result = appModel.branchChangesByThread[session.threadID] {
                        if result.succeeded, let summary = result.summary {
                            BranchChangeSummaryBlock(summary: summary)
                            ForEach(result.filesSummary, id: \.path) { file in
                                BranchFileChangeRow(file: file)
                            }
                        } else {
                            Text(result.error ?? "无法读取分支变更。")
                                .font(.footnote)
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                    } else if requestTimedOut {
                        Text("桌面 Gateway 暂未返回分支变更，请确认桌面端已运行后重试。")
                            .font(.footnote)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    } else if didRequest {
                        HStack(spacing: 8) {
                            ProgressView()
                            Text("正在读取当前 session 的分支变更。")
                                .font(.footnote)
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                    } else {
                        Text("尚未读取当前分支变更。")
                            .font(.footnote)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    }
                }
                .padding(16)
            }
            .niumaScreenBackground()
            .navigationTitle("分支变更")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("关闭") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button {
                        Task { await requestBranchChanges() }
                    } label: {
                        Image(systemName: "arrow.clockwise")
                    }
                }
            }
            .task(id: session.threadID) {
                if appModel.branchChangesByThread[session.threadID] == nil {
                    await requestBranchChanges()
                }
            }
        }
    }

    private func requestBranchChanges() async {
        didRequest = true
        requestTimedOut = false
        await appModel.refreshBranchChanges(threadID: session.threadID)
        do {
            try await Task.sleep(for: .seconds(8))
        } catch {
            return
        }
        if appModel.branchChangesByThread[session.threadID] == nil {
            requestTimedOut = true
        }
    }
}

private struct BranchChangeSummaryBlock: View {
    let summary: FileChangeBundleSummary

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text("\(summary.files) 个文件已更改")
                    .font(.headline)
                    .foregroundStyle(NiumaPalette.ink)
                Text("+\(summary.additions) -\(summary.deletions)")
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
}

private struct BranchFileChangeRow: View {
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
