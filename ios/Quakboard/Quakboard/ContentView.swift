import SwiftUI

struct ContentView: View {
    @StateObject private var model = SyncModel()

    var body: some View {
        TabView {
            HistoryView(model: model)
                .tabItem { Label("History", systemImage: "clock") }
            DevicesView(model: model)
                .tabItem { Label("Devices", systemImage: "laptopcomputer.and.iphone") }
        }
        .task { await model.start() }
    }
}
