import SwiftUI

/// The UI draft's palette: dark slate with a warm sand accent, and a light
/// counterpart that follows the system appearance.
enum Theme {
    static let background = dynamic(dark: 0x0C141F, light: 0xF6F3EE)
    static let card = dynamic(dark: 0x141C28, light: 0xFFFFFF)
    /// Pinned cards and unselected chips.
    static let cardRaised = dynamic(dark: 0x1A2332, light: 0xFBF8F3)
    /// Content blocks inside a card.
    static let inset = dynamic(dark: 0x070F1A, light: 0xF1ECE3)
    static let border = dynamic(dark: 0x323A46, light: 0xE2DACB)
    static let text = dynamic(dark: 0xDBE3F4, light: 0x1B2330)
    static let textSecondary = dynamic(dark: 0xB0BCCB, light: 0x4C5563)
    /// Lighter than the draft's outline (#5F6E7D), which is too faint for
    /// small text against the cards.
    static let textMuted = dynamic(dark: 0x7D8B9A, light: 0x7F7566)
    /// Fills: buttons, the selected chip.
    static let accent = Color(hex: 0xDDD2BD)
    /// Sand as text or an icon; darker in light mode so it stays readable.
    static let accentText = dynamic(dark: 0xDDD2BD, light: 0x7A6644)
    static let onAccent = Color(hex: 0x0C141F)

    private static func dynamic(dark: UInt32, light: UInt32) -> Color {
        Color(UIColor { traits in
            UIColor(Color(hex: traits.userInterfaceStyle == .dark ? dark : light))
        })
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
