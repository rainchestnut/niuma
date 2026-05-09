import SwiftUI

/// Shared editor for the persisted Niuma Server endpoint used by pairing and settings screens.
struct ServerEndpointEditor: View {
    @Environment(AppModel.self) private var appModel
    @FocusState private var isFocused: Bool
    @State private var draft = ""
    @State private var statusMessage: String?

    let helpText: String

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(helpText)
                .font(.footnote)
                .foregroundStyle(NiumaPalette.mutedInk)
                .fixedSize(horizontal: false, vertical: true)

            TextField(appModel.serverBaseURLPlaceholder, text: $draft)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textContentType(.URL)
                .focused($isFocused)
                .submitLabel(.done)
                .niumaInputChrome()
                .onSubmit {
                    Task { await applyDraft() }
                }

            if let validation = appModel.serverBaseURLValidationMessage {
                Text(validation)
                    .font(.footnote)
                    .foregroundStyle(NiumaPalette.critical)
            } else if let statusMessage {
                Text(statusMessage)
                    .font(.footnote)
                    .foregroundStyle(NiumaPalette.mutedInk)
            }

            Button(appModel.isUpdatingServerBaseURL ? appModel.localized("settings.server.applying") : appModel.localized("settings.server.apply")) {
                Task { await applyDraft() }
            }
            .buttonStyle(NiumaPrimaryButtonStyle())
            .disabled(appModel.isUpdatingServerBaseURL || draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
        }
        .onAppear {
            draft = appModel.serverBaseURLText
        }
        .onChange(of: appModel.serverBaseURLText) { _, nextValue in
            guard !isFocused else { return }
            draft = nextValue
        }
    }

    /// Applies the visible draft to the shared app model so the next pairing request uses the selected server.
    private func applyDraft() async {
        let applied = await appModel.updateServerBaseURL(from: draft)
        guard applied else {
            statusMessage = nil
            return
        }
        draft = appModel.serverBaseURLText
        isFocused = false
        statusMessage = appModel.localized("settings.server.applied")
    }
}
