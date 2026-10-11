import SwiftUI

struct HistoryView: View {
    @ObservedObject var model: SyncModel

    var body: some View {
        NavigationStack {
            List {
                ForEach(Self.groups(model.history), id: \.title) { group in
                    Section(group.title) {
                        ForEach(group.items) { item in
                            NavigationLink {
                                HistoryDetailView(item: item, imageURL: model.imageURL(of: item))
                            } label: {
                                HistoryRow(item: item, imageURL: model.imageURL(of: item))
                            }
                        }
                    }
                }
            }
            .overlay {
                if model.history.isEmpty {
                    Text(model.search.isEmpty ? "Nothing received yet" : "No matches")
                        .foregroundStyle(.secondary)
                }
            }
            .searchable(text: $model.search)
            .navigationTitle("History")
        }
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
