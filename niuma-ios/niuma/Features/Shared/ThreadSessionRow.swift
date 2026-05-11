import SwiftUI

/// Shared session-list row and action plumbing for every visible thread list.
struct ThreadSessionActionsHost<Content: View>: View {
    @Environment(AppModel.self) private var appModel

    private let content: (ThreadSessionActions) -> Content

    @State private var resetCandidate: ThreadSummary?
    @State private var archiveCandidate: ThreadSummary?
    @State private var renameCandidate: ThreadSummary?
    @State private var renameDraftTitle = ""

    init(@ViewBuilder content: @escaping (ThreadSessionActions) -> Content) {
        self.content = content
    }

    var body: some View {
        content(
            ThreadSessionActions(
                rename: beginRename,
                resetHistory: beginResetHistory,
                archive: beginArchive
            )
        )
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
            isPresented: resetConfirmationBinding
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
            isPresented: archiveConfirmationBinding
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

    private var resetConfirmationBinding: Binding<Bool> {
        Binding(
            get: { resetCandidate != nil },
            set: { isPresented in
                if !isPresented {
                    resetCandidate = nil
                }
            }
        )
    }

    private var archiveConfirmationBinding: Binding<Bool> {
        Binding(
            get: { archiveCandidate != nil },
            set: { isPresented in
                if !isPresented {
                    archiveCandidate = nil
                }
            }
        )
    }

    private func beginRename(_ session: ThreadSummary) {
        renameDraftTitle = session.title
        renameCandidate = session
    }

    private func beginResetHistory(_ session: ThreadSummary) {
        resetCandidate = session
    }

    private func beginArchive(_ session: ThreadSummary) {
        archiveCandidate = session
    }
}

/// User-visible actions that every session-list row must expose consistently.
struct ThreadSessionActions {
    let rename: (ThreadSummary) -> Void
    let resetHistory: (ThreadSummary) -> Void
    let archive: (ThreadSummary) -> Void
}

/// Reusable row for project sessions and no-project conversation sessions.
struct ThreadSessionRow: View {
    @Environment(AppModel.self) private var appModel

    let project: ProjectSummary
    let session: ThreadSummary
    let actions: ThreadSessionActions
    let accessibilityIdentifier: String

    init(
        project: ProjectSummary,
        session: ThreadSummary,
        actions: ThreadSessionActions,
        accessibilityIdentifier: String = "thread-session-row"
    ) {
        self.project = project
        self.session = session
        self.actions = actions
        self.accessibilityIdentifier = accessibilityIdentifier
    }

    var body: some View {
        let isBusy = appModel.archivingThreadIDs.contains(session.threadID)
            || appModel.renamingThreadIDs.contains(session.threadID)

        HStack(alignment: .top, spacing: 10) {
            NavigationLink {
                ThreadView(project: project, session: session)
            } label: {
                ThreadSessionRowContent(session: session)
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier(accessibilityIdentifier)

            ThreadSessionActionsMenu(
                session: session,
                isBusy: isBusy,
                actions: actions
            )
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

private struct ThreadSessionActionsMenu: View {
    @Environment(AppModel.self) private var appModel

    let session: ThreadSummary
    let isBusy: Bool
    let actions: ThreadSessionActions

    var body: some View {
        Menu {
            Button {
                actions.rename(session)
            } label: {
                Label(
                    L10n.string("session.rename.action", language: appModel.appLanguage),
                    systemImage: "pencil"
                )
            }
            Button {
                actions.resetHistory(session)
            } label: {
                Label(
                    L10n.string("session.reset.action", language: appModel.appLanguage),
                    systemImage: "arrow.counterclockwise"
                )
            }
            Button(role: .destructive) {
                actions.archive(session)
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
}

private struct ThreadSessionRowContent: View {
    @Environment(AppModel.self) private var appModel

    let session: ThreadSummary

    private var pendingApprovals: Int {
        appModel.pendingApprovalCount(for: session.threadID)
    }

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
