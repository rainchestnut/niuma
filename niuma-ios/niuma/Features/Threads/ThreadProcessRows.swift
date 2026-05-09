import Foundation
import SwiftUI

struct ProcessGroupRow: View {
    @Environment(AppModel.self) private var appModel

    let group: ThreadProcessGroup
    let isExpanded: Bool
    let onToggle: () -> Void

    private var title: String {
        L10n.processedGroupTitle(
            durationSeconds: group.durationSeconds,
            language: appModel.appLanguage
        )
    }

    var body: some View {
        HStack(alignment: .top, spacing: 0) {
            VStack(alignment: .leading, spacing: 8) {
                Button {
                    onToggle()
                } label: {
                    HStack(spacing: 8) {
                        Text(title)
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(NiumaPalette.mutedInk)
                        Image(systemName: "chevron.right")
                            .font(.caption2.weight(.bold))
                            .foregroundStyle(NiumaPalette.mutedInk)
                            .rotationEffect(.degrees(isExpanded ? 90 : 0))
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 12)
                    .padding(.vertical, 9)
                    .background(
                        RoundedRectangle(cornerRadius: 14, style: .continuous)
                            .fill(NiumaPalette.raisedCard.opacity(0.72))
                    )
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)

                if isExpanded {
                    VStack(alignment: .leading, spacing: 12) {
                        ForEach(group.entries) { item in
                            ProcessTimelineEntryRow(item: item)
                        }
                    }
                    .padding(.top, 2)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)

            Spacer(minLength: 34)
        }
    }
}

private struct ProcessTimelineEntryRow: View {
    let item: ThreadMessageRenderItem

    var body: some View {
        if let payload = ProcessSummaryPayload.decode(from: item) {
            ProcessActionRow(item: item, payload: payload)
        } else {
            ProcessTextRow(item: item)
        }
    }
}

private struct ProcessTextRow: View {
    let item: ThreadMessageRenderItem

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            ThreadEntryContentView(item: item)

            Text(DateFormatting.timeAndDate.string(from: item.entry.createdAt))
                .font(.caption2)
                .foregroundStyle(NiumaPalette.mutedInk)
        }
        .padding(.horizontal, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct ProcessActionRow: View {
    @Environment(AppModel.self) private var appModel

    let item: ThreadMessageRenderItem
    let payload: ProcessSummaryPayload

    private var presentation: ProcessActionPresentation {
        ProcessActionPresentation(payload: payload, language: appModel.appLanguage)
    }

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: presentation.iconName)
                .font(.system(size: 13, weight: .semibold))
                .foregroundStyle(presentation.accent)
                .frame(width: 24, height: 24)
                .background(Circle().fill(presentation.background))

            VStack(alignment: .leading, spacing: 7) {
                HStack(alignment: .center, spacing: 8) {
                    Text(presentation.title)
                        .font(.footnote.weight(.semibold))
                        .foregroundStyle(NiumaPalette.ink)
                        .lineLimit(1)

                    Spacer(minLength: 0)
                }

                if presentation.warningCount > 0 || presentation.errorCount > 0 {
                    HStack(spacing: 8) {
                        if presentation.warningCount > 0 {
                            ProcessDiagnosticPill(
                                title: L10n.diagnosticCount(
                                    presentation.warningCount,
                                    severity: .warning,
                                    language: appModel.appLanguage
                                ),
                                tone: .warning
                            )
                        }
                        if presentation.errorCount > 0 {
                            ProcessDiagnosticPill(
                                title: L10n.diagnosticCount(
                                    presentation.errorCount,
                                    severity: .error,
                                    language: appModel.appLanguage
                                ),
                                tone: .critical
                            )
                        }
                    }
                }

                ForEach(presentation.diagnostics) { diagnostic in
                    Text(diagnostic.text)
                        .font(.caption2)
                        .foregroundStyle(diagnostic.tone.foreground)
                        .textSelection(.enabled)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if !item.entry.externalImageParts.isEmpty {
                    ExternalImageAttachmentStrip(
                        parts: item.entry.externalImageParts,
                        alignment: .leading
                    )
                }
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 9)
        .background(
            RoundedRectangle(cornerRadius: 12, style: .continuous)
                .fill(NiumaPalette.raisedCard.opacity(0.58))
        )
    }
}

private struct ProcessDiagnosticPill: View {
    let title: String
    let tone: StatusBadge.Tone

    var body: some View {
        Text(title)
            .font(.caption2.weight(.semibold))
            .foregroundStyle(tone.foreground)
            .padding(.horizontal, 8)
            .padding(.vertical, 4)
            .background(tone.background, in: Capsule())
    }
}

private struct ProcessActionPresentation {
    struct Diagnostic: Identifiable {
        let id: String
        let text: String
        let tone: StatusBadge.Tone
    }

    let title: String
    let warningCount: Int
    let errorCount: Int
    let diagnostics: [Diagnostic]
    let iconName: String
    let accent: Color
    let background: Color

    init(payload: ProcessSummaryPayload, language: AppLanguage) {
        let status = ProcessStatus(payload.status)
        let style = ProcessActionStyle(
            toolKey: payload.toolKey,
            statusTone: status.tone,
            language: language
        )
        title = style.title
        warningCount = payload.warningCount ?? payload.diagnostics.filter { $0.severity == .warning }.count
        errorCount = payload.errorCount ?? payload.diagnostics.filter { $0.severity == .error }.count
        diagnostics = payload.diagnostics.enumerated().map {
            Diagnostic(
                id: "\($0.element.severity.rawValue)-\($0.offset)",
                text: $0.element.message,
                tone: $0.element.severity.tone
            )
        }
        iconName = style.iconName
        accent = style.accent
        background = style.background
    }
}

private struct ProcessActionStyle {
    let title: String
    let iconName: String
    let accent: Color
    let background: Color

    init(toolKey: String, statusTone: StatusBadge.Tone, language: AppLanguage) {
        let baseTitle: String
        let baseIconName: String
        let baseAccent: Color
        let baseBackground: Color

        switch toolKey {
        case "tap":
            baseTitle = L10n.processToolTitle(toolKey: toolKey, language: language)
            baseIconName = "hand.tap"
            baseAccent = NiumaPalette.info
            baseBackground = NiumaPalette.infoSoft
        case "snapshot_ui":
            baseTitle = L10n.processToolTitle(toolKey: toolKey, language: language)
            baseIconName = "list.bullet.rectangle"
            baseAccent = NiumaPalette.info
            baseBackground = NiumaPalette.infoSoft
        case "screenshot":
            baseTitle = L10n.processToolTitle(toolKey: toolKey, language: language)
            baseIconName = "camera"
            baseAccent = NiumaPalette.info
            baseBackground = NiumaPalette.infoSoft
        case "build_run_sim":
            baseTitle = L10n.processToolTitle(toolKey: toolKey, language: language)
            baseIconName = "play.rectangle"
            baseAccent = NiumaPalette.accent
            baseBackground = NiumaPalette.accentSoft
        case "session_show_defaults":
            baseTitle = L10n.processToolTitle(toolKey: toolKey, language: language)
            baseIconName = "gearshape"
            baseAccent = NiumaPalette.mutedInk
            baseBackground = NiumaPalette.neutralSoft
        default:
            baseTitle = L10n.processToolTitle(toolKey: toolKey, language: language)
            baseIconName = "terminal"
            baseAccent = NiumaPalette.mutedInk
            baseBackground = NiumaPalette.neutralSoft
        }

        title = baseTitle
        iconName = baseIconName
        switch statusTone {
        case .critical:
            accent = NiumaPalette.critical
            background = NiumaPalette.criticalSoft
        default:
            accent = baseAccent
            background = baseBackground
        }
    }
}

struct ProcessSummaryPayload: Decodable {
    let kind: String
    let toolKey: String
    let status: String?
    let warningCount: Int?
    let errorCount: Int?
    let diagnostics: [ProcessDiagnosticPayload]

    enum CodingKeys: String, CodingKey {
        case kind
        case toolKey = "tool_key"
        case status
        case warningCount = "warning_count"
        case errorCount = "error_count"
        case diagnostics
    }

    static func decode(from item: ThreadMessageRenderItem) -> ProcessSummaryPayload? {
        let candidates = item.contentParts.compactMap(\.part.text) + [item.entry.text]
        for candidate in candidates {
            guard let data = candidate.data(using: .utf8),
                  let payload = try? JSONDecoder().decode(ProcessSummaryPayload.self, from: data),
                  payload.kind == "process_summary" else {
                continue
            }
            return payload
        }
        return nil
    }

    init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        kind = try container.decode(String.self, forKey: .kind)
        toolKey = try container.decode(String.self, forKey: .toolKey)
        status = try container.decodeIfPresent(String.self, forKey: .status)
        warningCount = try container.decodeIfPresent(Int.self, forKey: .warningCount)
        errorCount = try container.decodeIfPresent(Int.self, forKey: .errorCount)
        diagnostics = try container.decodeIfPresent([ProcessDiagnosticPayload].self, forKey: .diagnostics) ?? []
    }
}

struct ProcessDiagnosticPayload: Decodable {
    let severity: ProcessDiagnosticSeverity
    let message: String
}

nonisolated enum ProcessDiagnosticSeverity: String, Decodable {
    case warning
    case error
    case unknown

    init(from decoder: Decoder) throws {
        let container = try decoder.singleValueContainer()
        let rawValue = try container.decode(String.self)
        self = ProcessDiagnosticSeverity(rawValue: rawValue) ?? .unknown
    }

    var tone: StatusBadge.Tone {
        switch self {
        case .error:
            return .critical
        case .warning:
            return .warning
        case .unknown:
            return .neutral
        }
    }
}

private struct ProcessStatus {
    let tone: StatusBadge.Tone

    init(_ rawStatus: String?) {
        switch rawStatus {
        case "succeeded", "completed", "success":
            tone = .positive
        case "failed", "error":
            tone = .critical
        case let status? where !status.isEmpty:
            tone = .neutral
        default:
            tone = .neutral
        }
    }
}
