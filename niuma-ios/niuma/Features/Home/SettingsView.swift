import SwiftUI

struct SettingsView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @State private var isShowingResetConfirmation = false
    @State private var isResettingAllData = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                SurfaceCard(title: appModel.localized("偏好设置", "Preferences")) {
                    VStack(alignment: .leading, spacing: 16) {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(appModel.localized("语言", "Language"))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(NiumaPalette.mutedInk)
                            Picker("", selection: Binding(
                                get: { appModel.appLanguage },
                                set: { appModel.updateLanguage($0) }
                            )) {
                                Text("中文").tag(AppLanguage.chinese)
                                Text("English").tag(AppLanguage.english)
                            }
                            .pickerStyle(.segmented)
                        }

                        VStack(alignment: .leading, spacing: 8) {
                            Text(appModel.localized("主题", "Theme"))
                                .font(.caption.weight(.semibold))
                                .foregroundStyle(NiumaPalette.mutedInk)
                            Picker("", selection: Binding(
                                get: { appModel.appTheme },
                                set: { appModel.updateTheme($0) }
                            )) {
                                Text(appModel.localized("系统", "System")).tag(AppTheme.system)
                                Text(appModel.localized("浅色", "Light")).tag(AppTheme.light)
                                Text(appModel.localized("深色", "Dark")).tag(AppTheme.dark)
                            }
                            .pickerStyle(.segmented)
                        }
                    }
                }

                SurfaceCard(title: appModel.localized("设备身份", "Device")) {
                    VStack(alignment: .leading, spacing: 12) {
                        settingsRow(title: appModel.localized("设备名", "Device Name"), value: appModel.identity?.displayName ?? appModel.localized("未初始化", "Not Initialized"))
                        settingsRow(title: "Device ID", value: appModel.identity?.deviceID ?? appModel.localized("未生成", "Unavailable"), monospaced: true)
                        settingsRow(title: appModel.localized("控制面", "Control Plane"), value: appModel.controllerModeLabel)
                    }
                }

                SurfaceCard(
                    title: appModel.localized("服务地址", "Server Address"),
                    subtitle: appModel.localized("修改后会断开当前实时连接，并使用新地址重新认证。", "Changing this disconnects the current realtime channel and reauthenticates through the new address.")
                ) {
                    ServerEndpointEditor(
                        helpText: appModel.localized(
                            "保持与桌面 Gateway 的 server-url 一致，否则扫码绑定和后续同步会失败。",
                            "Keep this aligned with the desktop Gateway server-url or pairing and sync will fail."
                        )
                    )
                }

                SurfaceCard(title: appModel.localized("状态", "Status")) {
                    VStack(alignment: .leading, spacing: 12) {
                        let deviceBadge = appModel.deviceState.badge(for: appModel.appLanguage)
                        settingsRow(title: appModel.localized("设备状态", "Device State"), value: deviceBadge.0)
                        let connectionBadge = appModel.connectionState.badge(for: appModel.appLanguage)
                        settingsRow(title: appModel.localized("实时连接", "Realtime"), value: connectionBadge.0)
                        settingsRow(title: appModel.localized("线程状态", "Thread State"), value: appModel.runtimeState.title(for: appModel.appLanguage))
                    }
                }

                SurfaceCard(
                    title: appModel.localized("数据", "Data"),
                    subtitle: appModel.localized(
                        "清除本机设置、缓存、设备身份和所有桌面绑定。",
                        "Clear local settings, cache, device identity, and all desktop links."
                    )
                ) {
                    resetAllDataButton
                }
            }
            .padding()
        }
        .niumaScreenBackground()
        .navigationTitle(appModel.localized("设置", "Settings"))
        .confirmationDialog(
            appModel.localized("重置所有数据？", "Reset All Data?"),
            isPresented: $isShowingResetConfirmation,
            titleVisibility: .visible
        ) {
            Button(appModel.localized("重置所有数据", "Reset All Data"), role: .destructive) {
                Task { await resetAllData() }
            }
            Button(appModel.localized("取消", "Cancel"), role: .cancel) {}
        } message: {
            Text(appModel.localized(
                "这会删除本机所有对话缓存、偏好设置、设备身份和配对数据。完成后需要重新扫码配对。",
                "This deletes all local chat cache, preferences, device identity, and pairing data. You will need to scan and pair again."
            ))
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

                Text(isResettingAllData ? appModel.localized("正在重置…", "Resetting…") : appModel.localized("重置所有数据", "Reset All Data"))
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
