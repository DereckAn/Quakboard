import SwiftUI

@MainActor
final class ShareStatus: ObservableObject {
    enum State {
        case sending
        case sent(UInt32)
        case failed(String)
    }

    @Published var state = State.sending
}

/// The small card shown over the app that shared.
struct ShareView: View {
    @ObservedObject var status: ShareStatus
    let onDone: () -> Void

    var body: some View {
        VStack(spacing: 14) {
            switch status.state {
            case .sending:
                ProgressView()
                    .controlSize(.large)
                Text("Sending to your devices…")
            case .sent(let count):
                Image(systemName: "checkmark.circle.fill")
                    .font(.system(size: 40))
                    .foregroundStyle(.green)
                    .accessibilityHidden(true)
                Text(count == 1 ? "Sent to 1 device" : "Sent to \(count) devices")
            case .failed(let message):
                Image(systemName: "exclamationmark.triangle.fill")
                    .font(.system(size: 40))
                    .foregroundStyle(.orange)
                    .accessibilityHidden(true)
                Text(message)
                    .multilineTextAlignment(.center)
            }
            Button("Done", action: onDone)
                .buttonStyle(.borderedProminent)
        }
        .font(.headline)
        .padding(24)
        .frame(maxWidth: 320)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 20))
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color.black.opacity(0.25))
        .accessibilityElement(children: .contain)
    }
}
