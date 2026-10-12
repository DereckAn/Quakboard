import ImageIO
import QuakboardSync
import SwiftUI
import UIKit
import UniformTypeIdentifiers

/// "Share to Quakboard": sends what was shared to the paired devices, then
/// closes. iOS gives extensions little time and memory, so it only sends;
/// receiving stays in the app.
final class ShareViewController: UIViewController {
    private enum Content {
        case text(String)
        case image(Data)
    }

    // ponytail: photos are scaled to 3072 px; larger ones can exhaust the
    // extension's ~120 MB of memory and go over the 20 MB image limit.
    private static let maxImagePixels = 3072

    private let status = ShareStatus()

    override func viewDidLoad() {
        super.viewDidLoad()
        let host = UIHostingController(rootView: ShareView(status: status) { [weak self] in self?.close() })
        host.view.backgroundColor = .clear
        addChild(host)
        host.view.frame = view.bounds
        host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
        view.addSubview(host.view)
        host.didMove(toParent: self)
        Task { await send() }
    }

    private func send() async {
        do {
            guard let node = try Self.makeNode() else {
                status.state = .failed("Open Quakboard and pair a device first.")
                return
            }
            let reached: UInt32
            switch await sharedContent() {
            case .text(let text):
                reached = await node.sendText(text: text)
            case .image(let png):
                reached = try await node.sendImage(png: png)
            case nil:
                status.state = .failed("Quakboard can send text, links and images.")
                return
            }
            guard reached > 0 else {
                status.state = .failed("No paired device was reached. Is Quakboard running on a computer on this network?")
                return
            }
            status.state = .sent(reached)
            try? await Task.sleep(for: .seconds(1))
            close()
        } catch {
            status.state = .failed(Self.message(error))
        }
    }

    private func close() {
        extensionContext?.completeRequest(returningItems: nil)
    }

    /// The app's own sync identity, from the App Group. Nil before the app has
    /// ever run, so the extension never creates a second identity.
    private static func makeNode() throws -> SyncNode? {
        guard let group = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: "group.com.dereckan.quakboard") else {
            return nil
        }
        let dir = group.appending(path: "sync", directoryHint: .isDirectory)
        guard FileManager.default.fileExists(atPath: dir.appending(path: "sync.json").path(percentEncoded: false)) else {
            return nil
        }
        return try SyncNode(
            dataDir: dir.path(percentEncoded: false),
            deviceName: UIDevice.current.name,
            listenPort: defaultSyncPort(),
            delegate: SilentDelegate()
        )
    }

    /// An image first (Photos), then a web link (Safari), then text (a
    /// WhatsApp message).
    private func sharedContent() async -> Content? {
        let providers = (extensionContext?.inputItems as? [NSExtensionItem] ?? []).flatMap { $0.attachments ?? [] }
        if let provider = providers.first(where: { $0.hasItemConformingToTypeIdentifier(UTType.image.identifier) }),
           let item = try? await provider.loadItem(forTypeIdentifier: UTType.image.identifier) {
            // Photos shares a file; other apps share the data or the image.
            let source: CGImageSource? = switch item {
            case let url as URL: CGImageSourceCreateWithURL(url as CFURL, nil)
            case let data as Data: CGImageSourceCreateWithData(data as CFData, nil)
            case let image as UIImage: image.pngData().flatMap { CGImageSourceCreateWithData($0 as CFData, nil) }
            default: nil
            }
            if let png = source.flatMap(Self.png) {
                return .image(png)
            }
        }
        if let provider = providers.first(where: { $0.hasItemConformingToTypeIdentifier(UTType.url.identifier) }),
           let url = try? await provider.loadItem(forTypeIdentifier: UTType.url.identifier) as? URL,
           url.scheme == "http" || url.scheme == "https" {
            return .text(url.absoluteString)
        }
        if let provider = providers.first(where: { $0.hasItemConformingToTypeIdentifier(UTType.plainText.identifier) }),
           let text = try? await provider.loadItem(forTypeIdentifier: UTType.plainText.identifier) as? String {
            return .text(text)
        }
        return nil
    }

    /// Decodes at most `maxImagePixels` on the longest side, rotated upright.
    private static func png(from source: CGImageSource) -> Data? {
        let options = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: maxImagePixels,
        ] as CFDictionary
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, options) else { return nil }
        let png = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(png, UTType.png.identifier as CFString, 1, nil) else {
            return nil
        }
        CGImageDestinationAddImage(destination, image, nil)
        return CGImageDestinationFinalize(destination) ? png as Data : nil
    }

    private static func message(_ error: Error) -> String {
        switch error as? SyncNodeError {
        case .Storage(let message), .Network(let message), .InvalidInput(let message), .Fetch(let message):
            message
        case nil:
            error.localizedDescription
        }
    }
}

/// The extension only sends, so nothing arrives here.
private final class SilentDelegate: SyncDelegate {
    func onText(text: String, contentType: String) {}
    func onImage(png: Data, width: UInt32, height: UInt32, fromName: String) {}
    func onFileOffer(file: RemoteFileInfo) {}
    func onPairingEvent(event: PairingEvent) {}
}
