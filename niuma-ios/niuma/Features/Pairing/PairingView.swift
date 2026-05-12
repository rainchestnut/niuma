import AVFoundation
import SwiftUI
import UIKit

struct PairingView: View {
    @Environment(AppModel.self) private var appModel
    @Environment(\.dismiss) private var dismiss
    @FocusState private var isServerEndpointFocused: Bool
    @State private var cameraStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)
    @State private var isShowingScanner = false
    @State private var isShowingManualPairing = false
    @State private var serverEndpointDraft = ""
    @State private var serverEndpointStatusMessage: String?
    @State private var submittedScanRaw: String?
    @State private var isPairing = false
    @State private var pairingStatusMessage: String?
    @State private var scannerErrorMessage: String?
    @State private var manualPairingErrorMessage: String?

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

                        serverEndpointSection

                        Divider()
                            .overlay(NiumaPalette.border)

                        cameraSection

                        #if DEBUG
                        manualPairingButton
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
        .onAppear {
            syncServerEndpointDraft()
        }
        .onChange(of: appModel.serverBaseURLText) { _, nextValue in
            guard !isServerEndpointFocused else { return }
            serverEndpointDraft = nextValue
        }
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
        .sheet(isPresented: $isShowingManualPairing) {
            ManualPairingSheet(
                isPairing: isPairing,
                statusMessage: pairingStatusMessage,
                errorMessage: manualPairingErrorMessage,
                language: appModel.appLanguage,
                onConfirm: handleManualPair
            )
            .interactiveDismissDisabled(isPairing)
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
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

    #if DEBUG
    private var manualPairingButton: some View {
        Button {
            openManualPairingSheet()
        } label: {
            Label(
                isPairing
                    ? appModel.localized("pairing.connecting")
                    : appModel.localized("pairing.manual.action"),
                systemImage: "doc.text"
            )
            .font(.headline.weight(.semibold))
            .foregroundStyle(NiumaPalette.accent)
            .padding(.horizontal, 18)
            .padding(.vertical, 14)
            .frame(maxWidth: .infinity)
            .background(
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .fill(NiumaPalette.accentSoft.opacity(0.76))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .stroke(NiumaPalette.accent.opacity(0.28), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
        .disabled(isPairing)
        .opacity(isPairing ? 0.64 : 1)
        .accessibilityIdentifier("pairing-manual-payload-button")
    }
    #endif

    private var serverEndpointSection: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(appModel.localized("pairing.server.help"))
                .font(.footnote)
                .foregroundStyle(NiumaPalette.mutedInk)
                .fixedSize(horizontal: false, vertical: true)

            TextField(appModel.serverBaseURLPlaceholder, text: $serverEndpointDraft)
                .keyboardType(.URL)
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .textContentType(.URL)
                .focused($isServerEndpointFocused)
                .submitLabel(.done)
                .niumaInputChrome()
                .onSubmit {
                    Task { await applyServerEndpointDraftIfNeeded() }
                }

            if let validation = appModel.serverBaseURLValidationMessage {
                Text(validation)
                    .font(.footnote)
                    .foregroundStyle(NiumaPalette.critical)
            } else if let serverEndpointStatusMessage {
                Text(serverEndpointStatusMessage)
                    .font(.footnote)
                    .foregroundStyle(NiumaPalette.mutedInk)
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
        guard await applyServerEndpointDraftIfNeeded() else {
            scannerErrorMessage = serverEndpointFailureMessage
            return
        }
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
        Task { await confirmScannedPair(raw: raw) }
    }

    private func confirmScannedPair(raw: String) async {
        let paired = await confirmPairing(raw: raw)
        if paired {
            isShowingScanner = false
        } else {
            submittedScanRaw = nil
            scannerErrorMessage = pairingFailureMessage
        }
        await finishPairingIfNeeded(paired: paired)
    }

    private func openManualPairingSheet() {
        pairingStatusMessage = nil
        manualPairingErrorMessage = nil
        isShowingManualPairing = true
    }

    private func handleManualPair(_ raw: String) {
        let trimmed = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            manualPairingErrorMessage = appModel.localized("pairing.manual.empty")
            return
        }
        manualPairingErrorMessage = nil
        Task { await confirmManualPair(raw: trimmed) }
    }

    private func confirmManualPair(raw: String) async {
        let paired = await confirmPairing(raw: raw)
        if paired {
            isShowingManualPairing = false
        } else {
            manualPairingErrorMessage = pairingFailureMessage
        }
        await finishPairingIfNeeded(paired: paired)
    }

    private func confirmPairing(raw: String) async -> Bool {
        guard !isPairing else { return false }
        guard await applyServerEndpointDraftIfNeeded() else { return false }
        isPairing = true
        defer { isPairing = false }
        pairingStatusMessage = appModel.localized("pairing.status.confirming")
        return await appModel.pairWithScannedPayload(raw)
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

    private var pairingFailureMessage: String {
        appModel.pendingError
            ?? appModel.serverBaseURLValidationMessage
            ?? serverEndpointStatusMessage
            ?? appModel.localized("pairing.scan.invalid")
    }

    private var serverEndpointFailureMessage: String {
        appModel.serverBaseURLValidationMessage ?? appModel.localized("pairing.server.required")
    }

    private func syncServerEndpointDraft() {
        serverEndpointDraft = appModel.serverBaseURLText
    }

    /// Applies the endpoint lazily so pairing no longer needs a separate
    /// "apply server address" button on the same screen.
    @discardableResult
    private func applyServerEndpointDraftIfNeeded() async -> Bool {
        let trimmed = serverEndpointDraft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            appModel.serverBaseURLValidationMessage = nil
            serverEndpointStatusMessage = appModel.localized("pairing.server.required")
            return false
        }

        let applied = await appModel.updateServerBaseURL(from: trimmed)
        serverEndpointStatusMessage = applied ? appModel.localized("settings.server.applied") : nil
        if applied {
            serverEndpointDraft = appModel.serverBaseURLText
            isServerEndpointFocused = false
        }
        return applied
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

/// Manual DEBUG entry point for pasting the exact QR payload when the simulator
/// cannot scan a camera frame.
private struct ManualPairingSheet: View {
    @Environment(\.dismiss) private var dismiss
    @FocusState private var isInputFocused: Bool
    @State private var pairingInfo = ""

    let isPairing: Bool
    let statusMessage: String?
    let errorMessage: String?
    let language: AppLanguage
    let onConfirm: (String) -> Void

    var body: some View {
        NavigationStack {
            VStack(alignment: .leading, spacing: 16) {
                Text(L10n.string("pairing.manual.instructions", language: language))
                    .font(.footnote)
                    .foregroundStyle(NiumaPalette.mutedInk)

                pairingInfoEditor

                if let message = errorMessage ?? statusMessage {
                    Text(message)
                        .font(.footnote)
                        .foregroundStyle(errorMessage == nil ? NiumaPalette.mutedInk : NiumaPalette.critical)
                        .fixedSize(horizontal: false, vertical: true)
                }

                Button {
                    onConfirm(pairingInfo)
                } label: {
                    if isPairing {
                        ProgressView()
                            .tint(NiumaPalette.darkButtonText)
                    } else {
                        Text(L10n.string("pairing.manual.confirm", language: language))
                    }
                }
                .buttonStyle(NiumaPrimaryButtonStyle())
                .disabled(isPairing)
            }
            .padding(20)
            .navigationTitle(L10n.string("pairing.manual.title", language: language))
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(L10n.string("common.cancel", language: language)) {
                        dismiss()
                    }
                    .disabled(isPairing)
                }
            }
            .onAppear {
                isInputFocused = true
            }
        }
        .niumaScreenBackground()
    }

    private var pairingInfoEditor: some View {
        ZStack(alignment: .topLeading) {
            if pairingInfo.isEmpty {
                Text(L10n.string("pairing.manual.placeholder", language: language))
                    .font(.body)
                    .foregroundStyle(NiumaPalette.mutedInk)
                    .padding(.horizontal, 18)
                    .padding(.vertical, 16)
            }

            TextEditor(text: $pairingInfo)
                .font(.system(.body, design: .monospaced))
                .textInputAutocapitalization(.never)
                .autocorrectionDisabled()
                .focused($isInputFocused)
                .scrollContentBackground(.hidden)
                .padding(10)
        }
        .frame(minHeight: 220)
        .background(NiumaPalette.raisedCard, in: RoundedRectangle(cornerRadius: 18, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 18, style: .continuous)
                .stroke(NiumaPalette.border, lineWidth: 1)
        )
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
