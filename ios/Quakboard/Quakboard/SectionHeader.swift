import SwiftUI

/// A section title in the draft's style: sand icon, title, and a count.
struct SectionHeader: View {
    let title: String
    let systemImage: String
    var count: Int?

    var body: some View {
        HStack {
            HStack(spacing: 6) {
                Image(systemName: systemImage)
                    .foregroundStyle(Theme.accentText)
                    .accessibilityHidden(true)
                Text(title)
                    .foregroundStyle(Theme.text)
            }
            .font(.subheadline.weight(.semibold))
            Spacer()
            if let count {
                Text("\(count) \(count == 1 ? "ITEM" : "ITEMS")")
                    .font(.caption2.monospaced().weight(.semibold))
                    .tracking(1)
                    .foregroundStyle(Theme.textMuted)
            }
        }
        .padding(.top, 8)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}
