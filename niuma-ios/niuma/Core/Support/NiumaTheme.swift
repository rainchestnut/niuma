import SwiftUI
import UIKit

/// Creates a SwiftUI color backed by separate light and dark UIKit colors.
private func dynamicColor(light: UIColor, dark: UIColor) -> Color {
    Color(
        uiColor: UIColor { traits in
            traits.userInterfaceStyle == .dark ? dark : light
        }
    )
}

enum NiumaPalette {
    static let ink = dynamicColor(
        light: UIColor(red: 0.09, green: 0.10, blue: 0.12, alpha: 1),
        dark: UIColor(red: 0.95, green: 0.96, blue: 0.97, alpha: 1)
    )
    static let mutedInk = dynamicColor(
        light: UIColor(red: 0.45, green: 0.47, blue: 0.50, alpha: 1),
        dark: UIColor(red: 0.67, green: 0.69, blue: 0.72, alpha: 1)
    )

    static let accent = dynamicColor(
        light: UIColor(red: 0.08, green: 0.53, blue: 0.30, alpha: 1),
        dark: UIColor(red: 0.24, green: 0.73, blue: 0.45, alpha: 1)
    )
    static let accentSoft = dynamicColor(
        light: UIColor(red: 0.90, green: 0.97, blue: 0.92, alpha: 1),
        dark: UIColor(red: 0.11, green: 0.23, blue: 0.16, alpha: 1)
    )
    static let info = dynamicColor(
        light: UIColor(red: 0.19, green: 0.48, blue: 0.94, alpha: 1),
        dark: UIColor(red: 0.43, green: 0.64, blue: 0.99, alpha: 1)
    )
    static let infoSoft = dynamicColor(
        light: UIColor(red: 0.91, green: 0.95, blue: 1.0, alpha: 1),
        dark: UIColor(red: 0.11, green: 0.18, blue: 0.28, alpha: 1)
    )
    static let warning = dynamicColor(
        light: UIColor(red: 0.80, green: 0.45, blue: 0.13, alpha: 1),
        dark: UIColor(red: 0.95, green: 0.67, blue: 0.34, alpha: 1)
    )
    static let warningSoft = dynamicColor(
        light: UIColor(red: 1.0, green: 0.95, blue: 0.89, alpha: 1),
        dark: UIColor(red: 0.26, green: 0.19, blue: 0.10, alpha: 1)
    )
    static let critical = dynamicColor(
        light: UIColor(red: 0.83, green: 0.27, blue: 0.24, alpha: 1),
        dark: UIColor(red: 1.0, green: 0.51, blue: 0.48, alpha: 1)
    )
    static let criticalSoft = dynamicColor(
        light: UIColor(red: 1.0, green: 0.92, blue: 0.91, alpha: 1),
        dark: UIColor(red: 0.30, green: 0.14, blue: 0.13, alpha: 1)
    )
    static let neutralSoft = dynamicColor(
        light: UIColor(red: 0.95, green: 0.95, blue: 0.96, alpha: 1),
        dark: UIColor(red: 0.17, green: 0.18, blue: 0.20, alpha: 1)
    )

    static let canvas = dynamicColor(
        light: .white,
        dark: UIColor(red: 0.05, green: 0.06, blue: 0.08, alpha: 1)
    )
    static let card = dynamicColor(
        light: .white,
        dark: UIColor(red: 0.09, green: 0.10, blue: 0.12, alpha: 1)
    )
    static let raisedCard = dynamicColor(
        light: UIColor(red: 0.97, green: 0.97, blue: 0.98, alpha: 1),
        dark: UIColor(red: 0.13, green: 0.14, blue: 0.17, alpha: 1)
    )
    static let border = dynamicColor(
        light: UIColor.black.withAlphaComponent(0.06),
        dark: UIColor.white.withAlphaComponent(0.08)
    )
    static let shadow = dynamicColor(
        light: UIColor.black.withAlphaComponent(0.06),
        dark: UIColor.black.withAlphaComponent(0.22)
    )

    static let darkButton = dynamicColor(
        light: UIColor(red: 0.14, green: 0.14, blue: 0.15, alpha: 1),
        dark: UIColor(red: 0.93, green: 0.94, blue: 0.96, alpha: 1)
    )
    static let darkButtonText = dynamicColor(
        light: .white,
        dark: UIColor(red: 0.08, green: 0.09, blue: 0.10, alpha: 1)
    )
}

private struct NiumaScreenBackgroundModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .background(NiumaPalette.canvas.ignoresSafeArea())
    }
}

private struct NiumaInputChromeModifier: ViewModifier {
    func body(content: Content) -> some View {
        content
            .padding(.horizontal, 14)
            .padding(.vertical, 12)
            .background(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .fill(NiumaPalette.card)
            )
            .overlay(
                RoundedRectangle(cornerRadius: 18, style: .continuous)
                    .stroke(NiumaPalette.border, lineWidth: 1)
            )
    }
}

/// Wraps a control group in a native glass treatment on iOS 26 and falls back
/// to material-backed chrome on earlier runtimes.
private struct NiumaGlassChromeModifier: ViewModifier {
    let cornerRadius: CGFloat
    let interactive: Bool

    func body(content: Content) -> some View {
        content
            .background(glassBackground)
    }

    @ViewBuilder
    private var glassBackground: some View {
        if #available(iOS 26.0, *) {
            Color.clear
                .glassEffect(
                    interactive ? .regular.interactive() : .regular,
                    in: .rect(cornerRadius: cornerRadius)
                )
        } else {
            RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                .fill(.ultraThinMaterial)
                .overlay(
                    RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                        .stroke(NiumaPalette.border, lineWidth: 1)
                )
        }
    }
}

struct NiumaPrimaryButtonStyle: ButtonStyle {
    /// Builds the styled SwiftUI control body.
    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.headline)
            .foregroundStyle(NiumaPalette.darkButtonText)
            .padding(.horizontal, 18)
            .padding(.vertical, 14)
            .frame(maxWidth: .infinity)
            .background(
                RoundedRectangle(cornerRadius: 22, style: .continuous)
                    .fill(NiumaPalette.darkButton)
            )
            .shadow(color: NiumaPalette.shadow, radius: 16, x: 0, y: 10)
            .opacity(configuration.isPressed ? 0.92 : 1)
            .scaleEffect(configuration.isPressed ? 0.99 : 1)
    }
}

extension View {
    /// Applies the standard Niuma screen background modifier.
    func niumaScreenBackground() -> some View {
        modifier(NiumaScreenBackgroundModifier())
    }

    /// Applies the standard Niuma input chrome modifier.
    func niumaInputChrome() -> some View {
        modifier(NiumaInputChromeModifier())
    }

    /// Applies the standard Niuma glass chrome modifier.
    func niumaGlassChrome(cornerRadius: CGFloat = 24, interactive: Bool = false) -> some View {
        modifier(NiumaGlassChromeModifier(cornerRadius: cornerRadius, interactive: interactive))
    }
}
