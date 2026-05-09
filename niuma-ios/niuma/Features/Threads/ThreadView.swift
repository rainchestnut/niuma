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
    @State private var selectedPhotoItem: PhotosPickerItem?
    @State private var pendingAttachments: [OutgoingAttachment] = []
    @State private var isShowingAttachmentOptions = false
    @State private var isShowingBranchChanges = false
    @State private var isPickingPhotoMedia = false
    @State private var isImportingFile = false
    @FocusState private var isPromptFocused: Bool

    private var thread: ThreadSummary {
        session
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

    private var pendingApprovals: [ApprovalSummary] {
        return appModel.approvals.filter { $0.threadID == thread.threadID && $0.status == .pending }
    }

    var body: some View {
        let snapshot = currentRenderSnapshot
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    runtimeStrip

                    if !pendingApprovals.isEmpty {
                        approvalsStrip
                    }

                    VStack(spacing: 10) {
                        ForEach(snapshot.items) { item in
                            switch item {
                            case .processGroup(let group):
                                ProcessGroupRow(group: group)
                                    .id(item.id)
                            case .message(let item):
                                ThreadEntryRow(item: item)
                                    .id(item.id)
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
            .confirmationDialog("添加附件", isPresented: $isShowingAttachmentOptions, titleVisibility: .visible) {
                Button("添加图片或视频") {
                    isPickingPhotoMedia = true
                }
                Button("添加文件") {
                    isImportingFile = true
                }
                Button("取消", role: .cancel) {}
            }
            .sheet(isPresented: $isShowingBranchChanges) {
                BranchChangesSheet(session: session)
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
                handleRenderSnapshotChange(snapshot, using: proxy)
            }
            .onChange(of: snapshot.lastEntryID) { _, _ in
                handleRenderSnapshotChange(currentRenderSnapshot, using: proxy)
            }
            .onChange(of: scenePhase) { _, phase in
                guard phase == .active, hasRequestedInitialLoad else { return }
                Task {
                    await refreshDetails()
                }
            }
        }
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

                Text(session.title)
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
                .accessibilityLabel("查看分支变更")
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
                .accessibilityLabel("刷新会话")
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
                if let sessionBadge = session.status.compactBadge {
                    StatusBadge(title: sessionBadge.0, tone: sessionBadge.1)
                }
                if !pendingApprovals.isEmpty {
                    StatusBadge(title: "\(pendingApprovals.count) 待审批", tone: .warning)
                }
            }
            if let refreshError {
                Text(refreshError)
                    .font(.caption)
                    .foregroundStyle(NiumaPalette.critical)
            }
        }
    }

    private var approvalsStrip: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("需要处理")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(NiumaPalette.ink)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 10) {
                    ForEach(pendingApprovals) { approval in
                        NavigationLink {
                            ApprovalDetailView(approval: approval)
                        } label: {
                            HStack(spacing: 10) {
                                Image(systemName: "exclamationmark.bubble")
                                    .foregroundStyle(NiumaPalette.warning)
                                VStack(alignment: .leading, spacing: 3) {
                                    Text("审批")
                                        .font(.caption2.weight(.semibold))
                                        .foregroundStyle(NiumaPalette.mutedInk)
                                    Text(approval.requestMethod ?? approval.approvalType)
                                        .font(.footnote.weight(.medium))
                                        .foregroundStyle(NiumaPalette.ink)
                                }
                            }
                            .padding(.horizontal, 12)
                            .padding(.vertical, 10)
                            .background(
                                RoundedRectangle(cornerRadius: 18, style: .continuous)
                                    .fill(NiumaPalette.warningSoft)
                            )
                        }
                        .buttonStyle(.plain)
                    }
                }
            }
        }
    }

    private var composer: some View {
        ThreadComposerBar(
            prompt: $prompt,
            placeholder: "给“\(session.title)”发送消息",
            attachments: pendingAttachments,
            isSending: false,
            isPromptFocused: $isPromptFocused,
            onAddAttachment: {
                isShowingAttachmentOptions = true
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
                threadID: session.threadID,
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
        await appModel.refreshThreadDetails(threadID: session.threadID)
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
            return ("刷新中", .warning)
        case .failed:
            return ("刷新失败", .critical)
        case .timedOut:
            return ("刷新超时", .critical)
        }
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
            return ("提交中", .warning)
        case .streaming:
            return ("流式中", .positive)
        case .waitingApproval:
            return ("待审批", .warning)
        case .reconnecting:
            return ("重连中", .warning)
        case .failed:
            if refreshStatus.phase == .failed || refreshStatus.phase == .timedOut {
                return nil
            }
            return ("失败", .critical)
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
