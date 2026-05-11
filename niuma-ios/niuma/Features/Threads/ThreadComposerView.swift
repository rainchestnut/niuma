import SwiftUI

/// Shared bottom composer used by thread detail and new-thread screens.
struct ThreadComposerBar: View {
    @Environment(AppModel.self) private var appModel
    @Binding var prompt: String
    @State private var speechTranscriber = ComposerSpeechTranscriber()
    @State private var isAttachmentPanelExpanded = false

    let placeholder: String
    let attachments: [OutgoingAttachment]
    let currentBranch: String?
    let isSending: Bool
    let isPromptFocused: FocusState<Bool>.Binding
    let onPickPhotoOrVideo: () -> Void
    let onPickFile: () -> Void
    let onRemoveAttachment: (OutgoingAttachment) -> Void
    let onSend: () -> Void

    private let reasoningEfforts: [ReasoningEffort] = [.xhigh, .high, .medium, .low]
    private let permissionPresets: [ApprovalPermissionPreset] = [
        .defaultPermissions,
        .autoReview,
        .fullAccess,
        .custom,
    ]

    private var canSend: Bool {
        !isSending
            && !speechTranscriber.isRecording
            && !speechTranscriber.isPreparing
            && !speechTranscriber.isFinalizing
            && (!prompt.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty)
    }

    var body: some View {
        VStack(spacing: 0) {
            VStack(spacing: 8) {
                HStack(spacing: 8) {
                    if let currentBranch {
                        ComposerBranchPill(branch: currentBranch)
                    }
                    if appModel.availableModels.isEmpty {
                        ComposerPill(title: appModel.displayedModelID)
                    } else {
                        Menu {
                            ForEach(appModel.availableModels, id: \.self) { modelID in
                                Button {
                                    appModel.selectModel(modelID)
                                } label: {
                                    if appModel.selectedModelID == modelID {
                                        Label(modelID, systemImage: "checkmark")
                                    } else {
                                        Text(modelID)
                                    }
                                }
                            }
                        } label: {
                            ComposerPill(title: appModel.displayedModelID)
                        }
                        .buttonStyle(.plain)
                    }
                    Menu {
                        ForEach(reasoningEfforts) { effort in
                            Button {
                                appModel.selectReasoningEffort(effort)
                            } label: {
                                if appModel.selectedReasoningEffort == effort {
                                    Label(effort.rawValue, systemImage: "checkmark")
                                } else {
                                    Text(effort.rawValue)
                                }
                            }
                        }
                    } label: {
                        ComposerPill(title: appModel.selectedReasoningEffort.rawValue)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel(appModel.localized("thread.reasoning.accessibility"))
                    .accessibilityIdentifier("thread-reasoning-effort-menu")
                    permissionMenu
                    Spacer()
                }

                if !attachments.isEmpty {
                    ComposerAttachmentStrip(attachments: attachments, onRemove: onRemoveAttachment)
                }

                HStack(spacing: 10) {
                    Button {
                        toggleAttachmentPanel()
                    } label: {
                        Image(systemName: "plus")
                            .font(.system(size: 17, weight: .medium))
                            .foregroundStyle(NiumaPalette.ink)
                            .rotationEffect(.degrees(isAttachmentPanelExpanded ? 45 : 0))
                            .frame(width: 34, height: 34)
                            .background(
                                Circle()
                                    .fill(isAttachmentPanelExpanded ? NiumaPalette.neutralSoft : .clear)
                            )
                            .contentShape(Circle())
                    }
                    .buttonStyle(.plain)
                    .disabled(isSending)
                    .accessibilityLabel(appModel.localized("thread.attachment.add.accessibility"))
                    .accessibilityIdentifier("thread-attachment-button")

                    HStack(alignment: .bottom, spacing: 10) {
                        TextField(placeholder, text: $prompt, axis: .vertical)
                            .font(.callout)
                            .foregroundStyle(NiumaPalette.ink)
                            .textFieldStyle(.plain)
                            .lineLimit(1...4)
                            .submitLabel(.send)
                            .focused(isPromptFocused)
                            .textInputAutocapitalization(.never)
                            .autocorrectionDisabled()
                            .disabled(isSending)
                            .onSubmit {
                                send()
                            }
                            .padding(.vertical, 3)
                            .contentShape(Rectangle())
                            .onTapGesture {
                                closeAttachmentPanel()
                                isPromptFocused.wrappedValue = true
                            }
                            .layoutPriority(1)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .accessibilityLabel(appModel.localized("thread.input.accessibility"))
                            .accessibilityIdentifier("thread-prompt-field")

                        Button {
                            speechTranscriber.toggle(
                                currentPrompt: prompt,
                                locale: Locale(identifier: appModel.localeIdentifier),
                                updatePrompt: { transcript in
                                    prompt = transcript
                                },
                                reportError: { message in
                                    appModel.pendingError = message
                                }
                            )
                        } label: {
                            Group {
                                if speechTranscriber.isPreparing || speechTranscriber.isFinalizing {
                                    ProgressView()
                                        .tint(NiumaPalette.mutedInk)
                                } else {
                                    Image(systemName: speechTranscriber.isRecording ? "mic.fill" : "mic")
                                        .font(.system(size: 17, weight: .medium))
                                }
                            }
                            .foregroundStyle(speechTranscriber.isRecording ? NiumaPalette.critical : NiumaPalette.mutedInk)
                            .frame(width: 30, height: 30)
                        }
                        .buttonStyle(.plain)
                        .disabled(isSending || speechTranscriber.isFinalizing)
                        .accessibilityLabel(appModel.localized(speechTranscriber.isRecording ? "thread.voice.stop" : "thread.voice.start"))
                        .accessibilityIdentifier("thread-voice-input-button")

                        Button {
                            send()
                        } label: {
                            Group {
                                if isSending {
                                    ProgressView()
                                        .tint(.white)
                                } else {
                                    Image(systemName: "arrow.up")
                                        .font(.system(size: 16, weight: .bold))
                                }
                            }
                            .foregroundStyle(.white)
                            .frame(width: 34, height: 34)
                            .background(Circle().fill(.black))
                            .contentShape(Circle())
                        }
                        .buttonStyle(.plain)
                        .disabled(!canSend)
                        .opacity(canSend ? 1 : 0.45)
                        .accessibilityLabel(appModel.localized("common.send"))
                        .accessibilityIdentifier("thread-send-button")
                    }
                }
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .niumaGlassChrome(cornerRadius: 28)
            }
            .padding(.horizontal, 12)
            .padding(.top, 8)
            .padding(.bottom, isAttachmentPanelExpanded ? 8 : 10)

            if isAttachmentPanelExpanded {
                ComposerAttachmentPanel(
                    photoTitle: L10n.string("attachment.media", language: appModel.appLanguage),
                    fileTitle: L10n.string("attachment.file", language: appModel.appLanguage),
                    onPickPhotoOrVideo: openPhotoPicker,
                    onPickFile: openFilePicker
                )
                .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .frame(maxWidth: .infinity)
        .background(alignment: .bottom) {
            if isAttachmentPanelExpanded {
                NiumaPalette.neutralSoft
                    .ignoresSafeArea(edges: .bottom)
            } else {
                ThreadBottomFade()
            }
        }
        .onDisappear {
            speechTranscriber.stop()
        }
    }

    private func send() {
        guard canSend else { return }
        closeAttachmentPanel()
        speechTranscriber.stop()
        isPromptFocused.wrappedValue = false
        onSend()
    }

    private func toggleAttachmentPanel() {
        guard !isSending else { return }
        isPromptFocused.wrappedValue = false
        withAnimation(.easeInOut(duration: 0.18)) {
            isAttachmentPanelExpanded.toggle()
        }
    }

    private func closeAttachmentPanel() {
        guard isAttachmentPanelExpanded else { return }
        withAnimation(.easeInOut(duration: 0.18)) {
            isAttachmentPanelExpanded = false
        }
    }

    private func openPhotoPicker() {
        closeAttachmentPanel()
        onPickPhotoOrVideo()
    }

    private func openFilePicker() {
        closeAttachmentPanel()
        onPickFile()
    }

    private var permissionMenu: some View {
        Menu {
            Section(appModel.localized("thread.permissions.section")) {
                ForEach(permissionPresets) { preset in
                    Button {
                        appModel.selectApprovalPermissionPreset(preset)
                    } label: {
                        menuLabel(
                            title: permissionPresetTitle(preset),
                            isSelected: appModel.approvalPermissionPreset == preset
                        )
                    }
                }
            }

            if appModel.approvalPermissionPreset == .custom {
                Section(appModel.localized("thread.permissions.approval_policy")) {
                    ForEach(CodexApprovalPolicy.allCases) { policy in
                        Button {
                            appModel.selectCustomApprovalPolicy(policy)
                        } label: {
                            menuLabel(
                                title: approvalPolicyTitle(policy),
                                isSelected: appModel.customApprovalPolicy == policy
                            )
                        }
                    }
                }

                Section(appModel.localized("thread.permissions.reviewer")) {
                    ForEach(CodexApprovalsReviewer.allCases) { reviewer in
                        Button {
                            appModel.selectCustomApprovalsReviewer(reviewer)
                        } label: {
                            menuLabel(
                                title: approvalsReviewerTitle(reviewer),
                                isSelected: appModel.customApprovalsReviewer == reviewer
                            )
                        }
                    }
                }

                Section(appModel.localized("thread.permissions.sandbox")) {
                    ForEach(CodexSandboxMode.allCases) { mode in
                        Button {
                            appModel.selectCustomSandboxMode(mode)
                        } label: {
                            menuLabel(
                                title: sandboxModeTitle(mode),
                                isSelected: appModel.customSandboxMode == mode
                            )
                        }
                    }
                }
            }
        } label: {
            ComposerPill(title: permissionPresetTitle(appModel.approvalPermissionPreset))
        }
        .buttonStyle(.plain)
        .accessibilityLabel(appModel.localized("thread.permissions.accessibility"))
        .accessibilityIdentifier("thread-permission-menu")
    }

    private func menuLabel(title: String, isSelected: Bool) -> some View {
        Group {
            if isSelected {
                Label(title, systemImage: "checkmark")
            } else {
                Text(title)
            }
        }
    }

    private func permissionPresetTitle(_ preset: ApprovalPermissionPreset) -> String {
        switch preset {
        case .defaultPermissions:
            return appModel.localized("permission_preset.default")
        case .autoReview:
            return appModel.localized("permission_preset.auto_review")
        case .fullAccess:
            return appModel.localized("permission_preset.full_access")
        case .custom:
            return appModel.localized("permission_preset.custom")
        }
    }

    private func approvalPolicyTitle(_ policy: CodexApprovalPolicy) -> String {
        switch policy {
        case .untrusted:
            return appModel.localized("approval_policy.untrusted")
        case .onRequest:
            return appModel.localized("approval_policy.on_request")
        case .never:
            return appModel.localized("approval_policy.never")
        }
    }

    private func approvalsReviewerTitle(_ reviewer: CodexApprovalsReviewer) -> String {
        switch reviewer {
        case .user:
            return appModel.localized("approval_reviewer.user")
        case .guardianSubagent:
            return appModel.localized("approval_reviewer.guardian_subagent")
        }
    }

    private func sandboxModeTitle(_ mode: CodexSandboxMode) -> String {
        switch mode {
        case .readOnly:
            return appModel.localized("sandbox_mode.read_only")
        case .workspaceWrite:
            return appModel.localized("sandbox_mode.workspace_write")
        case .dangerFullAccess:
            return appModel.localized("sandbox_mode.danger_full_access")
        }
    }
}

/// WeChat-style attachment tray with only the two currently supported attachment sources.
private struct ComposerAttachmentPanel: View {
    let photoTitle: String
    let fileTitle: String
    let onPickPhotoOrVideo: () -> Void
    let onPickFile: () -> Void

    var body: some View {
        HStack(alignment: .top, spacing: 18) {
            ComposerAttachmentTile(
                title: photoTitle,
                systemImage: "photo.on.rectangle",
                action: onPickPhotoOrVideo
            )
            ComposerAttachmentTile(
                title: fileTitle,
                systemImage: "doc",
                action: onPickFile
            )
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 22)
        .padding(.top, 14)
        .padding(.bottom, 24)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(
            ZStack(alignment: .top) {
                Rectangle()
                    .fill(NiumaPalette.neutralSoft)
                Rectangle()
                    .fill(NiumaPalette.border)
                    .frame(height: 1)
            }
        )
        .accessibilityIdentifier("thread-attachment-panel")
    }
}

private struct ComposerAttachmentTile: View {
    let title: String
    let systemImage: String
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            VStack(alignment: .center, spacing: 7) {
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(NiumaPalette.card.opacity(0.94))
                    .frame(width: 46, height: 46)
                    .overlay(
                        Image(systemName: systemImage)
                            .font(.system(size: 18, weight: .medium))
                            .foregroundStyle(NiumaPalette.ink)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: 18, style: .continuous)
                            .stroke(NiumaPalette.border, lineWidth: 1)
                    )

                Text(title)
                    .font(.caption2.weight(.medium))
                    .foregroundStyle(NiumaPalette.mutedInk)
                    .lineLimit(2)
                    .multilineTextAlignment(.center)
                    .minimumScaleFactor(0.82)
                    .frame(width: 76, alignment: .top)
                    .frame(minHeight: 28, alignment: .top)
            }
            .frame(width: 76, alignment: .top)
            .frame(minHeight: 84, alignment: .top)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(title)
    }
}

struct ThreadTopFade: View {
    var body: some View {
        ZStack {
            Rectangle()
                .fill(.regularMaterial)
                .mask(
                    LinearGradient(
                        colors: [.white, .white.opacity(0.98), .white.opacity(0.78), .clear],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
            LinearGradient(
                colors: [NiumaPalette.canvas.opacity(0.72), NiumaPalette.canvas.opacity(0.42), .clear],
                startPoint: .top,
                endPoint: .bottom
            )
        }
        .ignoresSafeArea(edges: .top)
    }
}

struct ThreadBottomFade: View {
    var body: some View {
        ZStack {
            Rectangle()
                .fill(.ultraThinMaterial)
                .mask(
                    LinearGradient(
                        colors: [.clear, .white.opacity(0.78), .white],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                )
            LinearGradient(
                colors: [.clear, NiumaPalette.canvas.opacity(0.10), NiumaPalette.canvas.opacity(0.22)],
                startPoint: .top,
                endPoint: .bottom
            )
        }
        .ignoresSafeArea(edges: .bottom)
    }
}

struct ComposerPill: View {
    let title: String

    var body: some View {
        Text(title)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(NiumaPalette.mutedInk)
            .lineLimit(1)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(Capsule().fill(NiumaPalette.raisedCard))
    }
}

private struct ComposerBranchPill: View {
    @Environment(AppModel.self) private var appModel
    @State private var isShowingFullBranch = false

    let branch: String

    var body: some View {
        Button {
            isShowingFullBranch = true
        } label: {
            HStack(spacing: 6) {
                Image(systemName: "arrow.triangle.branch")
                Text(branch)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Image(systemName: "info.circle")
            }
            .font(.caption2.weight(.semibold))
            .foregroundStyle(NiumaPalette.mutedInk)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .frame(maxWidth: 132, alignment: .leading)
            .background(Capsule().fill(NiumaPalette.raisedCard))
        }
        .buttonStyle(.plain)
        .alert(branch, isPresented: $isShowingFullBranch) {
            Button(appModel.localized("common.ok"), role: .cancel) {}
        }
        .accessibilityLabel(branch)
        .accessibilityIdentifier("thread-current-branch-pill")
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
