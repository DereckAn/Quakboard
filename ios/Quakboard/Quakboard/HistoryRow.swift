import SwiftUI

struct HistoryRow: View {
    let item: HistoryItem
    let imageURL: URL?

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            if let imageURL {
                HistoryImage(url: imageURL, maxPixels: 400)
                    .frame(maxHeight: 120)
            } else {
                Text(item.text ?? "")
                    .lineLimit(3)
            }
            Text(subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
        }
    }

    private var subtitle: String {
        let time = item.createdAt.formatted(date: .omitted, time: .shortened)
        return item.fromName.map { "\(time) · from \($0)" } ?? time
    }
}
