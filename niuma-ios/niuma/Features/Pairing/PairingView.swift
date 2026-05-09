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

                SurfaceCard(title: appModel.localized("pairing.title")) {
                    VStack(alignment: .leading, spacing: 18) {
                        gatewaySummary

                        ServerEndpointEditor(helpText: appModel.localized("pairing.server.help"))

                        Divider()
                            .overlay(NiumaPalette.border)

                        cameraSection

                        #if DEBUG
                        Button(isPairing ? appModel.localized("pairing.connecting") : appModel.localized("pairing.debug.current_qr")) {
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
        .navigationTitle(appModel.localized("pairing.title"))
        .navigationBarTitleDisplayMode(.inline)
        .task {
            // Re-check status on entry so we react to permission changes that
            // happened while the user was in Settings.
            cameraStatus = AVCaptureDevice.authorizationStatus(for: .video)
        }
        .fullScreenCover(isPresented: $isShowingScanner) {
            PairingScannerSheet(
                title: appModel.localized("pairing.scan.title"),
                idleMessage: appModel.localized("pairing.scan.idle"),
                statusMessage: pairingStatusMessage,
                errorMessage: scannerErrorMessage,
                isPairing: isPairing,
                language: appModel.appLanguage,
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
            Text(appModel.localized(dismissOnSuccess ? "pairing.add_desktop" : "pairing.connect_niuma"))
                .font(.system(size: 38, weight: .bold))
                .foregroundStyle(NiumaPalette.ink)
            Text(appModel.localized("pairing.header.description"))
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
                Text(appModel.localized("pairing.gateway.title"))
                    .font(.headline.weight(.semibold))
                    .foregroundStyle(NiumaPalette.ink)
                Text(appModel.localized("pairing.gateway.description"))
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
                Text(appModel.localized("pairing.camera.instructions"))
                .font(.footnote)
                .foregroundStyle(NiumaPalette.mutedInk)
                .fixedSize(horizontal: false, vertical: true)

                Button {
                    Task { await openScanner() }
                } label: {
                    Label(
                        isPairing
                            ? appModel.localized("pairing.pairing")
                            : appModel.localized("pairing.scan.action"),
                        systemImage: "qrcode.viewfinder"
                    )
                }
                .buttonStyle(NiumaPrimaryButtonStyle())
                .disabled(isPairing)
                .accessibilityIdentifier("pairing-scan-button")
            }
        case .denied, .restricted:
            VStack(spacing: 12) {
                Text(appModel.localized("pairing.camera.denied"))
                Button(appModel.localized("pairing.open_settings")) {
                    openSystemSettings()
                }
                .buttonStyle(NiumaPrimaryButtonStyle())
            }
        @unknown default:
            Text(appModel.localized("pairing.camera.unknown"))
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
        pairingStatusMessage = appModel.localized("pairing.status.confirming")
        let paired = await appModel.pairWithScannedPayload(raw)
        if paired {
            isShowingScanner = false
        } else {
            submittedScanRaw = nil
            scannerErrorMessage = appModel.pendingError ?? appModel.localized("pairing.scan.invalid")
        }
        await finishPairingIfNeeded(paired: paired)
    }

    private func runDesktopDebugPair() async {
        isPairing = true
        defer { isPairing = false }
        pairingStatusMessage = appModel.localized("pairing.status.debug_pairing")
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
            pairingStatusMessage = appModel.pendingError ?? appModel.localized("pairing.status.failed")
            return
        }
        pairingStatusMessage = appModel.localized("pairing.status.succeeded")
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
    let language: AppLanguage
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
            .accessibilityLabel(L10n.string("pairing.scan.close", language: language))
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
