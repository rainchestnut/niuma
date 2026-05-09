@preconcurrency import AVFoundation
import Observation
import Speech

/// Drives the message composer microphone button with iOS 26 SpeechAnalyzer.
@MainActor
@Observable
final class ComposerSpeechTranscriber {
    var isPreparing = false
    var isRecording = false
    var isFinalizing = false

    @ObservationIgnored private let audioInput = ComposerSpeechAudioInput()
    @ObservationIgnored private var analyzer: SpeechAnalyzer?
    @ObservationIgnored private var transcriber: SpeechTranscriber?
    @ObservationIgnored private var analyzerInput: AsyncStream<AnalyzerInput>.Continuation?
    @ObservationIgnored private var analyzerTask: Task<Void, Never>?
    @ObservationIgnored private var audioTask: Task<Void, Never>?
    @ObservationIgnored private var resultTask: Task<Void, Never>?
    @ObservationIgnored private var converter: AVAudioConverter?
    @ObservationIgnored private var analyzerFormat: AVAudioFormat?
    @ObservationIgnored private var startingPrompt = ""
    @ObservationIgnored private var finalizedTranscript = ""
    @ObservationIgnored private var volatileTranscript = ""

    /// Starts recording if idle, otherwise stops and finalizes the active transcription.
    func toggle(
        currentPrompt: String,
        locale: Locale,
        updatePrompt: @escaping @MainActor (String) -> Void,
        reportError: @escaping @MainActor (String) -> Void
    ) {
        guard !isFinalizing else { return }
        if isPreparing || isRecording {
            stop()
        } else {
            start(currentPrompt: currentPrompt, locale: locale, updatePrompt: updatePrompt, reportError: reportError)
        }
    }

    /// Stops microphone capture and asks SpeechAnalyzer to finalize volatile text.
    func stop() {
        guard isPreparing || isRecording || analyzer != nil || audioTask != nil else { return }
        let wasPreparing = isPreparing
        isPreparing = false
        isRecording = false
        audioInput.stop()
        analyzerInput?.finish()

        if wasPreparing {
            audioTask?.cancel()
            resultTask?.cancel()
            analyzerTask?.cancel()
            resetSession()
            return
        }

        let analyzer = analyzer
        let analyzerTask = analyzerTask
        isFinalizing = true
        Task {
            try? await analyzer?.finalizeAndFinishThroughEndOfInput()
            await analyzerTask?.value
            resetSession()
        }
    }

    private func start(
        currentPrompt: String,
        locale: Locale,
        updatePrompt: @escaping @MainActor (String) -> Void,
        reportError: @escaping @MainActor (String) -> Void
    ) {
        resetTranscript(currentPrompt: currentPrompt)
        isPreparing = true
        audioTask = Task {
            do {
                try await requestMicrophonePermission()
                try setUpAudioSession()
                try await setUpTranscriber(locale: locale, updatePrompt: updatePrompt, reportError: reportError)
                let stream = try audioInput.start()
                isPreparing = false
                isRecording = true

                for await buffer in stream {
                    guard !Task.isCancelled else { break }
                    try streamAudioToAnalyzer(buffer)
                }
            } catch {
                guard !Task.isCancelled else { return }
                stop()
                reportError(error.localizedDescription)
            }
        }
    }

    /// Configures SpeechTranscriber, ensures its on-device model, and starts result handling.
    private func setUpTranscriber(
        locale: Locale,
        updatePrompt: @escaping @MainActor (String) -> Void,
        reportError: @escaping @MainActor (String) -> Void
    ) async throws {
        guard SpeechTranscriber.isAvailable else {
            throw ComposerSpeechTranscriberError.transcriberUnavailable
        }
        guard let supportedLocale = await SpeechTranscriber.supportedLocale(equivalentTo: locale) else {
            throw ComposerSpeechTranscriberError.unsupportedLocale(locale.identifier)
        }

        let transcriber = SpeechTranscriber(
            locale: supportedLocale,
            transcriptionOptions: [],
            reportingOptions: [.volatileResults],
            attributeOptions: []
        )
        try await ensureModel(for: transcriber)

        let analyzer = SpeechAnalyzer(
            modules: [transcriber],
            options: SpeechAnalyzer.Options(priority: .userInitiated, modelRetention: .whileInUse)
        )
        guard let analyzerFormat = await SpeechAnalyzer.bestAvailableAudioFormat(compatibleWith: [transcriber]) else {
            throw ComposerSpeechTranscriberError.missingAnalyzerFormat
        }
        try await analyzer.prepareToAnalyze(in: analyzerFormat)

        let input = AsyncStream.makeStream(of: AnalyzerInput.self, bufferingPolicy: .unbounded)
        self.transcriber = transcriber
        self.analyzer = analyzer
        self.analyzerFormat = analyzerFormat
        self.analyzerInput = input.continuation
        self.resultTask = makeResultTask(transcriber: transcriber, updatePrompt: updatePrompt, reportError: reportError)
        self.analyzerTask = Task {
            do {
                try await analyzer.start(inputSequence: input.stream)
            } catch {
                guard !Task.isCancelled else { return }
                reportError(error.localizedDescription)
            }
        }
    }

    private func ensureModel(for transcriber: SpeechTranscriber) async throws {
        if let request = try await AssetInventory.assetInstallationRequest(supporting: [transcriber]) {
            try await request.downloadAndInstall()
        }
    }

    private func makeResultTask(
        transcriber: SpeechTranscriber,
        updatePrompt: @escaping @MainActor (String) -> Void,
        reportError: @escaping @MainActor (String) -> Void
    ) -> Task<Void, Never> {
        Task {
            do {
                for try await result in transcriber.results {
                    apply(result: result, updatePrompt: updatePrompt)
                }
            } catch {
                guard !Task.isCancelled else { return }
                reportError(error.localizedDescription)
            }
        }
    }

    private func requestMicrophonePermission() async throws {
        let microphoneAllowed = await AVCaptureDevice.requestAccess(for: .audio)
        guard microphoneAllowed else {
            throw ComposerSpeechTranscriberError.microphonePermissionDenied
        }
    }

    private func setUpAudioSession() throws {
        let audioSession = AVAudioSession.sharedInstance()
        try audioSession.setCategory(.playAndRecord, mode: .spokenAudio)
        try audioSession.setActive(true, options: .notifyOthersOnDeactivation)
    }

    private func streamAudioToAnalyzer(_ buffer: AVAudioPCMBuffer) throws {
        guard let analyzerFormat, let analyzerInput else {
            throw ComposerSpeechTranscriberError.missingAnalyzerFormat
        }
        let converted = try convert(buffer, to: analyzerFormat)
        analyzerInput.yield(AnalyzerInput(buffer: converted))
    }

    private func convert(_ buffer: AVAudioPCMBuffer, to format: AVAudioFormat) throws -> AVAudioPCMBuffer {
        if buffer.format.niumaMatches(format) {
            return buffer
        }
        if converter == nil || converter?.inputFormat.niumaMatches(buffer.format) != true {
            converter = AVAudioConverter(from: buffer.format, to: format)
        }
        guard let converter else {
            throw ComposerSpeechTranscriberError.converterUnavailable
        }

        let ratio = format.sampleRate / buffer.format.sampleRate
        let capacity = AVAudioFrameCount((Double(buffer.frameLength) * ratio).rounded(.up)) + 1
        guard let converted = AVAudioPCMBuffer(pcmFormat: format, frameCapacity: capacity) else {
            throw ComposerSpeechTranscriberError.converterUnavailable
        }

        var didProvideInput = false
        var conversionError: NSError?
        let status = converter.convert(to: converted, error: &conversionError) { _, outStatus in
            if didProvideInput {
                outStatus.pointee = .noDataNow
                return nil
            }
            didProvideInput = true
            outStatus.pointee = .haveData
            return buffer
        }
        if let conversionError {
            throw conversionError
        }
        switch status {
        case .haveData, .inputRanDry, .endOfStream:
            return converted
        case .error:
            throw ComposerSpeechTranscriberError.conversionFailed
        @unknown default:
            throw ComposerSpeechTranscriberError.conversionFailed
        }
    }

    private func apply(result: SpeechTranscriber.Result, updatePrompt: @MainActor (String) -> Void) {
        let text = String(result.text.characters).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        if result.isFinal {
            finalizedTranscript = ComposerSpeechTranscriber.join(finalizedTranscript, text)
            volatileTranscript = ""
        } else {
            volatileTranscript = text
        }
        updatePrompt(composedPrompt())
    }

    private func resetTranscript(currentPrompt: String) {
        startingPrompt = currentPrompt
        finalizedTranscript = ""
        volatileTranscript = ""
    }

    private func composedPrompt() -> String {
        let transcript = ComposerSpeechTranscriber.join(finalizedTranscript, volatileTranscript)
        return ComposerSpeechTranscriber.join(startingPrompt, transcript)
    }

    private func resetSession() {
        resultTask?.cancel()
        analyzerTask?.cancel()
        audioTask = nil
        resultTask = nil
        analyzerTask = nil
        analyzerInput = nil
        analyzer = nil
        transcriber = nil
        converter = nil
        analyzerFormat = nil
        isFinalizing = false
        try? AVAudioSession.sharedInstance().setActive(false, options: .notifyOthersOnDeactivation)
    }

    private static func join(_ lhs: String, _ rhs: String) -> String {
        let trimmedRHS = rhs.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedRHS.isEmpty else { return lhs }
        let trimmedLHS = lhs.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmedLHS.isEmpty else { return trimmedRHS }
        if lhs.last?.isWhitespace == true {
            return lhs + trimmedRHS
        }
        return lhs + " " + trimmedRHS
    }
}

/// Provides microphone audio buffers to SpeechAnalyzer without exposing AVAudioEngine to SwiftUI views.
private final class ComposerSpeechAudioInput {
    private let audioEngine = AVAudioEngine()
    private var continuation: AsyncStream<AVAudioPCMBuffer>.Continuation?
    private var hasInstalledTap = false

    func start() throws -> AsyncStream<AVAudioPCMBuffer> {
        let stream = AsyncStream.makeStream(of: AVAudioPCMBuffer.self, bufferingPolicy: .unbounded)
        continuation = stream.continuation

        let inputNode = audioEngine.inputNode
        let inputFormat = inputNode.outputFormat(forBus: 0)
        inputNode.installTap(onBus: 0, bufferSize: 4096, format: inputFormat) { buffer, _ in
            stream.continuation.yield(buffer)
        }
        hasInstalledTap = true
        audioEngine.prepare()
        try audioEngine.start()
        return stream.stream
    }

    func stop() {
        if hasInstalledTap {
            audioEngine.inputNode.removeTap(onBus: 0)
            hasInstalledTap = false
        }
        audioEngine.stop()
        continuation?.finish()
        continuation = nil
    }
}

private enum ComposerSpeechTranscriberError: LocalizedError {
    case microphonePermissionDenied
    case transcriberUnavailable
    case unsupportedLocale(String)
    case missingAnalyzerFormat
    case converterUnavailable
    case conversionFailed

    var errorDescription: String? {
        switch self {
        case .microphonePermissionDenied:
            return String(localized: "speech.error.microphone_permission_denied", table: "Localizable")
        case .transcriberUnavailable:
            return String(localized: "speech.error.transcriber_unavailable", table: "Localizable")
        case .unsupportedLocale(let locale):
            return String(format: String(localized: "speech.error.unsupported_locale", table: "Localizable"), locale)
        case .missingAnalyzerFormat:
            return String(localized: "speech.error.missing_analyzer_format", table: "Localizable")
        case .converterUnavailable:
            return String(localized: "speech.error.converter_unavailable", table: "Localizable")
        case .conversionFailed:
            return String(localized: "speech.error.conversion_failed", table: "Localizable")
        }
    }
}

private extension AVAudioFormat {
    func niumaMatches(_ other: AVAudioFormat) -> Bool {
        sampleRate == other.sampleRate
            && channelCount == other.channelCount
            && commonFormat == other.commonFormat
            && isInterleaved == other.isInterleaved
    }
}
