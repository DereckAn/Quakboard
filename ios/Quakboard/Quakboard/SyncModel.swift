import QuakboardSync
import SwiftUI

@MainActor
final class SyncModel: ObservableObject {
    @Published private(set) var identityLine = "Starting…"
    @Published private(set) var peers: [PeerInfo] = []
    @Published private(set) var nearby: [NearbyDevice] = []
    @Published private(set) var discoveryProblem: String?
    @Published private(set) var history: [HistoryItem] = []
    @Published var search = "" {
        didSet { reloadHistory() }
    }
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
    private var store: History?
    private var discovery: Discovery?

    func start() async {
        guard node == nil else { return }
        do {
            let dir = URL.applicationSupportDirectory
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let store = try History(dir: dir)
            self.store = store
            reloadHistory()
            // Pairing keys: Rust saves through a temporary file and a rename,
            // and new files take their folder's protection class.
            // TODO(dereck): move to the App Group container with the Share Extension (Phase 4, step 5)
            let syncDir = dir.appending(path: "sync", directoryHint: .isDirectory)
            try FileManager.default.createDirectory(at: syncDir, withIntermediateDirectories: true)
            try FileManager.default.setAttributes(
                [.protectionKey: FileProtectionType.complete],
                ofItemAtPath: syncDir.path(percentEncoded: false)
            )
            let node = try SyncNode(
                dataDir: syncDir.path(percentEncoded: false),
                deviceName: UIDevice.current.name,
                listenPort: Self.port,
                delegate: Delegate(model: self, history: store)
            )
            self.node = node
            try await node.start()
            let me = node.identity()
            identityLine = "\(me.name) (\(me.shortId)), port \(Self.port)"
            peers = node.peers()
            discovery = Discovery(
                deviceId: me.id,
                port: Self.port,
                onChange: { [weak self] devices in self?.nearbyChanged(devices) },
                onProblem: { [weak self] problem in self?.discoveryProblem = problem }
            )
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

    func pair(withDevice id: String) {
        do {
            try node?.pairWithDevice(deviceId: id)
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

    func imageURL(of item: HistoryItem) -> URL? {
        guard let store, let file = item.imageFile else { return nil }
        return store.imagesDir.appending(path: file)
    }

    private func reloadHistory() {
        guard let store else { return }
        do {
            history = try store.items(matching: search)
        } catch {
            status = error.localizedDescription
        }
    }

    private func nearbyChanged(_ devices: [NearbyDevice]) {
        node?.setNearby(devices: devices)
        peers = node?.peers() ?? []
        let paired = Set(peers.map(\.id))
        nearby = devices.filter { !paired.contains($0.id) }.sorted { $0.id < $1.id }
    }

    fileprivate func handle(_ event: PairingEvent) {
        switch event {
        case .codeShown(let code):
            status = "Type \(code) on the other device"
        case .paired(let peerId, let name):
            isWaitingForCode = false
            peers = node?.peers() ?? []
            nearby.removeAll { $0.id == peerId }
            status = "Paired with \(name)"
        case .failed(let reason, _):
            isWaitingForCode = false
            status = reason
        }
    }

    fileprivate func received(text: String) {
        UIPasteboard.general.string = text
        reloadHistory()
    }

    fileprivate func receivedImage() {
        reloadHistory()
    }

    fileprivate func failedToSave(_ error: Error) {
        status = "Couldn't save to history: \(error.localizedDescription)"
    }
}

// Rust calls these on a background thread, so saving (a PNG can be 20 MB)
// happens here, off the main thread.
private final class Delegate: SyncDelegate {
    let model: SyncModel
    let history: History

    init(model: SyncModel, history: History) {
        self.model = model
        self.history = history
    }

    func onText(text: String) {
        do {
            try history.saveText(text)
            Task { @MainActor in model.received(text: text) }
        } catch {
            Task { @MainActor in model.failedToSave(error) }
        }
    }

    func onImage(png: Data, width: UInt32, height: UInt32, fromName: String) {
        do {
            try history.saveImage(png: png, fromName: fromName)
            Task { @MainActor in model.receivedImage() }
        } catch {
            Task { @MainActor in model.failedToSave(error) }
        }
    }

    func onFileOffer(file: RemoteFileInfo) {}

    func onPairingEvent(event: PairingEvent) {
        Task { @MainActor in model.handle(event) }
    }
}
