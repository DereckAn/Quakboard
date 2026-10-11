import SwiftUI

/// One clip in the UI draft's card style: type badge, sender and time, the
/// content in an inset block, then details and Copy.
struct ClipCard: View {
    let item: HistoryItem
    let imageURL: URL?
    let onCopy: () -> Void
    let onTogglePin: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            topRow
            content
            HStack {
                Text(details)
                    .font(.caption2.monospaced())
                    .foregroundStyle(Theme.textMuted)
                Spacer()
                copyButton
            }
        }
        .card(item.isPinned ? Theme.cardRaised : Theme.card)
    }

    private var text: String {
        item.text ?? ""
    }

    /// Without the trailing newline most copied code ends with, which would
    /// show as an empty line.
    private var trimmedText: String {
        String(text.reversed().drop { $0.isNewline }.reversed())
    }

    private var topRow: some View {
        HStack(spacing: 6) {
            Text(item.kind.rawValue.uppercased())
                .font(.caption2.monospaced().weight(.semibold))
                .foregroundStyle(Theme.accentText)
                .padding(.horizontal, 6)
                .padding(.vertical, 2)
                .overlay(RoundedRectangle(cornerRadius: 5).stroke(Theme.border))
            if let from = item.fromName {
                Label(from, systemImage: "laptopcomputer")
                    .font(.caption2.monospaced())
                    .foregroundStyle(Theme.textMuted)
                    .lineLimit(1)
            }
            Text("· \(item.createdAt.formatted(.relative(presentation: .named, unitsStyle: .abbreviated)))")
                .font(.caption)
                .foregroundStyle(Theme.textMuted)
                .lineLimit(1)
            Spacer(minLength: 4)
            Button(action: onTogglePin) {
                Image(systemName: item.isPinned ? "pin.fill" : "pin")
                    .font(.system(size: 13, weight: item.isPinned ? .semibold : .regular))
                    .rotationEffect(.degrees(item.isPinned ? 0 : 45))
                    .foregroundStyle(item.isPinned ? Theme.onAccent : Theme.textMuted)
                    .frame(width: 28, height: 28)
                    .background(item.isPinned ? Theme.accent : Theme.inset, in: RoundedRectangle(cornerRadius: 8))
                    .overlay(RoundedRectangle(cornerRadius: 8).stroke(item.isPinned ? Color.clear : Theme.border))
                    .animation(.spring(response: 0.25, dampingFraction: 0.8), value: item.isPinned)
                    // A 44-point target around the small icon.
                    .contentShape(Rectangle().inset(by: -8))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Pin")
            .accessibilityAddTraits(item.isPinned ? .isSelected : [])
        }
    }

    @ViewBuilder
    private var content: some View {
        switch item.kind {
        case .image:
            if let imageURL {
                HistoryImage(url: imageURL, maxPixels: 600)
                    .frame(maxWidth: .infinity, maxHeight: 180)
                    .clipShape(RoundedRectangle(cornerRadius: 10))
            }
        case .color:
            HStack(spacing: 12) {
                RoundedRectangle(cornerRadius: 12)
                    .fill(Self.swatch(for: text) ?? Theme.inset)
                    .frame(width: 56, height: 56)
                    .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.border))
                    .accessibilityHidden(true)
                Text(text.trimmingCharacters(in: .whitespacesAndNewlines))
                    .font(.headline.monospaced())
                    .foregroundStyle(Theme.text)
                    .lineLimit(2)
            }
        case .link:
            VStack(alignment: .leading, spacing: 2) {
                if let host = URL(string: text.trimmingCharacters(in: .whitespacesAndNewlines))?.host {
                    Text(host)
                        .font(.caption.monospaced().weight(.semibold))
                        .foregroundStyle(Theme.accentText)
                }
                Text(text)
                    .font(.footnote.monospaced())
                    .foregroundStyle(Theme.textSecondary)
                    .lineLimit(2)
            }
            .insetBlock()
        case .code:
            Text(trimmedText)
                .font(.caption.monospaced())
                .foregroundStyle(Theme.text)
                .lineLimit(8)
                .insetBlock()
        case .text:
            Text(text)
                .font(.subheadline)
                .foregroundStyle(Theme.text)
                .lineLimit(4)
                .insetBlock()
        }
    }

    private var details: String {
        switch item.kind {
        case .image:
            return "PNG"
        case .code:
            let lines = trimmedText.split(separator: "\n", omittingEmptySubsequences: false).count
            return "\(lines) lines • \(text.utf8.count) bytes"
        case .text, .link, .color:
            return "\(text.count) chars"
        }
    }

    /// The draft's primary (sand) button on pinned cards, outlined otherwise.
    private var copyButton: some View {
        Button(action: onCopy) {
            Label("Copy", systemImage: "doc.on.doc")
        }
        .buttonStyle(ThemeButtonStyle(isPrimary: item.isPinned))
    }

    /// `#RGB` or `#RRGGBB`. The desktop also detects rgb(), hsl() and
    /// oklch(); those show no swatch.
    private static func swatch(for text: String) -> Color? {
        var hex = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard hex.hasPrefix("#") else { return nil }
        hex.removeFirst()
        if hex.count == 3 {
            hex = hex.map { "\($0)\($0)" }.joined()
        }
        guard hex.count == 6, let value = UInt32(hex, radix: 16) else { return nil }
        return Color(hex: value)
    }
}

private extension View {
    func insetBlock() -> some View {
        frame(maxWidth: .infinity, alignment: .leading)
            .padding(10)
            .background(Theme.inset, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border.opacity(0.6)))
    }
}
