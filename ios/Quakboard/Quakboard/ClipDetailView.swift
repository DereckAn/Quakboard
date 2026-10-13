import SwiftUI

/// One clip in full: heading and actions, its details, then the content.
/// Pushed on iPhone; the pane beside the list on iPad.
struct ClipDetailView: View {
    @ObservedObject var model: SyncModel
    let item: HistoryItem
    let onDeleted: () -> Void

    @State private var isConfirmingDelete = false

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                ViewThatFits(in: .horizontal) {
                    HStack(alignment: .top, spacing: 16) {
                        heading
                        Spacer(minLength: 8)
                        actions
                    }
                    VStack(alignment: .leading, spacing: 14) {
                        heading
                        actions
                    }
                }
                details
                content
            }
            .padding(20)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 24))
            .overlay(RoundedRectangle(cornerRadius: 24).stroke(Theme.border))
            .padding(16)
        }
        .background(Theme.background)
        .navigationBarTitleDisplayMode(.inline)
        .confirmationDialog("Delete this clip?", isPresented: $isConfirmingDelete, titleVisibility: .visible) {
            Button("Delete", role: .destructive) {
                model.delete(clip)
                onDeleted()
            }
        } message: {
            Text("It's removed from this device only.")
        }
    }

    /// The latest version, so pinning shows right away.
    private var clip: HistoryItem {
        model.item(withID: item.id) ?? item
    }

    private var heading: some View {
        HStack(alignment: .top, spacing: 14) {
            Image(systemName: clip.systemImage)
                .font(.system(size: 22, weight: .semibold))
                .foregroundStyle(Theme.onAccent)
                .frame(width: 48, height: 48)
                .background(Theme.accent, in: RoundedRectangle(cornerRadius: 14))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 4) {
                Text(clip.title)
                    .font(.title2.weight(.bold))
                    .foregroundStyle(Theme.text)
                    .lineLimit(2)
                Text(subtitle)
                    .font(.subheadline)
                    .foregroundStyle(Theme.textSecondary)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }

    private var subtitle: String {
        let when = clip.createdAt.formatted(.relative(presentation: .named))
        return clip.fromName.map { "From \($0) · \(when)" } ?? when.capitalized(with: .current)
    }

    private var actions: some View {
        HStack(spacing: 8) {
            if let fileState = model.fileState(of: clip) {
                Button { model.fileAction(clip) } label: {
                    Label(fileState.actionTitle, systemImage: fileState.actionImage)
                }
                .buttonStyle(ThemeButtonStyle(isPrimary: true))
            } else {
                Button { model.copy(clip) } label: {
                    Label("Copy", systemImage: "doc.on.doc")
                }
                .buttonStyle(ThemeButtonStyle(isPrimary: true))
                .keyboardShortcut("c", modifiers: .command)
            }
            shareButton
            iconButton(clip.isPinned ? "pin.fill" : "pin", label: "Pin", isOn: clip.isPinned) {
                model.togglePin(clip)
            }
            iconButton("trash", label: "Delete") { isConfirmingDelete = true }
        }
    }

    @ViewBuilder
    private var shareButton: some View {
        if case .saved(let url) = model.fileState(of: clip) {
            ShareLink(item: url) { iconLabel("square.and.arrow.up", isOn: false) }
                .accessibilityLabel("Share")
        } else if clip.kind == .file {
            // Nothing to share until the file is here.
            EmptyView()
        } else if let url = model.imageURL(of: clip) {
            ShareLink(item: url) { iconLabel("square.and.arrow.up", isOn: false) }
                .accessibilityLabel("Share")
        } else {
            ShareLink(item: clip.text ?? "") { iconLabel("square.and.arrow.up", isOn: false) }
                .accessibilityLabel("Share")
        }
    }

    private func iconButton(_ systemImage: String, label: String, isOn: Bool = false, action: @escaping () -> Void) -> some View {
        Button(action: action) { iconLabel(systemImage, isOn: isOn) }
            .buttonStyle(.plain)
            .accessibilityLabel(label)
            .accessibilityAddTraits(isOn ? .isSelected : [])
    }

    private func iconLabel(_ systemImage: String, isOn: Bool) -> some View {
        Image(systemName: systemImage)
            .font(.system(size: 15, weight: .medium))
            .foregroundStyle(isOn ? Theme.onAccent : Theme.textSecondary)
            .frame(width: 36, height: 36)
            .background(isOn ? Theme.accent : Theme.cardRaised, in: RoundedRectangle(cornerRadius: 10))
            .overlay(RoundedRectangle(cornerRadius: 10).stroke(isOn ? Color.clear : Theme.border))
            // A 44-point target around the 36-point button.
            .contentShape(Rectangle().inset(by: -4))
    }

    private var details: some View {
        LazyVGrid(columns: [GridItem(.adaptive(minimum: 120), alignment: .leading)], alignment: .leading, spacing: 12) {
            detail("Format", clip.formatName)
            detail("Sent by", clip.fromName ?? "—")
            detail("Captured", clip.createdAt.formatted(date: .abbreviated, time: .shortened))
            detail("Size", size)
        }
        .padding(12)
        .background(Theme.inset, in: RoundedRectangle(cornerRadius: 16))
    }

    private func detail(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(label.uppercased())
                .font(.caption2.monospaced().weight(.semibold))
                .tracking(1)
                .foregroundStyle(Theme.textMuted)
            Text(value)
                .font(.subheadline.weight(.medium))
                .foregroundStyle(Theme.text)
        }
        .accessibilityElement(children: .combine)
    }

    private var size: String {
        guard let url = model.imageURL(of: clip),
              let bytes = try? url.resourceValues(forKeys: [.fileSizeKey]).fileSize else {
            return clip.sizeDescription
        }
        return "PNG • " + ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
    }

    @ViewBuilder
    private var content: some View {
        switch clip.kind {
        case .image:
            if let url = model.imageURL(of: clip) {
                HistoryImage(url: url, maxPixels: 2048)
                    .clipShape(RoundedRectangle(cornerRadius: 16))
            }
        case .color:
            HStack(spacing: 16) {
                RoundedRectangle(cornerRadius: 16)
                    .fill(clip.swatch ?? Theme.inset)
                    .frame(width: 96, height: 96)
                    .overlay(RoundedRectangle(cornerRadius: 16).stroke(Theme.border))
                    .accessibilityHidden(true)
                Text(clip.title)
                    .font(.title2.monospaced().weight(.semibold))
                    .foregroundStyle(Theme.text)
                    .textSelection(.enabled)
            }
        case .link:
            VStack(alignment: .leading, spacing: 10) {
                Text(clip.trimmedText)
                    .font(.callout.monospaced())
                    .foregroundStyle(Theme.text)
                    .textSelection(.enabled)
                if let url = URL(string: clip.trimmedText.trimmingCharacters(in: .whitespaces)), url.scheme != nil {
                    Link(destination: url) {
                        Label("Open link", systemImage: "arrow.up.right.square")
                    }
                    .buttonStyle(ThemeButtonStyle())
                }
            }
            .insetBlock()
        case .code:
            codeBox
        case .file:
            let state = model.fileState(of: clip) ?? .remote
            VStack(alignment: .leading, spacing: 12) {
                HStack(spacing: 14) {
                    Image(systemName: "doc.fill")
                        .font(.system(size: 34))
                        .foregroundStyle(Theme.accentText)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 4) {
                        Text(clip.title)
                            .font(.headline)
                            .foregroundStyle(Theme.text)
                            .textSelection(.enabled)
                        Text(state.status(origin: clip.fromName))
                            .font(.subheadline)
                            .foregroundStyle(Theme.textSecondary)
                    }
                }
                if case .fetching(let fraction) = state {
                    ProgressView(value: fraction)
                        .tint(Theme.accentText)
                }
                if state == .remote {
                    Text("Fetch downloads it from \(clip.fromName ?? "the other device"), which must be on and on this network.")
                        .font(.caption)
                        .foregroundStyle(Theme.textMuted)
                }
            }
            .insetBlock()
        case .text:
            Text(clip.text ?? "")
                .font(.body)
                .foregroundStyle(Theme.text)
                .textSelection(.enabled)
                .insetBlock()
        }
    }

    /// Line numbers beside the code, which scrolls sideways instead of
    /// wrapping so the two columns stay aligned.
    private var codeBox: some View {
        let lineCount = clip.trimmedText.components(separatedBy: "\n").count
        return ScrollView(.horizontal) {
            HStack(alignment: .top, spacing: 14) {
                Text((1...lineCount).map(String.init).joined(separator: "\n"))
                    .foregroundStyle(Theme.codeLineNumber)
                    .multilineTextAlignment(.trailing)
                    .accessibilityHidden(true)
                Text(clip.trimmedText)
                    .foregroundStyle(Theme.codeText)
                    .textSelection(.enabled)
            }
            .font(.system(.callout, design: .monospaced))
            .fixedSize()
            .padding(14)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.codeBackground, in: RoundedRectangle(cornerRadius: 16))
    }
}
