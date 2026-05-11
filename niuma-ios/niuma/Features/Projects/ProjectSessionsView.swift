import SwiftUI

struct ProjectSessionsView: View {
    @Environment(AppModel.self) private var appModel

    let project: ProjectSummary
    @State private var resetCandidate: ThreadSummary?
    @State private var isShowingResetConfirmation = false
    @State private var archiveCandidate: ThreadSummary?
    @State private var isShowingArchiveConfirmation = false
    @State private var renameCandidate: ThreadSummary?
    @State private var renameDraftTitle = ""

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
                        VStack(spacing: 8) {
                            ForEach(appModel.threads(for: project.projectID)) { session in
                                SessionRow(
                                    project: project,
                                    session: session,
                                    isArchiving: appModel.archivingThreadIDs.contains(session.threadID),
                                    isRenaming: appModel.renamingThreadIDs.contains(session.threadID),
                                    onRename: {
                                        renameDraftTitle = session.title
                                        renameCandidate = session
                                    },
                                    onResetHistory: {
                                        resetCandidate = session
                                        isShowingResetConfirmation = true
                                    },
                                    onArchive: {
                                        archiveCandidate = session
                                        isShowingArchiveConfirmation = true
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
        .sheet(item: $renameCandidate) { session in
            ThreadRenameSheet(
                session: session,
                title: $renameDraftTitle,
                isSaving: appModel.renamingThreadIDs.contains(session.threadID),
                onCancel: {
                    renameCandidate = nil
                },
                onSave: { title in
                    Task {
                        await appModel.renameThread(session, title: title)
                        renameCandidate = nil
                    }
                }
            )
        }
        .alert(
            L10n.string("session.reset.confirm.title", language: appModel.appLanguage),
            isPresented: $isShowingResetConfirmation
        ) {
            Button(L10n.string("common.cancel", language: appModel.appLanguage), role: .cancel) {
                resetCandidate = nil
            }
            Button(L10n.string("session.reset.action", language: appModel.appLanguage), role: .destructive) {
                if let resetCandidate {
                    appModel.resetLocalHistory(for: resetCandidate)
                }
                resetCandidate = nil
            }
        } message: {
            Text(L10n.string("session.reset.confirm.message", language: appModel.appLanguage))
        }
        .alert(
            L10n.string("session.archive.confirm.title", language: appModel.appLanguage),
            isPresented: $isShowingArchiveConfirmation
        ) {
            Button(L10n.string("common.cancel", language: appModel.appLanguage), role: .cancel) {
                archiveCandidate = nil
            }
            Button(L10n.string("session.archive.action", language: appModel.appLanguage), role: .destructive) {
                if let archiveCandidate {
                    Task { await appModel.archiveThread(archiveCandidate) }
                }
                archiveCandidate = nil
            }
        } message: {
            Text(L10n.string("session.archive.confirm.message", language: appModel.appLanguage))
        }
    }
}

private struct SessionRow: View {
    @Environment(AppModel.self) private var appModel

    let project: ProjectSummary
    let session: ThreadSummary
    let isArchiving: Bool
    let isRenaming: Bool
    let onRename: () -> Void
    let onResetHistory: () -> Void
    let onArchive: () -> Void

    var body: some View {
        let isBusy = isArchiving || isRenaming
        HStack(alignment: .top, spacing: 10) {
            NavigationLink {
                ThreadView(project: project, session: session)
            } label: {
                SessionRowContent(session: session)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("project-session-row")

            Menu {
                Button {
                    onRename()
                } label: {
                    Label(
                        L10n.string("session.rename.action", language: appModel.appLanguage),
                        systemImage: "pencil"
                    )
                }
                Button {
                    onResetHistory()
                } label: {
                    Label(
                        L10n.string("session.reset.action", language: appModel.appLanguage),
                        systemImage: "arrow.counterclockwise"
                    )
                }
                Button(role: .destructive) {
                    onArchive()
                } label: {
                    Label(
                        L10n.string("session.archive.action", language: appModel.appLanguage),
                        systemImage: "archivebox"
                    )
                }
            } label: {
                Group {
                    if isBusy {
                        ProgressView()
                            .tint(NiumaPalette.mutedInk)
                    } else {
                        Image(systemName: "ellipsis")
                            .font(.system(size: 13, weight: .semibold))
                    }
                }
                .foregroundStyle(NiumaPalette.mutedInk)
                .frame(width: 30, height: 30)
                .background(Circle().fill(NiumaPalette.neutralSoft))
            }
            .buttonStyle(.plain)
            .disabled(isBusy)
            .accessibilityLabel(L10n.string("session.actions.accessibility", language: appModel.appLanguage))
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
                if let statusBadge = session.status.compactBadge(for: appModel.appLanguage) {
                    StatusBadge(title: statusBadge.0, tone: statusBadge.1)
                }
            }

            HStack(spacing: 8) {
                let pendingApprovals = appModel.pendingApprovalCount(for: session.threadID)
                if pendingApprovals > 0 {
                    StatusBadge(
                        title: pendingApprovalCountText(pendingApprovals),
                        tone: .warning
                    )
                }

                Text(DateFormatting.timeAndDate.string(from: session.updatedAt))
                    .font(.caption2)
                    .foregroundStyle(NiumaPalette.mutedInk)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func pendingApprovalCountText(_ count: Int) -> String {
        L10n.string(
            count == 1 ? "approval.pending.count.one" : "approval.pending.count.other",
            language: appModel.appLanguage,
            count
        )
    }
}

private struct ThreadRenameSheet: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss

    let session: ThreadSummary
    @Binding var title: String
    let isSaving: Bool
    let onCancel: () -> Void
    let onSave: (String) -> Void

    private let maxTitleLength = 80

    private var trimmedTitle: String {
        title.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var canSave: Bool {
        !trimmedTitle.isEmpty && trimmedTitle != session.title && !isSaving
    }

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    TextField(
                        L10n.string("session.rename.placeholder", language: appModel.appLanguage),
                        text: $title
                    )
                    .textInputAutocapitalization(.sentences)
                    .disabled(isSaving)
                    .onChange(of: title) { _, newValue in
                        if newValue.count > maxTitleLength {
                            title = String(newValue.prefix(maxTitleLength))
                        }
                    }
                } footer: {
                    Text(L10n.string("session.rename.message", language: appModel.appLanguage))
                }
            }
            .navigationTitle(L10n.string("session.rename.title", language: appModel.appLanguage))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.string("common.cancel", language: appModel.appLanguage)) {
                        onCancel()
                        dismiss()
                    }
                    .disabled(isSaving)
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button {
                        onSave(trimmedTitle)
                    } label: {
                        if isSaving {
                            ProgressView()
                        } else {
                            Text(L10n.string("common.save", language: appModel.appLanguage))
                        }
                    }
                    .disabled(!canSave)
                }
            }
        }
        .presentationDetents([.height(220)])
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
                    FileChangeDetailSheet(part: part, rawData: branchDetailData)
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

    private var branchDetailData: Data? {
        guard let transferID = appModel.branchChangesByThread[session.threadID]?.transferID else {
            return nil
        }
        return appModel.localAttachmentData(forTransferID: transferID)
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
            Image(systemName: "chevron.up.forward")
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
