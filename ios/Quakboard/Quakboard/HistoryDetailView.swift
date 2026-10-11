import SwiftUI

struct HistoryDetailView: View {
    @ObservedObject var model: SyncModel
    let item: HistoryItem

    var body: some View {
        ScrollView {
            if let imageURL = model.imageURL(of: item) {
                HistoryImage(url: imageURL, maxPixels: 2048)
                    .padding()
            } else {
                Text(item.text ?? "")
                    .font(item.kind == .code ? .callout.monospaced() : .body)
                    .foregroundStyle(Theme.text)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
        }
        .background(Theme.background)
        .navigationTitle(item.createdAt.formatted(date: .abbreviated, time: .shortened))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            Button("Copy") { model.copy(item) }
        }
    }
}
