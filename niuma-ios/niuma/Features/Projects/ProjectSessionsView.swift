import SwiftUI

struct ProjectSessionsView: View {
    @Environment(AppModel.self) private var appModel

    let project: ProjectSummary

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
                        NavigationLink {
                            NewTaskView(project: project)
                        } label: {
                            Image(systemName: "square.and.pencil")
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(NiumaPalette.ink)
                                .frame(width: 38, height: 38)
                                .background(Circle().fill(NiumaPalette.raisedCard))
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel(appModel.localized("project.new_thread.accessibility"))
                        .accessibilityIdentifier("project-new-thread-button")

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
                    Text(appModel.localized("project.sessions.title"))
                        .font(.title3.weight(.semibold))
                        .foregroundStyle(NiumaPalette.ink)

                    if appModel.threads(for: project.projectID).isEmpty {
                        Text(appModel.localized("project.sessions.empty"))
                            .foregroundStyle(NiumaPalette.mutedInk)
                    } else {
                        ThreadSessionActionsHost { actions in
                            VStack(spacing: 8) {
                                ForEach(appModel.threads(for: project.projectID)) { session in
                                    ThreadSessionRow(
                                        project: project,
                                        session: session,
                                        actions: actions,
                                        accessibilityIdentifier: "project-session-row"
                                    )
                                }
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
    }
}

struct BranchChangesSheet: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @State private var didRequest = false
    @State private var requestTimedOut = false
    @State private var isShowingBranchChangeDetail = false

    let session: ThreadSummary

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    if let result = appModel.branchChangesByThread[session.threadID] {
                        if result.succeeded, let summary = result.summary {
                            Button {
                                openBranchChangeDetail(result)
                            } label: {
                                BranchChangeSummaryBlock(summary: summary, language: appModel.appLanguage)
                            }
                            .buttonStyle(.plain)
                            ForEach(result.filesSummary, id: \.path) { file in
                                Button {
                                    openBranchChangeDetail(result)
                                } label: {
                                    BranchFileChangeRow(file: file)
                                }
                                .buttonStyle(.plain)
                            }
                        } else {
                            Text(result.error ?? appModel.localized("branch_changes.error.unavailable"))
                                .font(.footnote)
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                    } else if requestTimedOut {
                        Text(appModel.localized("branch_changes.timeout"))
                            .font(.footnote)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    } else if didRequest {
                        HStack(spacing: 8) {
                            ProgressView()
                            Text(appModel.localized("branch_changes.loading"))
                                .font(.footnote)
                                .foregroundStyle(NiumaPalette.mutedInk)
                        }
                    } else {
                        Text(appModel.localized("branch_changes.empty"))
                            .font(.footnote)
                            .foregroundStyle(NiumaPalette.mutedInk)
                    }
                }
                .padding(16)
            }
            .niumaScreenBackground()
            .navigationTitle(appModel.localized("branch_changes.title"))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button(appModel.localized("common.close")) { dismiss() }
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
            .fullScreenCover(isPresented: $isShowingBranchChangeDetail) {
                if let part = branchDetailPart {
                    FileChangeDetailSheet(part: part)
                }
            }
        }
    }

    private var branchDetailPart: ContentPart? {
        guard let result = appModel.branchChangesByThread[session.threadID],
              result.succeeded,
              let summary = result.summary else {
            return nil
        }
        return ContentPart(
            kind: .fileChangeSummary,
            transferID: result.transferID,
            fileType: "file",
            fileName: "branch-diff-bundle.json",
            mimeType: "application/json",
            sizeBytes: result.sizeBytes,
            files: summary.files,
            additions: summary.additions,
            deletions: summary.deletions,
            filesSummary: result.filesSummary
        )
    }

    private func openBranchChangeDetail(_ result: BranchChangesResult) {
        isShowingBranchChangeDetail = true
        Task { await appModel.ensureBranchChangeBundleDownloaded(result) }
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
    let language: AppLanguage

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(L10n.string("files.changed.other", language: language, summary.files))
                    .font(.headline)
                    .foregroundStyle(NiumaPalette.ink)
                Text("+\(summary.additions) -\(summary.deletions)")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(NiumaPalette.mutedInk)
            }
            Spacer()
            Image(systemName: "arrow.up.forward")
                .font(.caption.weight(.semibold))
                .foregroundStyle(NiumaPalette.mutedInk)
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
            Image(systemName: "chevron.right")
                .font(.caption.weight(.semibold))
                .foregroundStyle(NiumaPalette.mutedInk)
        }
        .padding(12)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
    }
}
