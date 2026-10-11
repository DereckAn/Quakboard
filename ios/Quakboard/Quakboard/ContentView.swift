import SwiftUI

struct ContentView: View {
    @StateObject private var model = SyncModel()

    var body: some View {
        TabView {
            ClipsView(model: model, isPinnedOnly: false)
                .tabItem { Label("All Clips", systemImage: "doc.on.clipboard") }
            ClipsView(model: model, isPinnedOnly: true)
                .tabItem { Label("Pinned", systemImage: "pin") }
            DevicesView(model: model)
                .tabItem { Label("Settings", systemImage: "slider.horizontal.3") }
        }
        .tint(Theme.accentText)
        .overlay(alignment: .top) { toast }
        .animation(.default, value: model.toast)
        .task { await model.start() }
    }

    @ViewBuilder
    private var toast: some View {
        if let toast = model.toast {
            Label(toast.message, systemImage: toast.isError ? "exclamationmark.circle.fill" : "checkmark.circle.fill")
                .font(.footnote.weight(.semibold))
                .foregroundStyle(Theme.text)
                .padding(.horizontal, 14)
                .padding(.vertical, 8)
                .background(Theme.cardRaised, in: Capsule())
                .overlay(Capsule().stroke(Theme.accent.opacity(0.3)))
                .shadow(color: .black.opacity(0.25), radius: 12, y: 4)
                .padding(.horizontal, 16)
                .padding(.top, 8)
                .transition(.move(edge: .top).combined(with: .opacity))
        }
    }
}
