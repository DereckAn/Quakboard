import SwiftUI

/// The UI drafts' palette: dark slate with a warm sand accent, and light
/// cream with a navy accent (from the iPad draft), following the appearance.
enum Theme {
    static let background = dynamic(dark: 0x0C141F, light: 0xFFF8F2)
    static let card = dynamic(dark: 0x141C28, light: 0xFFFFFF)
    /// Pinned cards and unselected chips.
    static let cardRaised = dynamic(dark: 0x1A2332, light: 0xFEF2E2)
    /// Content blocks inside a card.
    static let inset = dynamic(dark: 0x070F1A, light: 0xF8ECDD)
    static let border = dynamic(dark: 0x323A46, light: 0xE7DBC9)
    static let text = dynamic(dark: 0xDBE3F4, light: 0x201B12)
    static let textSecondary = dynamic(dark: 0xB0BCCB, light: 0x44474D)
    /// Lighter (dark) and darker (light) than the drafts' outline color, which
    /// is too faint for small text against the cards.
    static let textMuted = dynamic(dark: 0x7D8B9A, light: 0x6E7077)
    /// Fills: buttons, the selected chip and sidebar row.
    static let accent = Color(uiAccent)
    /// The accent as text or an icon on the background.
    static let accentText = dynamic(dark: 0xDDD2BD, light: 0x1E2F46)
    /// Text and icons on an accent fill.
    static let onAccent = Color(uiOnAccent)
    /// The iPad sidebar.
    static let sidebar = dynamic(dark: 0x101824, light: 0xFEF2E2)
    /// The code box: dark in both appearances, like the drafts.
    static let codeBackground = dynamic(dark: 0x070F1A, light: 0x1E2F46)
    static let codeText = dynamic(dark: 0xDBE3F4, light: 0xFFF8F2)
    static let codeLineNumber = dynamic(dark: 0x5F6E7D, light: 0x8A97AB)

    /// UIKit controls (the Paste button) take these directly, so they also
    /// follow appearance changes.
    static let uiAccent = dynamicUI(dark: 0xDDD2BD, light: 0x1E2F46)
    static let uiOnAccent = dynamicUI(dark: 0x0C141F, light: 0xFFFFFF)

    private static func dynamic(dark: UInt32, light: UInt32) -> Color {
        Color(dynamicUI(dark: dark, light: light))
    }

    private static func dynamicUI(dark: UInt32, light: UInt32) -> UIColor {
        UIColor { traits in
            UIColor(Color(hex: traits.userInterfaceStyle == .dark ? dark : light))
        }
    }
}

extension Color {
    init(hex: UInt32) {
        self.init(
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255
        )
    }
}

extension View {
    /// A block of content inside a card.
    func insetBlock() -> some View {
        frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(Theme.inset, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border.opacity(0.6)))
    }

    /// The draft's card: a rounded fill with a thin border.
    func card(_ fill: Color = Theme.card) -> some View {
        padding(12)
            .background(fill, in: RoundedRectangle(cornerRadius: 14))
            .overlay(RoundedRectangle(cornerRadius: 14).stroke(Theme.border))
    }
}

/// The draft's small buttons: sand when primary, outlined otherwise.
struct ThemeButtonStyle: ButtonStyle {
    var isPrimary = false

    @Environment(\.isEnabled) private var isEnabled

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .font(.footnote.weight(.semibold))
            .foregroundStyle(isPrimary ? Theme.onAccent : Theme.text)
            .padding(.horizontal, 10)
            .padding(.vertical, 6)
            .background(isPrimary ? Theme.accent : Theme.cardRaised, in: RoundedRectangle(cornerRadius: 8))
            .overlay(RoundedRectangle(cornerRadius: 8).stroke(isPrimary ? Color.clear : Theme.border))
            .opacity(!isEnabled ? 0.4 : configuration.isPressed ? 0.7 : 1)
    }
}

/// The Appearance choice in Settings, saved under `storageKey`.
enum Appearance: String, CaseIterable, Identifiable {
    case system, light, dark

    static let storageKey = "appearance"

    var id: Self { self }

    var title: String {
        rawValue.capitalized
    }

    /// nil follows the system setting.
    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
}
