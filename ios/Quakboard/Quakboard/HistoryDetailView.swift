import SwiftUI
import UniformTypeIdentifiers

struct HistoryDetailView: View {
    let item: HistoryItem
    let imageURL: URL?

    @State private var isCopied = false

    var body: some View {
        ScrollView {
            if let imageURL {
                HistoryImage(url: imageURL, maxPixels: 2048)
                    .padding()
            } else {
                Text(item.text ?? "")
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding()
            }
        }
        .navigationTitle(item.createdAt.formatted(date: .abbreviated, time: .shortened))
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            Button(isCopied ? "Copied" : "Copy", action: copy)
        }
    }

    private func copy() {
        if let imageURL, let png = try? Data(contentsOf: imageURL) {
            UIPasteboard.general.setData(png, forPasteboardType: UTType.png.identifier)
        } else if let text = item.text {
            UIPasteboard.general.string = text
        }
        isCopied = true
    }
}
