import SwiftUI

/// "You copied something": offered when the app opens after a copy in another
/// app. The system Paste button sends it without an "Allow Paste?" prompt.
struct ClipboardOfferBanner: View {
    @ObservedObject var model: SyncModel

    var body: some View {
        HStack(spacing: 12) {
            Image(systemName: "doc.on.clipboard.fill")
                .font(.title3)
                .foregroundStyle(Theme.accentText)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text("You copied something")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Theme.text)
                Text("Send it to your paired devices?")
                    .font(.caption)
                    .foregroundStyle(Theme.textSecondary)
            }
            Spacer(minLength: 8)
            PasteControl { providers in model.pasteAndSend(providers) }
                .frame(width: 104, height: 36)
            Button { model.dismissClipboardOffer() } label: {
                Image(systemName: "xmark")
                    .font(.footnote.weight(.semibold))
                    .foregroundStyle(Theme.textMuted)
                    .frame(width: 32, height: 32)
                    // A 44-point target.
                    .contentShape(Rectangle().inset(by: -6))
            }
            .buttonStyle(.plain)
            .accessibilityLabel("Dismiss")
        }
        .card(Theme.cardRaised)
    }
}
