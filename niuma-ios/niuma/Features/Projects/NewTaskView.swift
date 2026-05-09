import Foundation
import PhotosUI
import SwiftUI
import UniformTypeIdentifiers

/// Composer-first screen for creating either a project thread or a projectless conversation.
struct NewTaskView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss

    let project: ProjectSummary?

    @State private var prompt = ""
    @State private var isSubmitting = false
    @State private var selectedPhotoItem: PhotosPickerItem?
    @State private var pendingAttachments: [OutgoingAttachment] = []
    @State private var isPickingPhotoMedia = false
    @State private var isImportingFile = false
    @FocusState private var isPromptFocused: Bool

    init(project: ProjectSummary? = nil) {
        self.project = project
    }

    private var targetProjectID: String {
        project?.projectID ?? conversationProjectID
    }

    private var targetTitle: String {
        project?.projectName ?? appModel.localized("新对话", "New Chat")
    }

    private var targetSubtitle: String {
        project == nil
            ? appModel.localized("无项目对话", "No Project")
            : appModel.localized("项目对话", "Project")
    }

    private var targetIcon: String {
        project == nil ? "bubble.left.and.text.bubble.right" : "folder"
    }

    var body: some View {
        ScrollView {
            VStack(spacing: 14) {
                Spacer(minLength: 96)
                Image(systemName: targetIcon)
                    .font(.system(size: 28, weight: .medium))
                    .foregroundStyle(NiumaPalette.info)
                    .frame(width: 58, height: 58)
                    .background(Circle().fill(NiumaPalette.infoSoft))
                VStack(spacing: 6) {
                    Text(targetTitle)
                        .font(.title3.weight(.semibold))
                        .foregroundStyle(NiumaPalette.ink)
                        .multilineTextAlignment(.center)
                    Text(targetSubtitle)
                        .font(.caption)
                        .foregroundStyle(NiumaPalette.mutedInk)
                }
                Spacer(minLength: 160)
            }
            .frame(maxWidth: .infinity)
            .padding(.horizontal, 22)
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
        .onChange(of: selectedPhotoItem) { _, newItem in
            guard let newItem else { return }
            Task { await loadPhotoAttachment(newItem) }
        }
        .onAppear {
            isPromptFocused = true
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

                Text(targetTitle)
                    .font(.headline.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .padding(.horizontal, 18)
            .padding(.top, 10)
            .padding(.bottom, 12)
            .background(ThreadTopFade())
        }
    }

    private var composer: some View {
        ThreadComposerBar(
            prompt: $prompt,
            placeholder: appModel.localized("发送第一条消息", "Send the first message"),
            attachments: pendingAttachments,
            isSending: isSubmitting,
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
            onSend: {
                Task {
                    await submit()
                }
            }
        )
    }

    private func submit() async {
        guard !isSubmitting else { return }
        isSubmitting = true
        defer { isSubmitting = false }

        do {
            try await appModel.startNewTask(
                projectID: targetProjectID,
                prompt: prompt,
                attachments: pendingAttachments
            )
            dismiss()
        } catch {
            appModel.pendingError = error.localizedDescription
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

    /// Handles document picker results and keeps successfully read files attached.
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
}
