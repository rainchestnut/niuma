import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// Thread detail screen that keeps the newest streamed content in view while
/// offering a compact, glass-styled control surface for refresh and input.
struct ThreadView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @Environment(\.scenePhase) private var scenePhase

    let project: ProjectSummary
    let session: ThreadSummary

    @State private var prompt = ""
    @State private var hasRequestedInitialLoad = false
    @State private var isLoadingHistory = false
    @State private var renderedTimeline = ThreadRenderSnapshot.empty(threadID: "")
    @State private var hasScrolledAfterInitialRender = false
    @State private var shouldScrollAfterNextRender = false
    @State private var expandedProcessGroupIDs: Set<String> = []
    @State private var selectedPhotoItem: PhotosPickerItem?
    @State private var pendingAttachments: [OutgoingAttachment] = []
    @State private var isShowingBranchChanges = false
    @State private var isPickingPhotoMedia = false
    @State private var isImportingFile = false
    @FocusState private var isPromptFocused: Bool

    private var thread: ThreadSummary {
        appModel.threadSummary(for: session.threadID) ?? session
    }

    private var refreshStatus: ThreadRefreshStatus {
        appModel.refreshStatus(for: thread.threadID)
    }

    private var currentRenderSnapshot: ThreadRenderSnapshot {
        renderedTimeline.threadID == thread.threadID
            ? renderedTimeline
            : .empty(threadID: thread.threadID)
    }

    private var renderToken: ThreadRenderToken {
        ThreadRenderToken(
            threadID: thread.threadID,
            timeline: appModel.timelineSnapshot(for: thread.threadID)
        )
    }

    private var edgeSwipeBackGesture: some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .global)
            .onEnded { value in
                guard shouldDismissFromEdgeSwipe(value) else { return }
                dismiss()
            }
    }

    private var pendingApprovals: [ApprovalSummary] {
        appModel.approvals
            .filter { $0.threadID == thread.threadID && $0.status == .pending }
            .sorted { $0.updatedAt < $1.updatedAt }
    }

    private var pendingUserInputs: [UserInputRequestSummary] {
        appModel.userInputRequests
            .filter { $0.threadID == thread.threadID && ($0.status == .pending || $0.status == .submitting || $0.status == .failed) }
            .sorted { $0.updatedAt < $1.updatedAt }
    }

    var body: some View {
        let snapshot = currentRenderSnapshot
        let timelineRows = ThreadTimelineRow.merge(items: snapshot.items, approvals: pendingApprovals)
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    runtimeStrip

                    VStack(spacing: 10) {
                        ForEach(pendingUserInputs) { request in
                            UserInputPromptRow(request: request)
                                .id("user-input-\(request.requestID)")
                        }

                        ForEach(timelineRows) { row in
                            switch row {
                            case .processGroup(let group):
                                ProcessGroupRow(
                                    group: group,
                                    isExpanded: expandedProcessGroupIDs.contains(group.id),
                                    onToggle: {
                                        toggleProcessGroup(group.id)
                                    }
                                )
                                    .id(row.id)
                            case .message(let item):
                                ThreadEntryRow(item: item)
                                    .id(row.id)
                            case .approval(let approval):
                                ApprovalTimelineRow(approval: approval)
                                    .id(row.id)
                            }
                        }
                    }

                    Color.clear
                        .frame(height: 1)
                        .id("thread-bottom")
                }
                .padding(.horizontal, 16)
                .padding(.top, 8)
                .padding(.bottom, 28)
            }
            .refreshable {
                await loadPreviousHistory()
            }
            .niumaScreenBackground()
            .navigationBarBackButtonHidden()
            .toolbar(.hidden, for: .navigationBar)
            .safeAreaInset(edge: .top, spacing: 0) {
                header
            }
            .safeAreaInset(edge: .bottom, spacing: 0) {
                composer
            }
            .simultaneousGesture(edgeSwipeBackGesture)
            .fileImporter(
                isPresented: $isImportingFile,
                allowedContentTypes: [.item],
                allowsMultipleSelection: true
            ) { result in
                handleFileImport(result)
            }
            .photosPicker(
                isPresented: $isPickingPhotoMedia,
                selection: $selectedPhotoItem,
                matching: .any(of: [.images, .videos])
            )
            .sheet(isPresented: $isShowingBranchChanges) {
                BranchChangesSheet(session: thread)
            }
            .onChange(of: selectedPhotoItem) { _, newItem in
                guard let newItem else { return }
                Task { await loadPhotoAttachment(newItem) }
            }
            .task(id: renderToken) {
                rebuildRenderedTimeline()
            }
            .task {
                await loadInitialDetailsIfNeeded()
            }
            .onAppear {
                appModel.enterThreadDetail(thread.threadID)
                handleRenderSnapshotChange(snapshot, using: proxy)
            }
            .onDisappear {
                appModel.leaveThreadDetail(thread.threadID)
            }
            .onChange(of: snapshot.lastEntryID) { _, _ in
                handleRenderSnapshotChange(currentRenderSnapshot, using: proxy)
            }
            .onChange(of: pendingApprovalIDs) { _, _ in
                scrollToBottom(using: proxy, animated: true)
            }
            .onChange(of: pendingUserInputIDs) { _, _ in
                scrollToBottom(using: proxy, animated: true)
            }
            .onChange(of: scenePhase) { _, phase in
                if phase == .active {
                    appModel.enterThreadDetail(thread.threadID)
                } else {
                    appModel.leaveThreadDetail(thread.threadID)
                }
                guard phase == .active, hasRequestedInitialLoad else { return }
                Task {
                    await refreshDetails()
                }
            }
        }
    }

    private var pendingApprovalIDs: [String] {
        pendingApprovals.map(\.approvalID)
    }

    private var pendingUserInputIDs: [String] {
        pendingUserInputs.map(\.requestID)
    }

    private func pendingApprovalCountText(_ count: Int) -> String {
        L10n.string(
            count == 1 ? "approval.pending.count.one" : "approval.pending.count.other",
            language: appModel.appLanguage,
            count
        )
    }

    private func toggleProcessGroup(_ id: String) {
        withAnimation(.easeInOut(duration: 0.16)) {
            if expandedProcessGroupIDs.contains(id) {
                expandedProcessGroupIDs.remove(id)
            } else {
                expandedProcessGroupIDs.insert(id)
            }
        }
    }

    /// Restores the expected iOS edge-swipe pop gesture while this screen uses
    /// a custom hidden-navigation-bar header.
    private func shouldDismissFromEdgeSwipe(_ value: DragGesture.Value) -> Bool {
        let startsAtLeadingEdge = value.startLocation.x <= 32
        let horizontalDistance = value.translation.width
        let verticalDistance = abs(value.translation.height)
        let predictedDistance = value.predictedEndTranslation.width
        return startsAtLeadingEdge
            && horizontalDistance > 70
            && predictedDistance > 110
            && horizontalDistance > verticalDistance * 1.4
    }

    private var header: some View {
        VStack(spacing: 0) {
            HStack(spacing: 12) {
                Button {
                    dismiss()
                } label: {
                    Image(systemName: "chevron.left")
                        .font(.system(size: 17, weight: .semibold))
                        .foregroundStyle(NiumaPalette.ink)
                        .frame(width: 34, height: 34)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)

                Text(thread.title)
                    .font(.headline.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)

                Button {
                    isShowingBranchChanges = true
                } label: {
                    Image(systemName: "doc.text.magnifyingglass")
                        .font(.system(size: 15, weight: .semibold))
                        .foregroundStyle(NiumaPalette.ink)
                        .frame(width: 34, height: 34)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .accessibilityLabel(L10n.string("thread.branch_changes.view", language: appModel.appLanguage))
                .accessibilityIdentifier("thread-branch-changes-button")

                Button {
                    Task { await refreshDetails() }
                } label: {
                    Group {
                        if refreshStatus.isRefreshing {
                            ProgressView()
                                .tint(NiumaPalette.ink)
                        } else {
                            Image(systemName: "arrow.clockwise")
                                .font(.system(size: 15, weight: .semibold))
                                .foregroundStyle(NiumaPalette.ink)
                        }
                    }
                    .frame(width: 34, height: 34)
                    .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .disabled(refreshStatus.isRefreshing)
                .accessibilityLabel(L10n.string("thread.refresh.accessibility", language: appModel.appLanguage))
                .accessibilityIdentifier("thread-refresh-button")
            }
            .padding(.horizontal, 18)
            .padding(.top, 10)
            .padding(.bottom, 12)
            .background(ThreadTopFade())
        }
    }

    private var runtimeStrip: some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                if let refreshStateBadge = refreshBadge {
                    StatusBadge(title: refreshStateBadge.0, tone: refreshStateBadge.1)
                }
                if let runtimeStateBadge = runtimeBadge {
                    StatusBadge(title: runtimeStateBadge.0, tone: runtimeStateBadge.1)
                }
                if let sessionBadge = thread.status.compactBadge(for: appModel.appLanguage) {
                    StatusBadge(title: sessionBadge.0, tone: sessionBadge.1)
                }
                if !pendingApprovals.isEmpty {
                    StatusBadge(
                        title: pendingApprovalCountText(pendingApprovals.count),
                        tone: .warning
                    )
                }
                if !pendingUserInputs.isEmpty {
                    StatusBadge(
                        title: UserInputPromptCopy.text("user_input.needs", language: appModel.appLanguage),
                        tone: .warning
                    )
                }
            }
            if let refreshError {
                Text(refreshError)
                    .font(.caption)
                    .foregroundStyle(NiumaPalette.critical)
            }
        }
    }

    private var composer: some View {
        ThreadComposerBar(
            prompt: $prompt,
            placeholder: L10n.string("thread.composer.placeholder", language: appModel.appLanguage, thread.title),
            attachments: pendingAttachments,
            currentBranch: currentBranch,
            isSending: false,
            isPromptFocused: $isPromptFocused,
            onPickPhotoOrVideo: {
                isPickingPhotoMedia = true
            },
            onPickFile: {
                isImportingFile = true
            },
            onRemoveAttachment: { attachment in
                pendingAttachments.removeAll { $0.id == attachment.id }
            },
            onSend: sendPrompt
        )
    }

    /// Sends the current composer payload into this existing thread.
    private func sendPrompt() {
        let text = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty || !pendingAttachments.isEmpty else { return }
        let attachments = pendingAttachments
        prompt = ""
        pendingAttachments = []
        isPromptFocused = false
        shouldScrollAfterNextRender = true
        Task {
            await appModel.startTask(
                projectID: project.projectID,
                threadID: thread.threadID,
                prompt: text,
                attachments: attachments
            )
        }
    }

    private func loadPhotoAttachment(_ item: PhotosPickerItem) async {
        defer { selectedPhotoItem = nil }
        do {
            guard let attachment = try await ThreadAttachmentLoader.photoAttachment(
                from: item,
                nextIndex: pendingAttachments.count + 1
            ) else { return }
            pendingAttachments.append(attachment)
        } catch {
            appModel.pendingError = error.localizedDescription
        }
    }

    /// Handles FileImport.
    private func handleFileImport(_ result: Result<[URL], Error>) {
        switch result {
        case .success(let urls):
            Task { await loadFileAttachments(urls) }
        case .failure(let error):
            appModel.pendingError = error.localizedDescription
        }
    }

    private func loadFileAttachments(_ urls: [URL]) async {
        for url in urls {
            do {
                pendingAttachments.append(try ThreadAttachmentLoader.fileAttachment(from: url))
            } catch {
                appModel.pendingError = error.localizedDescription
            }
        }
    }

    private func loadInitialDetailsIfNeeded() async {
        guard !hasRequestedInitialLoad else { return }
        hasRequestedInitialLoad = true
        await refreshDetails()
    }

    /// Requests missing messages through the shared detail refresh flow.
    private func refreshDetails() async {
        guard !refreshStatus.isRefreshing else { return }
        await appModel.refresh()
        await appModel.refreshThreadDetails(threadID: thread.threadID)
    }

    /// Loads one older five-user-turn window from local SwiftData.
    private func loadPreviousHistory() async {
        guard !isLoadingHistory else { return }
        isLoadingHistory = true
        _ = await appModel.loadPreviousThreadWindow(threadID: thread.threadID)
        shouldScrollAfterNextRender = false
        isLoadingHistory = false
    }

    private var refreshBadge: (String, StatusBadge.Tone)? {
        switch refreshStatus.phase {
        case .idle, .succeeded:
            return nil
        case .refreshing:
            return (refreshStatus.phase.title(for: appModel.appLanguage), .warning)
        case .failed:
            return (refreshStatus.phase.title(for: appModel.appLanguage), .critical)
        case .timedOut:
            return (refreshStatus.phase.title(for: appModel.appLanguage), .critical)
        }
    }

    private var currentBranch: String? {
        guard let branch = thread.currentBranch?.trimmingCharacters(in: .whitespacesAndNewlines),
              !branch.isEmpty else {
            return nil
        }
        return branch
    }

    private var refreshError: String? {
        guard refreshStatus.phase == .failed || refreshStatus.phase == .timedOut else { return nil }
        guard let error = refreshStatus.error, !error.isEmpty else { return nil }
        return error
    }

    private var runtimeBadge: (String, StatusBadge.Tone)? {
        switch appModel.runtimeState {
        case .idle, .completed:
            return nil
        case .submitting:
            return (appModel.runtimeState.title(for: appModel.appLanguage), .warning)
        case .streaming:
            return (appModel.runtimeState.title(for: appModel.appLanguage), .positive)
        case .waitingApproval:
            return (appModel.runtimeState.title(for: appModel.appLanguage), .warning)
        case .reconnecting:
            return (appModel.runtimeState.title(for: appModel.appLanguage), .warning)
        case .failed:
            if refreshStatus.phase == .failed || refreshStatus.phase == .timedOut {
                return nil
            }
            return (appModel.runtimeState.title(for: appModel.appLanguage), .critical)
        }
    }

    private func rebuildRenderedTimeline() {
        guard let timeline = appModel.timelineSnapshot(for: thread.threadID) else {
            renderedTimeline = .empty(threadID: thread.threadID)
            return
        }
        renderedTimeline = ThreadRenderSnapshot.make(from: timeline)
    }

    private func handleRenderSnapshotChange(_ snapshot: ThreadRenderSnapshot, using proxy: ScrollViewProxy) {
        guard snapshot.threadID == thread.threadID, snapshot.entryCount > 0 else { return }
        if !hasScrolledAfterInitialRender {
            hasScrolledAfterInitialRender = true
            scrollToBottom(using: proxy, animated: false)
            return
        }
        guard shouldScrollAfterNextRender else { return }
        shouldScrollAfterNextRender = false
        scrollToBottom(using: proxy, animated: true)
    }

    private func scrollToBottom(using proxy: ScrollViewProxy, animated: Bool) {
        Task { @MainActor in
            await Task.yield()
            if animated {
                withAnimation(.easeOut(duration: 0.2)) {
                    proxy.scrollTo("thread-bottom", anchor: .bottom)
                }
            } else {
                proxy.scrollTo("thread-bottom", anchor: .bottom)
            }
        }
    }
}

private struct ThreadRenderToken: Hashable {
    let threadID: String
    let ackSeq: Int
    let checkpoint: String?
    let entryCount: Int
    let lastEntryID: String?
    let lastEntryTextLength: Int

    init(threadID: String, timeline: ThreadTimeline?) {
        self.threadID = threadID
        self.ackSeq = timeline?.ackSeq ?? 0
        self.checkpoint = timeline?.checkpoint
        self.entryCount = timeline?.entries.count ?? 0
        self.lastEntryID = timeline?.entries.last?.id
        self.lastEntryTextLength = timeline?.entries.last?.text.count ?? 0
    }
}
