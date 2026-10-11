import SwiftUI

/// The iPad's clips: the list with the open clip beside it, or, when there
/// isn't room for both (portrait with the sidebar open), the list alone with
/// clips opening on top like the iPhone.
struct ClipsWorkspace: View {
    @ObservedObject var model: SyncModel
    let title: String
    let isPinnedOnly: Bool
    @Binding var selectedID: Int64?

    /// Narrow layout only.
    @State private var opened: HistoryItem?

    private static let minDualPaneWidth: CGFloat = 720

    var body: some View {
        NavigationStack {
            GeometryReader { geometry in
                let isDualPane = geometry.size.width >= Self.minDualPaneWidth
                VStack(spacing: 0) {
                    header
                    HStack(alignment: .top, spacing: 0) {
                        ScrollView {
                            LazyVStack(alignment: .leading, spacing: 12) {
                                ClipList(
                                    model: model,
                                    isPinnedOnly: isPinnedOnly,
                                    selectedID: isDualPane ? selectedID : nil
                                ) { item in
                                    if isDualPane {
                                        selectedID = item.id
                                    } else {
                                        opened = item
                                    }
                                }
                            }
                            .padding(16)
                        }
                        .frame(width: isDualPane ? max(340, geometry.size.width * 0.42) : nil)
                        if isDualPane {
                            Theme.border.frame(width: 0.5)
                            detailPane
                        }
                    }
                }
            }
            .background(Theme.background)
            .navigationTitle(title)
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(Theme.background, for: .navigationBar)
            .navigationDestination(isPresented: Binding(
                get: { opened != nil },
                set: { if !$0 { opened = nil } }
            )) {
                if let opened {
                    ClipDetailView(model: model, item: opened) { self.opened = nil }
                }
            }
        }
    }

    private var header: some View {
        HStack(spacing: 12) {
            SearchField(text: $model.search)
            Text("\(model.page.pinned.count + model.page.recent.count) clips")
                .font(.caption.monospaced().weight(.semibold))
                .foregroundStyle(Theme.textSecondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(Theme.inset, in: Capsule())
                .fixedSize()
            if !isPinnedOnly {
                PasteControl { providers in model.pasteAndSend(providers) }
                    .frame(width: 112, height: 40)
            }
        }
        .headerBar()
    }

    @ViewBuilder
    private var detailPane: some View {
        if let item = model.item(withID: selectedID) {
            ClipDetailView(model: model, item: item) { selectedID = nil }
                // A fresh pane per clip, so a pending delete prompt doesn't carry over.
                .id(item.id)
        } else {
            VStack(spacing: 12) {
                Image(systemName: "doc.on.clipboard")
                    .font(.system(size: 40))
                    .accessibilityHidden(true)
                Text("Select a clip to see it here")
                    .font(.subheadline)
            }
            .foregroundStyle(Theme.textMuted)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
    }
}
