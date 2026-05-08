import AVFoundation
import SwiftUI
import UIKit

struct PairingView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @State private var cameraStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)
    @State private var scannedRaw: String?
    @State private var isPairing = false
    @State private var pairingStatusMessage: String?

    let dismissOnSuccess: Bool

    /// Initializes this pairing screen for either first-run onboarding or add-device navigation.
    init(dismissOnSuccess: Bool = true) {
        self.dismissOnSuccess = dismissOnSuccess
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                header

                SurfaceCard(title: appModel.localized("桌面配对", "Desktop Pairing")) {
                    VStack(alignment: .leading, spacing: 18) {
                        gatewaySummary

                        ServerEndpointEditor(
                            helpText: appModel.localized(
                                "请输入与桌面 Gateway 相同的 Niuma Server 地址，应用后再扫码配对。",
                                "Enter the same Niuma Server address used by the desktop Gateway, then apply it before pairing."
                            )
                        )

                        Divider()
                            .overlay(NiumaPalette.border)

                        cameraSection

                        if let scannedRaw {
                            scanResult(raw: scannedRaw)
                        }

                        #if DEBUG
                        Button(isPairing ? "连接中…" : "模拟扫码当前桌面二维码") {
                            Task { await runDesktopDebugPair() }
                        }
                        .buttonStyle(.bordered)
                        .controlSize(.large)
                        .frame(maxWidth: .infinity)
                        .disabled(isPairing)
                        .accessibilityIdentifier("pairing-debug-current-qr-button")
                        #endif

                        if let pairingStatusMessage {
                            Text(pairingStatusMessage)
                                .font(.footnote)
                                .foregroundStyle(NiumaPalette.mutedInk)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.top, 2)
                        }
                    }
                }
            }
            .padding(24)
        }
        .niumaScreenBackground()
        .navigationTitle("桌面配对")
        .navigationBarTitleDisplayMode(.inline)
        .task {
            // Re-check status on entry so we react to permission changes that
            // happened while the user was in Settings.
            cameraStatus = AVCaptureDevice.authorizationStatus(for: .video)
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(appModel.localized(dismissOnSuccess ? "添加桌面" : "连接 Niuma", dismissOnSuccess ? "Add Desktop" : "Connect Niuma"))
                .font(.system(size: 38, weight: .bold))
                .foregroundStyle(NiumaPalette.ink)
            Text(appModel.localized(
                "绑定已启动 Niuma Gateway 的桌面设备后，首页会展示最近需要处理的项目和对话。",
                "After linking a desktop device running Niuma Gateway, the home screen shows projects and chats needing attention."
            ))
            .font(.body)
            .foregroundStyle(NiumaPalette.mutedInk)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var gatewaySummary: some View {
        HStack(alignment: .top, spacing: 12) {
            Image(systemName: "desktopcomputer")
                .font(.system(size: 17, weight: .semibold))
                .foregroundStyle(NiumaPalette.darkButtonText)
                .frame(width: 34, height: 34)
                .background(Circle().fill(NiumaPalette.darkButton))

            VStack(alignment: .leading, spacing: 4) {
                Text(appModel.localized("连接运行中的 Niuma Gateway", "Connect a Running Niuma Gateway"))
                    .font(.headline.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                Text(appModel.localized(
                    "移动端和桌面 Gateway 必须指向同一个 Niuma Server。",
                    "Mobile and desktop Gateway must point to the same Niuma Server."
                ))
                .font(.footnote)
                .foregroundStyle(NiumaPalette.mutedInk)
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    @ViewBuilder
    private var cameraSection: some View {
        switch cameraStatus {
        case .authorized:
            QRScannerView(onScan: handleScan)
                .frame(height: 280)
                .clipShape(RoundedRectangle(cornerRadius: 12))
        case .notDetermined:
            VStack(spacing: 12) {
                Text("需要相机权限以扫描二维码。")
                Button("授权使用相机") {
                    Task { await requestCameraAccess() }
                }
                .buttonStyle(NiumaPrimaryButtonStyle())
                .accessibilityIdentifier("pairing-camera-permission-button")
            }
        case .denied, .restricted:
            VStack(spacing: 12) {
                Text("相机权限已被拒绝。")
                Button("打开系统设置") {
                    openSystemSettings()
                }
                .buttonStyle(NiumaPrimaryButtonStyle())
            }
        @unknown default:
            Text("无法获取相机状态。")
        }
    }

    /// Shows the decoded QR payload before the user commits the desktop binding.
    private func scanResult(raw: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("识别到桌面二维码")
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(NiumaPalette.ink)
            Text(raw)
                .font(.system(.footnote, design: .monospaced))
                .foregroundStyle(NiumaPalette.mutedInk)
                .lineLimit(3)
                .truncationMode(.middle)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    RoundedRectangle(cornerRadius: 14, style: .continuous)
                        .fill(NiumaPalette.raisedCard)
                )
            Button(isPairing ? "配对中…" : "确认配对") {
                Task { await confirmPair(raw: raw) }
            }
            .buttonStyle(NiumaPrimaryButtonStyle())
            .disabled(isPairing)
            .accessibilityIdentifier("pairing-confirm-button")
        }
    }

    /// Handles Scan.
    private func handleScan(_ raw: String) {
        // Skip duplicates so a held-up code doesn't pair-confirm twice.
        guard scannedRaw != raw, !isPairing else { return }
        scannedRaw = raw
    }

    private func confirmPair(raw: String) async {
        isPairing = true
        defer { isPairing = false }
        pairingStatusMessage = "正在完成桌面绑定…"
        let paired = await appModel.pairWithScannedPayload(raw)
        await finishPairingIfNeeded(paired: paired)
    }

    private func runDesktopDebugPair() async {
        isPairing = true
        defer { isPairing = false }
        pairingStatusMessage = "正在读取桌面 Gateway 二维码并完成绑定…"
        let paired = await appModel.pairWithDesktopGateway()
        await finishPairingIfNeeded(paired: paired)
    }

    /// Requests CameraAccess.
    private func requestCameraAccess() async {
        let granted = await AVCaptureDevice.requestAccess(for: .video)
        cameraStatus = granted ? .authorized : .denied
    }

    private func openSystemSettings() {
        guard let url = URL(string: UIApplication.openSettingsURLString) else { return }
        UIApplication.shared.open(url)
    }

    @MainActor
    private func finishPairingIfNeeded(paired: Bool) async {
        guard paired else {
            pairingStatusMessage = appModel.pendingError ?? "绑定失败，请重试。"
            return
        }
        pairingStatusMessage = "绑定成功，正在返回首页…"
        try? await Task.sleep(for: .milliseconds(350))
        if dismissOnSuccess {
            dismiss()
        }
    }
}
