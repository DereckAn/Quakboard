import SwiftUI

/// The All Clips and Pinned tabs: the UI draft's header, filter chips and cards.
struct ClipsView: View {
    @ObservedObject var model: SyncModel
    let isPinnedOnly: Bool

    @State private var opened: HistoryItem?

    private static let filters: [(title: String, kind: HistoryItem.Kind)] = [
        ("Links", .link), ("Code", .code), ("Colors", .color), ("Images", .image), ("Plain Text", .text),
    ]

    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 12) {
                    chips
                    if !model.page.pinned.isEmpty {
                        sectionHeader("Pinned", systemImage: "pin.fill", count: model.page.pinned.count)
                        ForEach(model.page.pinned) { card($0) }
                    }
                    if !isPinnedOnly {
                        ForEach(Self.groups(model.page.recent), id: \.title) { group in
                            sectionHeader(group.title, systemImage: "clock", count: group.items.count)
                            ForEach(group.items) { card($0) }
                        }
                    }
                }
                .padding(.horizontal, 16)
                .padding(.top, 12)
                // Room for the Paste button.
                .padding(.bottom, 88)
            }
            .safeAreaInset(edge: .top, spacing: 0) { header }
            .overlay { emptyState }
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
                    HistoryDetailView(model: model, item: opened)
                }
            }
        }
    }

    private var header: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                Image(systemName: "doc.on.clipboard")
                    .font(.system(size: 15, weight: .semibold))
                    .foregroundStyle(Theme.accentText)
                    .frame(width: 32, height: 32)
                    .background(Theme.cardRaised, in: RoundedRectangle(cornerRadius: 10))
                    .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 1) {
                    Text("Quakboard")
                        .font(.title3.weight(.semibold))
                        .foregroundStyle(Theme.text)
                    Text(isPinnedOnly ? "Pinned" : "All Clips")
                        .font(.caption.weight(.medium))
                        .foregroundStyle(Theme.textSecondary)
                }
            }
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .foregroundStyle(Theme.textMuted)
                    .accessibilityHidden(true)
                TextField("", text: $model.search, prompt: Text("Search clips").foregroundColor(Theme.textMuted))
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .foregroundStyle(Theme.text)
                if !model.search.isEmpty {
                    Button { model.search = "" } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .foregroundStyle(Theme.textMuted)
                    .accessibilityLabel("Clear search")
                }
            }
            .font(.subheadline)
            .padding(.horizontal, 12)
            .frame(height: 38)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 12))
            .overlay(RoundedRectangle(cornerRadius: 12).stroke(Theme.border))
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(Theme.background)
        .overlay(alignment: .bottom) {
            Theme.border.frame(height: 0.5)
        }
    }

    private var chips: some View {
        ScrollView(.horizontal, showsIndicators: false) {
            HStack(spacing: 6) {
                chip("All", kind: nil, count: model.page.counts.values.reduce(0, +))
                ForEach(Self.filters, id: \.kind) { filter in
                    chip(filter.title, kind: filter.kind, count: model.page.counts[filter.kind] ?? 0)
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

    private func sectionHeader(_ title: String, systemImage: String, count: Int) -> some View {
        HStack {
            HStack(spacing: 6) {
                Image(systemName: systemImage)
                    .foregroundStyle(Theme.accentText)
                    .accessibilityHidden(true)
                Text(title)
                    .foregroundStyle(Theme.text)
            }
            .font(.subheadline.weight(.semibold))
            Spacer()
            Text("\(count) \(count == 1 ? "ITEM" : "ITEMS")")
                .font(.caption2.monospaced().weight(.semibold))
                .tracking(1)
                .foregroundStyle(Theme.textMuted)
        }
        .padding(.top, 8)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }

    private func card(_ item: HistoryItem) -> some View {
        ClipCard(
            item: item,
            imageURL: model.imageURL(of: item),
            onCopy: { model.copy(item) },
            onTogglePin: { model.togglePin(item) }
        )
        .contentShape(Rectangle())
        .onTapGesture { opened = item }
        .accessibilityAction(named: "Open") { opened = item }
    }

    @ViewBuilder
    private var emptyState: some View {
        if model.page.pinned.isEmpty && (isPinnedOnly || model.page.recent.isEmpty) {
            Text(emptyMessage)
                .font(.subheadline)
                .foregroundStyle(Theme.textMuted)
                .multilineTextAlignment(.center)
                .padding(32)
        }
    }

    private var emptyMessage: String {
        if !model.search.isEmpty || model.filter != nil { return "No matches" }
        return isPinnedOnly
            ? "Pin a clip to keep it here."
            : "Nothing yet. Copy something on a paired device, or paste here to send it."
    }

    private var pasteButton: some View {
        PasteControl { providers in model.pasteAndSend(providers) }
            .frame(width: 112, height: 44)
            .shadow(color: Theme.accent.opacity(0.25), radius: 10, y: 6)
            .padding(16)
    }

    /// The desktop's groups: Today, Yesterday, This Week, This Month, Older.
    private static func groups(_ items: [HistoryItem], now: Date = .now) -> [(title: String, items: [HistoryItem])] {
        let calendar = Calendar.current
        let today = calendar.startOfDay(for: now)
        let yesterday = calendar.date(byAdding: .day, value: -1, to: today)!
        let weekAgo = calendar.date(byAdding: .day, value: -7, to: today)!
        let monthAgo = calendar.date(byAdding: .month, value: -1, to: today)!

        let titles = ["Today", "Yesterday", "This Week", "This Month", "Older"]
        let byTitle = Dictionary(grouping: items) { item -> String in
            let day = calendar.startOfDay(for: item.createdAt)
            if day == today { return "Today" }
            if day == yesterday { return "Yesterday" }
            if day >= weekAgo { return "This Week" }
            if day >= monthAgo { return "This Month" }
            return "Older"
        }
        return titles.compactMap { title in byTitle[title].map { (title, $0) } }
    }
}
