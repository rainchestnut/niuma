import SwiftUI

struct StatusBadge: View {
    let title: String
    let tone: Tone

    var body: some View {
        Text(title)
            .font(.caption2.weight(.semibold))
            .padding(.horizontal, 9)
            .padding(.vertical, 5)
            .background(tone.background, in: Capsule())
            .foregroundStyle(tone.foreground)
            .overlay(
                Capsule()
                    .stroke(tone.foreground.opacity(0.16), lineWidth: 1)
            )
    }

    enum Tone {
        case positive
        case warning
        case critical
        case neutral

        var background: Color {
            switch self {
            case .positive:
                return NiumaPalette.accentSoft
            case .warning:
                return NiumaPalette.warningSoft
            case .critical:
                return NiumaPalette.criticalSoft
            case .neutral:
                return NiumaPalette.neutralSoft
            }
        }

        var foreground: Color {
            switch self {
            case .positive:
                return NiumaPalette.accent
            case .warning:
                return NiumaPalette.warning
            case .critical:
                return NiumaPalette.critical
            case .neutral:
                return NiumaPalette.ink
            }
        }
    }
}

struct MetricCard: View {
    let title: String
    let value: String
    let detail: String
    let accent: Color

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                Circle()
                    .fill(accent)
                    .frame(width: 8, height: 8)
                Text(title)
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(NiumaPalette.mutedInk)
            }
            Text(value)
                .font(.title3.weight(.bold))
                .foregroundStyle(NiumaPalette.ink)
            Text(detail)
                .font(.footnote)
                .foregroundStyle(NiumaPalette.mutedInk)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding()
        .background(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .fill(NiumaPalette.card)
        )
        .overlay(
            RoundedRectangle(cornerRadius: 22, style: .continuous)
                .stroke(accent.opacity(0.22), lineWidth: 1)
        )
        .shadow(color: NiumaPalette.shadow, radius: 16, x: 0, y: 10)
    }
}

struct SurfaceCard<Content: View>: View {
    let title: String
    let subtitle: String?
    @ViewBuilder let content: Content

    init(title: String, subtitle: String? = nil, @ViewBuilder content: () -> Content) {
        self.title = title
        self.subtitle = subtitle
        self.content = content()
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            VStack(alignment: .leading, spacing: 4) {
                Text(title)
                    .font(.headline)
                    .foregroundStyle(NiumaPalette.ink)
                if let subtitle {
                    Text(subtitle)
                        .font(.footnote)
                        .foregroundStyle(NiumaPalette.mutedInk)
                }
            }
            content
        }
        .padding(20)
        .background(NiumaPalette.card, in: RoundedRectangle(cornerRadius: 28, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .stroke(NiumaPalette.border, lineWidth: 1)
        )
        .shadow(color: NiumaPalette.shadow, radius: 18, x: 0, y: 12)
    }
}

extension ThreadStatus {
    var badge: (String, StatusBadge.Tone) {
        badge(for: .chinese)
    }

    var compactBadge: (String, StatusBadge.Tone)? {
        compactBadge(for: .chinese)
    }

    func badge(for language: AppLanguage) -> (String, StatusBadge.Tone) {
        switch self {
        case .notLoaded:
            return (title(for: language), .neutral)
        case .created:
            return (title(for: language), .neutral)
        case .idle:
            return (title(for: language), .neutral)
        case .running:
            return (title(for: language), .positive)
        case .waitingApproval:
            return (title(for: language), .warning)
        case .pending:
            return (title(for: language), .warning)
        case .threadStarted:
            return (title(for: language), .positive)
        case .completed:
            return (title(for: language), .neutral)
        case .failed:
            return (title(for: language), .critical)
        case .systemError:
            return (title(for: language), .critical)
        case .cancelled:
            return (title(for: language), .neutral)
        case .closed:
            return (title(for: language), .neutral)
        case .archived:
            return (title(for: language), .neutral)
        case .unknown:
            return (title(for: language), .neutral)
        }
    }

    /// Compacts Badge.
    func compactBadge(for language: AppLanguage) -> (String, StatusBadge.Tone)? {
        switch self {
        case .notLoaded, .idle, .created:
            return nil
        default:
            return badge(for: language)
        }
    }
}

extension RealtimeConnectionState {
    var badge: (String, StatusBadge.Tone) {
        badge(for: .chinese)
    }

    func badge(for language: AppLanguage) -> (String, StatusBadge.Tone) {
        switch self {
        case .disconnected:
            return (title(for: language), .critical)
        case .authenticating:
            return (title(for: language), .warning)
        case .connected:
            return (title(for: language), .positive)
        case .degraded:
            return (title(for: language), .warning)
        case .retrying:
            return (title(for: language), .warning)
        }
    }
}

extension DeviceLifecycleState {
    var badge: (String, StatusBadge.Tone) {
        badge(for: .chinese)
    }

    func badge(for language: AppLanguage) -> (String, StatusBadge.Tone) {
        switch self {
        case .uninitialized:
            return (title(for: language), .warning)
        case .deviceRegistered:
            return (title(for: language), .neutral)
        case .unpaired:
            return (title(for: language), .warning)
        case .pairing:
            return (title(for: language), .warning)
        case .paired:
            return (title(for: language), .positive)
        case .pairFailed:
            return (title(for: language), .critical)
        }
    }
}
