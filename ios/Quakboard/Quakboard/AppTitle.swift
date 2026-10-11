import SwiftUI

/// The top of every tab: the app tile, "Quakboard" and where you are.
struct AppTitle: View {
    let subtitle: String

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "doc.on.clipboard")
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(Theme.accentText)
                .frame(width: 32, height: 32)
                .background(Theme.cardRaised, in: RoundedRectangle(cornerRadius: 10))
                .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 1) {
                Text("Quakboard")
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(Theme.text)
                Text(subtitle)
                    .font(.caption.weight(.medium))
                    .foregroundStyle(Theme.textSecondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

extension View {
    /// The bar the title sits in, pinned above the scrolling content.
    func headerBar() -> some View {
        frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, 10)
            .background(Theme.background)
            .overlay(alignment: .bottom) {
                Theme.border.frame(height: 0.5)
            }
    }
}
