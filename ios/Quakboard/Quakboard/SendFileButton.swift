import SwiftUI

/// Picks a file in the Files app and offers it to the paired devices.
struct SendFileButton: View {
    @ObservedObject var model: SyncModel

    @State private var isPicking = false

    var body: some View {
        Button { isPicking = true } label: {
            Image(systemName: "doc.badge.plus")
                .font(.system(size: 16, weight: .medium))
                .foregroundStyle(Theme.textSecondary)
                .frame(width: 38, height: 38)
                .background(Theme.card, in: RoundedRectangle(cornerRadius: 12))
                .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.border))
                // A 44-point target.
                .contentShape(Rectangle().inset(by: -3))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("Send a file")
        .fileImporter(isPresented: $isPicking, allowedContentTypes: [.item]) { result in
            if case .success(let url) = result {
                model.offerFile(at: url)
            }
        }
    }
}
