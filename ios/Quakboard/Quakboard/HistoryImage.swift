import SwiftUI

/// A saved PNG, decoded off the main thread and scaled down to `maxPixels`
/// on its longest side, so a list of screenshots stays light.
struct HistoryImage: View {
    let url: URL
    let maxPixels: CGFloat

    @State private var image: UIImage?

    var body: some View {
        Group {
            if let image {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
            } else {
                Color.secondary.opacity(0.1)
            }
        }
        .task(id: url) {
            let (url, size) = (url, CGSize(width: maxPixels, height: maxPixels))
            image = await Task.detached {
                UIImage(contentsOfFile: url.path(percentEncoded: false))?.preparingThumbnail(of: size)
            }.value
        }
    }
}
