import SwiftUI

/// Shared bottom composer used by thread detail and new-thread screens.
struct ThreadComposerBar: View {
    @Environment(AppModel.self) private var appModel
    @Binding var prompt: String
    @State private var speechTranscriber = ComposerSpeechTranscriber()

    let placeholder: String
    let attachments: [OutgoingAttachment]
    let isSending: Bool
    let isPromptFocused: FocusState<Bool>.Binding
    let onAddAttachment: () -> Void
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
        VStack(spacing: 8) {
            HStack(spacing: 8) {
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
                .accessibilityLabel("选择思考深度")
                .accessibilityIdentifier("thread-reasoning-effort-menu")
                permissionMenu
                Spacer()
            }

            if !attachments.isEmpty {
                ComposerAttachmentStrip(attachments: attachments, onRemove: onRemoveAttachment)
            }

            HStack(spacing: 10) {
                Button(action: onAddAttachment) {
                    Image(systemName: "plus")
                        .font(.system(size: 17, weight: .medium))
                        .foregroundStyle(NiumaPalette.ink)
                        .frame(width: 34, height: 34)
                        .contentShape(Circle())
                }
                .buttonStyle(.plain)
                .disabled(isSending)
                .accessibilityLabel("添加附件")
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
                            isPromptFocused.wrappedValue = true
                        }
                        .layoutPriority(1)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .accessibilityLabel("消息输入")
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
                    .accessibilityLabel(speechTranscriber.isRecording ? "停止语音输入" : "开始语音输入")
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
                    .accessibilityLabel("发送")
                    .accessibilityIdentifier("thread-send-button")
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .niumaGlassChrome(cornerRadius: 28)
        }
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .padding(.bottom, 10)
        .background(ThreadBottomFade())
        .onDisappear {
            speechTranscriber.stop()
        }
    }

    private func send() {
        guard canSend else { return }
        speechTranscriber.stop()
        isPromptFocused.wrappedValue = false
        onSend()
    }

    private var permissionMenu: some View {
        Menu {
            Section("权限") {
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
                Section("审批策略") {
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

                Section("审查人") {
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

                Section("沙盒") {
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
        .accessibilityLabel("选择权限")
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
            return "默认权限"
        case .autoReview:
            return "自动审查"
        case .fullAccess:
            return "完全访问"
        case .custom:
            return "自定义"
        }
    }

    private func approvalPolicyTitle(_ policy: CodexApprovalPolicy) -> String {
        switch policy {
        case .untrusted:
            return "不受信任"
        case .onRequest:
            return "按需审批"
        case .never:
            return "不询问"
        }
    }

    private func approvalsReviewerTitle(_ reviewer: CodexApprovalsReviewer) -> String {
        switch reviewer {
        case .user:
            return "用户审查"
        case .guardianSubagent:
            return "自动审查"
        }
    }

    private func sandboxModeTitle(_ mode: CodexSandboxMode) -> String {
        switch mode {
        case .readOnly:
            return "只读"
        case .workspaceWrite:
            return "工作区写入"
        case .dangerFullAccess:
            return "完全访问"
        }
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
