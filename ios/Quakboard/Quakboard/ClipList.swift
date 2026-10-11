import SwiftUI

/// The clips as cards: Pinned first, then grouped by date like the desktop.
/// Goes inside a lazy stack; shared by the iPhone tabs and the iPad workspace.
struct ClipList: View {
    @ObservedObject var model: SyncModel
    let isPinnedOnly: Bool
    /// Highlighted on iPad, where the opened clip shows beside the list.
    var selectedID: Int64?
    let onOpen: (HistoryItem) -> Void

    var body: some View {
        if model.page.pinned.isEmpty && (isPinnedOnly || model.page.recent.isEmpty) {
            Text(emptyMessage)
                .font(.subheadline)
                .foregroundStyle(Theme.textMuted)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity)
                .padding(32)
        }
        if !model.page.pinned.isEmpty {
            SectionHeader(title: "Pinned", systemImage: "pin.fill", count: model.page.pinned.count)
            ForEach(model.page.pinned) { card($0) }
        }
        if !isPinnedOnly {
            ForEach(Self.groups(model.page.recent), id: \.title) { group in
                SectionHeader(title: group.title, systemImage: "clock", count: group.items.count)
                ForEach(group.items) { card($0) }
            }
        }
    }

    private func card(_ item: HistoryItem) -> some View {
        ClipCard(
            item: item,
            imageURL: model.imageURL(of: item),
            isSelected: item.id != nil && item.id == selectedID,
            onCopy: { model.copy(item) },
            onTogglePin: { model.togglePin(item) }
        )
        .contentShape(Rectangle())
        .onTapGesture { onOpen(item) }
        .accessibilityAction(named: "Open") { onOpen(item) }
    }

    private var emptyMessage: String {
        if !model.search.isEmpty || model.filter != nil { return "No matches" }
        return isPinnedOnly
            ? "Pin a clip to keep it here."
            : "Nothing yet. Copy something on a paired device, or paste here to send it."
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
