import AVFoundation
import SwiftUI
import UIKit

struct PairingView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @State private var cameraStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)
    @State private var isShowingScanner = false
    @State private var submittedScanRaw: String?
    @State private var isPairing = false
    @State private var pairingStatusMessage: String?
    @State private var scannerErrorMessage: String?

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
        .fullScreenCover(isPresented: $isShowingScanner) {
            PairingScannerSheet(
                title: appModel.localized("扫描桌面二维码", "Scan Desktop QR Code"),
                idleMessage: appModel.localized(
                    "将桌面 Gateway 的二维码放入取景框，识别后会自动配对。",
                    "Place the desktop Gateway QR code in frame. Pairing starts automatically."
                ),
                statusMessage: pairingStatusMessage,
                errorMessage: scannerErrorMessage,
                isPairing: isPairing,
                onClose: {
                    isShowingScanner = false
                },
                onScan: handleScan
            )
            .interactiveDismissDisabled(isPairing)
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
        case .authorized, .notDetermined:
            VStack(alignment: .leading, spacing: 12) {
                Text(appModel.localized(
                    "点击按钮打开相机，识别到桌面二维码后会自动完成配对。",
                    "Open the camera and pairing will start as soon as the desktop QR code is recognized."
                ))
                .font(.footnote)
                .foregroundStyle(NiumaPalette.mutedInk)
                .fixedSize(horizontal: false, vertical: true)

                Button {
                    Task { await openScanner() }
                } label: {
                    Label(
                        isPairing
                            ? appModel.localized("配对中…", "Pairing…")
                            : appModel.localized("扫码配对", "Scan to Pair"),
                        systemImage: "qrcode.viewfinder"
                    )
                }
                .buttonStyle(NiumaPrimaryButtonStyle())
                .disabled(isPairing)
                .accessibilityIdentifier("pairing-scan-button")
            }
        case .denied, .restricted:
            VStack(spacing: 12) {
                Text(appModel.localized("相机权限已被拒绝。", "Camera access was denied."))
                Button(appModel.localized("打开系统设置", "Open Settings")) {
                    openSystemSettings()
                }
                .buttonStyle(NiumaPrimaryButtonStyle())
            }
        @unknown default:
            Text("无法获取相机状态。")
        }
    }

    /// Opens the camera scanner, requesting camera authorization first when needed.
    private func openScanner() async {
        scannerErrorMessage = nil
        pairingStatusMessage = nil
        submittedScanRaw = nil
        switch cameraStatus {
        case .authorized:
            isShowingScanner = true
        case .notDetermined:
            await requestCameraAccess(openScannerOnGrant: true)
        default:
            break
        }
    }

    /// Handles a QR payload from the full-screen scanner and starts pairing immediately.
    private func handleScan(_ raw: String) {
        // Skip duplicates so a held-up code doesn't pair-confirm repeatedly.
        guard submittedScanRaw != raw, !isPairing else { return }
        submittedScanRaw = raw
        scannerErrorMessage = nil
        Task { await confirmPair(raw: raw) }
    }

    private func confirmPair(raw: String) async {
        guard !isPairing else { return }
        isPairing = true
        defer { isPairing = false }
        pairingStatusMessage = "正在完成桌面绑定…"
        let paired = await appModel.pairWithScannedPayload(raw)
        if paired {
            isShowingScanner = false
        } else {
            submittedScanRaw = nil
            scannerErrorMessage = appModel.pendingError ?? "二维码无效或已过期，请重新扫描。"
        }
        await finishPairingIfNeeded(paired: paired)
    }

    private func runDesktopDebugPair() async {
        isPairing = true
        defer { isPairing = false }
        pairingStatusMessage = "正在读取桌面 Gateway 二维码并完成绑定…"
        let paired = await appModel.pairWithDesktopGateway()
        await finishPairingIfNeeded(paired: paired)
    }

    /// Requests camera access and optionally opens the scanner after authorization.
    private func requestCameraAccess(openScannerOnGrant: Bool = false) async {
        let granted = await AVCaptureDevice.requestAccess(for: .video)
        cameraStatus = granted ? .authorized : .denied
        if granted, openScannerOnGrant {
            isShowingScanner = true
        }
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

/// Full-screen QR scanner used only while pairing; recognition triggers pairing immediately.
private struct PairingScannerSheet: View {
    let title: String
    let idleMessage: String
    let statusMessage: String?
    let errorMessage: String?
    let isPairing: Bool
    let onClose: () -> Void
    let onScan: (String) -> Void

    var body: some View {
        ZStack {
            QRScannerView(onScan: onScan)
                .ignoresSafeArea()

            VStack(spacing: 0) {
                header
                Spacer()
                statusPanel
            }
            .padding(.horizontal, 20)
            .padding(.top, 18)
            .padding(.bottom, 30)
        }
        .background(Color.black.ignoresSafeArea())
    }

    private var header: some View {
        HStack(spacing: 12) {
            Button {
                onClose()
            } label: {
                Image(systemName: "xmark")
                    .font(.system(size: 16, weight: .semibold))
                    .foregroundStyle(.white)
                    .frame(width: 42, height: 42)
                    .background(Circle().fill(.black.opacity(0.42)))
            }
            .buttonStyle(.plain)
            .disabled(isPairing)
            .accessibilityLabel("关闭扫码")
            .accessibilityIdentifier("pairing-scanner-close-button")

            Text(title)
                .font(.headline.weight(.semibold))
                .foregroundStyle(.white)

            Spacer()
        }
    }

    private var statusPanel: some View {
        VStack(spacing: 10) {
            if isPairing {
                ProgressView()
                    .tint(.white)
            } else {
                Image(systemName: errorMessage == nil ? "qrcode.viewfinder" : "exclamationmark.triangle")
                    .font(.system(size: 22, weight: .semibold))
                    .foregroundStyle(errorMessage == nil ? .white : .yellow)
            }

            Text(displayMessage)
                .font(.footnote.weight(.medium))
                .multilineTextAlignment(.center)
                .foregroundStyle(.white)
        }
        .padding(.horizontal, 18)
        .padding(.vertical, 16)
        .frame(maxWidth: .infinity)
        .background(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(.black.opacity(0.42))
        )
    }

    private var displayMessage: String {
        if let errorMessage {
            return errorMessage
        }
        if let statusMessage {
            return statusMessage
        }
        return idleMessage
    }
}
