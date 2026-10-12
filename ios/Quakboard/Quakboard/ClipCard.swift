import SwiftUI

/// One clip in the UI draft's card style: type badge, sender and time, the
/// content in an inset block, then details and Copy.
struct ClipCard: View {
    let item: HistoryItem
    let imageURL: URL?
    var isSelected = false
    /// For a file clip.
    var fileState: SyncModel.FileState?
    let onCopy: () -> Void
    let onTogglePin: () -> Void
    var onFileAction: () -> Void = {}

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            topRow
            content
            HStack {
                Text(item.sizeDescription)
                    .font(.caption2.monospaced())
                    .foregroundStyle(Theme.textMuted)
                Spacer()
                if let fileState {
                    Button(action: onFileAction) {
                        Label(fileState.actionTitle, systemImage: fileState.actionImage)
                    }
                    .buttonStyle(ThemeButtonStyle(isPrimary: fileState == .remote))
                } else {
                    copyButton
                }
            }
        }
        .card(item.isPinned ? Theme.cardRaised : Theme.card)
        .overlay {
            if isSelected {
                RoundedRectangle(cornerRadius: 14).stroke(Theme.accentText, lineWidth: 2)
            }
        }
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var text: String {
        item.text ?? ""
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
                    .fill(item.swatch ?? Theme.inset)
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
                if let host = item.host {
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
            Text(item.trimmedText)
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
        case .file:
            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 12) {
                    Image(systemName: "doc.fill")
                        .font(.title3)
                        .foregroundStyle(Theme.accentText)
                        .frame(width: 40, height: 40)
                        .background(Theme.cardRaised, in: RoundedRectangle(cornerRadius: 10))
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(item.title)
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(Theme.text)
                            .lineLimit(2)
                        Text((fileState ?? .remote).status(origin: item.fromName))
                            .font(.caption)
                            .foregroundStyle(Theme.textMuted)
                    }
                }
                if case .fetching(let fraction) = fileState {
                    ProgressView(value: fraction)
                        .tint(Theme.accentText)
                }
            }
            .insetBlock()
        }
    }

    /// The accent button on pinned cards, outlined otherwise.
    private var copyButton: some View {
        Button(action: onCopy) {
            Label("Copy", systemImage: "doc.on.doc")
        }
        .buttonStyle(ThemeButtonStyle(isPrimary: item.isPinned))
    }
}
