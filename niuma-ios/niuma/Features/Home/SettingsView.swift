import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @State private var isShowingResetConfirmation = false
    @State private var isResettingAllData = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                SurfaceCard(title: appModel.localized("settings.preferences.title")) {
                    VStack(alignment: .leading, spacing: 16) {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(appModel.localized("settings.language"))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(NiumaPalette.mutedInk)
                            Picker("", selection: Binding(
                                get: { appModel.appLanguage },
                                set: { appModel.updateLanguage($0) }
                            )) {
                                Text(appModel.localized("settings.language.chinese")).tag(AppLanguage.chinese)
                                Text(appModel.localized("settings.language.english")).tag(AppLanguage.english)
                            }
                            .pickerStyle(.segmented)
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text(appModel.localized("settings.theme"))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(NiumaPalette.mutedInk)
                            Picker("", selection: Binding(
                                get: { appModel.appTheme },
                                set: { appModel.updateTheme($0) }
                            )) {
                                Text(appModel.localized("settings.theme.system")).tag(AppTheme.system)
                                Text(appModel.localized("settings.theme.light")).tag(AppTheme.light)
                                Text(appModel.localized("settings.theme.dark")).tag(AppTheme.dark)
                            }
                            .pickerStyle(.segmented)
                        }
                    }
                }

                SurfaceCard(title: appModel.localized("settings.device.title")) {
                    VStack(alignment: .leading, spacing: 12) {
                        settingsRow(title: appModel.localized("settings.device.name"), value: appModel.identity?.displayName ?? appModel.localized("device_state.uninitialized"))
                        settingsRow(title: appModel.localized("settings.device.id"), value: appModel.identity?.deviceID ?? appModel.localized("common.unavailable"), monospaced: true)
                    }
                }

                SurfaceCard(
                    title: appModel.localized("settings.server.title"),
                    subtitle: appModel.localized("settings.server.subtitle")
                ) {
                    ServerEndpointEditor(
                        helpText: appModel.localized("settings.server.help")
                    )
                }

                SurfaceCard(title: appModel.localized("settings.status.title")) {
                    VStack(alignment: .leading, spacing: 12) {
                        let deviceBadge = appModel.deviceState.badge(for: appModel.appLanguage)
                        settingsRow(title: appModel.localized("settings.status.device"), value: deviceBadge.0)
                        let connectionBadge = appModel.connectionState.badge(for: appModel.appLanguage)
                        settingsRow(title: appModel.localized("settings.status.realtime"), value: connectionBadge.0)
                        settingsRow(title: appModel.localized("settings.status.thread"), value: appModel.runtimeState.title(for: appModel.appLanguage))
                    }
                }

                SurfaceCard(
                    title: appModel.localized("settings.data.title"),
                    subtitle: appModel.localized("settings.data.subtitle")
                ) {
                    resetAllDataButton
                }
            }
            .padding()
        }
        .niumaScreenBackground()
        .navigationTitle(appModel.localized("settings.title"))
        .confirmationDialog(
            appModel.localized("settings.reset.confirm.title"),
            isPresented: $isShowingResetConfirmation,
            titleVisibility: .visible
        ) {
            Button(appModel.localized("settings.reset.action"), role: .destructive) {
                Task { await resetAllData() }
            }
            Button(appModel.localized("common.cancel"), role: .cancel) {}
        } message: {
            Text(appModel.localized("settings.reset.confirm.message"))
        }
    }

    private var resetAllDataButton: some View {
        Button {
            isShowingResetConfirmation = true
        } label: {
            HStack(spacing: 12) {
                Image(systemName: "trash")
                    .font(.system(size: 15, weight: .semibold))
                    .frame(width: 30, height: 30)
                    .background(Color.red.opacity(0.10), in: Circle())

                Text(isResettingAllData ? appModel.localized("settings.resetting") : appModel.localized("settings.reset.action"))
                    .font(.body.weight(.semibold))

                Spacer()

                if isResettingAllData {
                    ProgressView()
                        .tint(.red)
                }
            }
            .foregroundStyle(.red)
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(Color.red.opacity(0.07), in: RoundedRectangle(cornerRadius: 18, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(Color.red.opacity(0.18), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
        .disabled(isResettingAllData)
        .accessibilityIdentifier("settings-reset-all-data-button")
    }

    private func resetAllData() async {
        guard !isResettingAllData else { return }
        isResettingAllData = true
        defer { isResettingAllData = false }
        if await appModel.resetAllAppData() {
            dismiss()
        }
    }

    private func settingsRow(title: String, value: String, monospaced: Bool = false) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(title)
                .font(.caption.weight(.semibold))
                .foregroundStyle(NiumaPalette.mutedInk)
            Text(value)
                .font(monospaced ? .footnote.monospaced() : .body)
                .foregroundStyle(NiumaPalette.ink)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(14)
        .background(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .fill(NiumaPalette.raisedCard)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(NiumaPalette.border, lineWidth: 1)
        )
    }
}
