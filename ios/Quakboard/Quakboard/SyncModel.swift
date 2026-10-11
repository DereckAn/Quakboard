import QuakboardSync
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class SyncModel: ObservableObject {
    struct Toast: Equatable {
        var message: String
        var isError = false
    }

    @Published private(set) var identityLine = "Starting…"
    @Published private(set) var peers: [PeerInfo] = []
    @Published private(set) var nearby: [NearbyDevice] = []
    @Published private(set) var discoveryProblem: String?
    @Published private(set) var page = HistoryPage()
    @Published var search = "" {
        didSet { reloadHistory() }
    }
    /// The selected filter chip; nil is All.
    @Published var filter: HistoryItem.Kind? {
        didSet { reloadHistory() }
    }
    @Published private(set) var toast: Toast?
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
    private var toastTask: Task<Void, Never>?

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

    /// From the system Paste button, which doesn't ask "Allow Paste?".
    func pasteAndSend(_ providers: [NSItemProvider]) {
        guard let provider = providers.first else { return }
        if provider.canLoadObject(ofClass: UIImage.self) {
            _ = provider.loadObject(ofClass: UIImage.self) { [weak self] image, _ in
                // Encoded here, off the main thread.
                guard let png = (image as? UIImage)?.pngData() else { return }
                Task { await self?.send(png: png) }
            }
        } else if provider.canLoadObject(ofClass: String.self) {
            _ = provider.loadObject(ofClass: String.self) { [weak self] text, _ in
                guard let text else { return }
                Task { await self?.send(text: text) }
            }
        }
    }

    func copy(_ item: HistoryItem) {
        if let url = imageURL(of: item), let png = try? Data(contentsOf: url) {
            UIPasteboard.general.setData(png, forPasteboardType: UTType.png.identifier)
        } else if let text = item.text {
            UIPasteboard.general.string = text
        }
        showToast("Copied to clipboard")
    }

    func togglePin(_ item: HistoryItem) {
        guard let store, let id = item.id else { return }
        do {
            try store.setPinned(id, !item.isPinned)
            reloadHistory()
        } catch {
            showToast(error.localizedDescription, isError: true)
        }
    }

    func imageURL(of item: HistoryItem) -> URL? {
        guard let store, let file = item.imageFile else { return nil }
        return store.imagesDir.appending(path: file)
    }

    private func reloadHistory() {
        guard let store else { return }
        do {
            page = try store.page(search: search, kind: filter)
        } catch {
            showToast(error.localizedDescription, isError: true)
        }
    }

    private func send(text: String) async {
        guard let node, let store else { return }
        do {
            try store.saveText(text, kind: .text)
            reloadHistory()
            showToast(Self.sent(to: await node.sendText(text: text)))
        } catch {
            showToast(error.localizedDescription, isError: true)
        }
    }

    private func send(png: Data) async {
        guard let node, let store else { return }
        do {
            try await Task.detached { try store.saveImage(png: png, fromName: nil) }.value
            reloadHistory()
            showToast(Self.sent(to: try await node.sendImage(png: png)))
        } catch {
            showToast(error.localizedDescription, isError: true)
        }
    }

    private static func sent(to reached: UInt32) -> String {
        switch reached {
        case 0: "Saved, but no paired device was reached"
        case 1: "Sent to 1 device"
        default: "Sent to \(reached) devices"
        }
    }

    private func showToast(_ message: String, isError: Bool = false) {
        toast = Toast(message: message, isError: isError)
        UIAccessibility.post(notification: .announcement, argument: message)
        toastTask?.cancel()
        toastTask = Task {
            try? await Task.sleep(for: .seconds(1.8))
            if !Task.isCancelled { toast = nil }
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
        showToast("Couldn't save to history: \(error.localizedDescription)", isError: true)
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

    func onText(text: String, contentType: String) {
        do {
            try history.saveText(text, kind: HistoryItem.Kind(contentType: contentType))
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
