import SwiftUI

/// The All Clips and Pinned tabs: the UI draft's header, filter chips and cards.
struct ClipsView: View {
    @ObservedObject var model: SyncModel
    let isPinnedOnly: Bool

    @State private var opened: HistoryItem?

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    if model.hasNewClipboard && !isPinnedOnly {
                        ClipboardOfferBanner(model: model)
                    }
                    chips
                    ClipList(model: model, isPinnedOnly: isPinnedOnly) { opened = $0 }
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
                // Room for the Paste button.
                .padding(.bottom, 88)
            }
            .safeAreaInset(edge: .top, spacing: 0) { header }
            .overlay(alignment: .bottomTrailing) {
                if !isPinnedOnly { pasteButton }
            }
            .background(Theme.background)
            .toolbar(.hidden, for: .navigationBar)
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
        VStack(alignment: .leading, spacing: 10) {
            AppTitle(subtitle: isPinnedOnly ? "Pinned" : "All Clips")
            HStack(spacing: 8) {
                SearchField(text: $model.search)
                if !isPinnedOnly {
                    SendFileButton(model: model)
                }
            }
        }
        .headerBar()
    }

    private var chips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                chip("All", kind: nil, count: model.page.counts.values.reduce(0, +))
                ForEach(HistoryItem.Kind.filterOrder, id: \.self) { kind in
                    chip(kind.pluralTitle, kind: kind, count: model.page.counts[kind] ?? 0)
                }
            }
            .padding(.horizontal, 16)
        }
        .padding(.horizontal, -16)
    }

    private func chip(_ title: String, kind: HistoryItem.Kind?, count: Int) -> some View {
        let isSelected = model.filter == kind
        return Button { model.filter = kind } label: {
            HStack(spacing: 6) {
                Text(title)
                    .font(.footnote.weight(isSelected ? .bold : .medium))
                Text("\(count)")
                    .font(.caption2.monospaced().weight(.semibold))
                    .padding(.horizontal, 6)
                    .padding(.vertical, 1)
                    .background(isSelected ? Theme.onAccent.opacity(0.15) : Theme.card, in: Capsule())
            }
            .foregroundStyle(isSelected ? Theme.onAccent : Theme.textSecondary)
            .padding(.horizontal, 12)
            .padding(.vertical, 7)
            .background(isSelected ? Theme.accent : Theme.cardRaised, in: Capsule())
            .overlay(Capsule().stroke(isSelected ? Color.clear : Theme.border))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(isSelected ? .isSelected : [])
    }

    private var pasteButton: some View {
        PasteControl { providers in model.pasteAndSend(providers) }
            .frame(width: 112, height: 44)
            .shadow(color: Theme.accent.opacity(0.25), radius: 10, y: 6)
            .padding(16)
    }
}
