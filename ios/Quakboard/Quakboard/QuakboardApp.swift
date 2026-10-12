import SwiftUI

@main
struct QuakboardApp: App {
    // One for the whole app: every iPad window shares the same sync node and
    // history, instead of each trying to listen on the same port.
    @StateObject private var model = SyncModel()
    // The app's phase, not a window's: background only once every window is.
    @Environment(\.scenePhase) private var scenePhase

    var body: some Scene {
        WindowGroup {
            ContentView(model: model)
        }
        .onChange(of: scenePhase) { phase in
            model.setAppActive(phase != .background)
        }
    }
}
