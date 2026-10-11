import SwiftUI

/// The iPad layout: a sidebar of sections and types, then the clips with the
/// open one beside them, or Settings.
struct WorkspaceView: View {
    enum Section: Hashable {
        case all, pinned, settings
        case kind(HistoryItem.Kind)
    }

    @ObservedObject var model: SyncModel

    @State private var section = Section.all
    @State private var selectedID: Int64?

    var body: some View {
        NavigationSplitView {
            WorkspaceSidebar(model: model, selection: $section)
                .navigationSplitViewColumnWidth(min: 240, ideal: 280, max: 320)
                .toolbar(.hidden, for: .navigationBar)
        } detail: {
            if section == .settings {
                SettingsView(model: model, isInSplitView: true)
            } else {
                ClipsWorkspace(
                    model: model,
                    title: title,
                    isPinnedOnly: section == .pinned,
                    selectedID: $selectedID
                )
            }
        }
    }

    private var title: String {
        switch section {
        case .all: "All Clips"
        case .pinned: "Pinned"
        case .settings: "Settings"
        case .kind(let kind): kind.pluralTitle
        }
    }
}
