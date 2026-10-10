import QuakboardSync
import SwiftUI

@MainActor
final class SyncModel: ObservableObject {
    @Published private(set) var identityLine = "Starting…"
    @Published private(set) var peers: [PeerInfo] = []
    @Published private(set) var lastText = "Nothing yet"
    @Published private(set) var isWaitingForCode = false
    @Published private(set) var status = ""

    // The simulator shares the Mac's network, where the desktop app already
    // listens on the default port.
    #if targetEnvironment(simulator)
    private static let port = defaultSyncPort() + 1
    #else
    private static let port = defaultSyncPort()
    #endif

    private var node: SyncNode?

    func start() async {
        guard node == nil else { return }
        do {
            let dir = URL.applicationSupportDirectory
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let node = try SyncNode(
                dataDir: dir.path(percentEncoded: false),
                deviceName: UIDevice.current.name,
                listenPort: Self.port,
                delegate: Delegate(model: self)
            )
            self.node = node
            try await node.start()
            let me = node.identity()
            identityLine = "\(me.name) (\(me.shortId)), port \(Self.port)"
            peers = node.peers()
        } catch {
            status = error.localizedDescription
        }
    }

    func pair(with address: String) {
        do {
            try node?.pairWithAddress(address: address)
            isWaitingForCode = true
            status = "Type the code the desktop shows"
        } catch {
            status = error.localizedDescription
        }
    }

    func submit(code: String) {
        do {
            try node?.submitPairingCode(code: code)
        } catch {
            status = error.localizedDescription
        }
    }

    func send(_ text: String) async {
        guard let node else { return }
        let reached = await node.sendText(text: text)
        status = "Sent to \(reached) device(s)"
    }

    fileprivate func handle(_ event: PairingEvent) {
        switch event {
        case .codeShown(let code):
            status = "Type \(code) on the other device"
        case .paired(_, let name):
            isWaitingForCode = false
            peers = node?.peers() ?? []
            status = "Paired with \(name)"
        case .failed(let reason, _):
            isWaitingForCode = false
            status = reason
        }
    }

    fileprivate func received(text: String) {
        lastText = text
    }

    fileprivate func receivedImage(from name: String) {
        status = "Received an image from \(name)"
    }
}

// Rust calls these on a background thread.
private final class Delegate: SyncDelegate {
    let model: SyncModel

    init(model: SyncModel) {
        self.model = model
    }

    func onText(text: String) {
        Task { @MainActor in model.received(text: text) }
    }

    func onImage(png: Data, width: UInt32, height: UInt32, fromName: String) {
        Task { @MainActor in model.receivedImage(from: fromName) }
    }

    func onFileOffer(file: RemoteFileInfo) {}

    func onPairingEvent(event: PairingEvent) {
        Task { @MainActor in model.handle(event) }
    }
}
