import Foundation

/// Reads user-facing copy from Apple's String Catalog while honoring the
/// language selected inside Niuma settings.
nonisolated enum L10n {
    static func string(_ key: String, language: AppLanguage) -> String {
        String(
            localized: String.LocalizationValue(key),
            table: "Localizable",
            bundle: .main,
            locale: Locale(identifier: language.localeIdentifier)
        )
    }

    static func string(_ key: String, language: AppLanguage, _ arguments: CVarArg...) -> String {
        string(key, language: language, arguments)
    }

    static func string(_ key: String, language: AppLanguage, _ arguments: [CVarArg]) -> String {
        String(
            format: string(key, language: language),
            locale: Locale(identifier: language.localeIdentifier),
            arguments: arguments
        )
    }

    static func processedGroupTitle(durationSeconds: Int?, language: AppLanguage) -> String {
        guard let durationSeconds, durationSeconds > 0 else {
            return string("thread.processed", language: language)
        }
        if durationSeconds >= 60 {
            return string(
                "thread.processed.minutes_seconds",
                language: language,
                durationSeconds / 60,
                durationSeconds % 60
            )
        }
        return string("thread.processed.seconds", language: language, durationSeconds)
    }

    static func processToolTitle(toolKey: String, language: AppLanguage) -> String {
        let key = switch toolKey {
        case "tap": "process.tool.tap"
        case "snapshot_ui": "process.tool.snapshot_ui"
        case "screenshot": "process.tool.screenshot"
        case "build_run_sim": "process.tool.build_run_sim"
        case "session_show_defaults": "process.tool.session_show_defaults"
        default: "process.tool.unknown"
        }
        return string(key, language: language)
    }

    static func diagnosticCount(_ count: Int, severity: ProcessDiagnosticSeverity, language: AppLanguage) -> String {
        let baseKey = switch severity {
        case .warning: "diagnostic.warning"
        case .error: "diagnostic.error"
        case .unknown: "diagnostic.unknown"
        }
        let suffix = count == 1 ? "one" : "other"
        return string("\(baseKey).\(suffix)", language: language, count)
    }

    static func fileFallbackLabel(for part: ContentPart, language: AppLanguage) -> String {
        if part.isImageFile {
            return string("file.kind.image", language: language)
        }
        if part.fileType == "video" || part.mimeType?.hasPrefix("video/") == true {
            return string("file.kind.video", language: language)
        }
        return string("file.kind.file", language: language)
    }

    static func transferDirectionLabel(_ direction: TransferDirection, language: AppLanguage) -> String {
        switch direction {
        case .iosToAgent:
            return string("transfer.sent", language: language)
        case .agentToIOS:
            return string("transfer.received", language: language)
        }
    }
}

extension AppLanguage {
    nonisolated var localeIdentifier: String {
        switch self {
        case .chinese:
            return "zh-Hans"
        case .english:
            return "en"
        }
    }
}
