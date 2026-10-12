import SwiftUI

struct ContentView: View {
    @ObservedObject var model: SyncModel
    @AppStorage(Appearance.storageKey) private var appearance = Appearance.system
    @Environment(\.horizontalSizeClass) private var sizeClass

    var body: some View {
        Group {
            if usesWorkspace {
                WorkspaceView(model: model)
            } else {
                TabView {
                    ClipsView(model: model, isPinnedOnly: false)
                        .tabItem { Label("All Clips", systemImage: "doc.on.clipboard") }
                    ClipsView(model: model, isPinnedOnly: true)
                        .tabItem { Label("Pinned", systemImage: "pin") }
                    SettingsView(model: model)
                        .tabItem { Label("Settings", systemImage: "slider.horizontal.3") }
                }
            }
        }
        .tint(Theme.accentText)
        .overlay(alignment: .top) { toast }
        .animation(.default, value: model.toast)
        .preferredColorScheme(appearance.colorScheme)
        .task { await model.start() }
    }

    /// The iPad layout needs a full-width iPad. Slide Over, a narrow Split
    /// View and big iPhones in landscape keep the iPhone layout.
    private var usesWorkspace: Bool {
        UIDevice.current.userInterfaceIdiom == .pad && sizeClass == .regular
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
