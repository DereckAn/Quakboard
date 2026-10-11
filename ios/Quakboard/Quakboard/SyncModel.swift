import QuakboardSync
import SwiftUI
import UniformTypeIdentifiers

@MainActor
final class SyncModel: ObservableObject {
    struct Toast: Equatable {
        var message: String
        var isError = false
    }

    /// A pairing in progress, shown as a sheet.
    enum Pairing: Identifiable, Equatable {
        /// This phone asked: type the code the other device shows.
        case enteringCode
        /// Another device asked: show this code, to type over there.
        case showingCode(String)

        var id: String {
            switch self {
            case .enteringCode: "entering"
            case .showingCode(let code): "showing \(code)"
            }
        }
    }

    private enum Keys {
        static let isSyncEnabled = "isSyncEnabled"
        static let isSyncingImages = "isSyncingImages"
    }

    @Published private(set) var identity: DeviceIdentity?
    @Published private(set) var peers: [PeerInfo] = []
    @Published private(set) var nearby: [NearbyDevice] = []
    @Published private(set) var discoveryProblem: String?
    @Published private(set) var syncProblem: String?
    @Published private(set) var isSyncEnabled = UserDefaults.standard.object(forKey: Keys.isSyncEnabled) as? Bool ?? true
    @Published private(set) var isSyncingImages = UserDefaults.standard.object(forKey: Keys.isSyncingImages) as? Bool ?? true
    @Published var pairing: Pairing?
    @Published private(set) var page = HistoryPage()
    @Published var search = "" {
        didSet { reloadHistory() }
    }
    /// The selected filter chip; nil is All.
    @Published var filter: HistoryItem.Kind? {
        didSet { reloadHistory() }
    }
    @Published private(set) var toast: Toast?

    // The simulator shares the Mac's network, where the desktop app already
    // listens on the default port.
    #if targetEnvironment(simulator)
    static let port = defaultSyncPort() + 1
    #else
    static let port = defaultSyncPort()
    #endif

    private var node: SyncNode?
    private var store: History?
    private var discovery: Discovery?
    /// Everything discovery sees, paired or not; `nearby` is the unpaired part.
    private var allNearby: [NearbyDevice] = []
    private var toastTask: Task<Void, Never>?
    /// The latest start or stop; each waits for the one before.
    private var syncTask: Task<Void, Never>?

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
            node.setSyncingImages(enabled: isSyncingImages)
            self.node = node
            identity = node.identity()
            peers = node.peers()
        } catch {
            syncProblem = error.localizedDescription
            return
        }
        if isSyncEnabled {
            applySyncSetting()
        }
    }

    func setSyncEnabled(_ isEnabled: Bool) {
        isSyncEnabled = isEnabled
        UserDefaults.standard.set(isEnabled, forKey: Keys.isSyncEnabled)
        applySyncSetting()
    }

    func setSyncingImages(_ isEnabled: Bool) {
        isSyncingImages = isEnabled
        UserDefaults.standard.set(isEnabled, forKey: Keys.isSyncingImages)
        node?.setSyncingImages(enabled: isEnabled)
    }

    /// Answer pairing requests only while Settings is on screen, so nobody on
    /// the network can pop up a code at other times.
    func setAcceptingPairing(_ isAccepting: Bool) {
        node?.setAcceptingPairing(accepting: isAccepting)
    }

    func pair(withAddress address: String) {
        do {
            try node?.pairWithAddress(address: address)
            pairing = .enteringCode
        } catch {
            showToast(error.localizedDescription, isError: true)
        }
    }

    func pair(withDevice id: String) {
        do {
            try node?.pairWithDevice(deviceId: id)
            pairing = .enteringCode
        } catch {
            showToast(error.localizedDescription, isError: true)
        }
    }

    /// The outcome arrives as a pairing event.
    func submit(code: String) {
        do {
            try node?.submitPairingCode(code: code)
        } catch {
            pairing = nil
            showToast(error.localizedDescription, isError: true)
        }
    }

    func cancelPairing() {
        node?.cancelPairing()
        pairing = nil
    }

    func unpair(_ peer: PeerInfo) {
        do {
            _ = try node?.unpair(peerId: peer.id)
            refreshDevices()
            showToast("Unpaired \(peer.name)")
        } catch {
            showToast(error.localizedDescription, isError: true)
        }
    }

    /// From the system Paste button, which doesn't ask "Allow Paste?".
    func pasteAndSend(_ providers: [NSItemProvider]) {
        guard isSyncEnabled else {
            showToast("Turn on Sync in Settings to send", isError: true)
            return
        }
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

    /// Starts and stops run one at a time, each applying the switch as it is
    /// when its turn comes, so quick toggles end in the last state chosen.
    private func applySyncSetting() {
        let previous = syncTask
        syncTask = Task {
            await previous?.value
            if isSyncEnabled {
                await startSync()
            } else {
                stopSync()
            }
        }
    }

    private func startSync() async {
        guard let node, let identity, discovery == nil else { return }
        do {
            try await node.start()
            syncProblem = nil
        } catch {
            syncProblem = "Couldn't start sync: \(error.localizedDescription)"
            return
        }
        discovery = Discovery(
            deviceId: identity.id,
            port: Self.port,
            onChange: { [weak self] devices in self?.nearbyChanged(devices) },
            onProblem: { [weak self] problem in self?.discoveryProblem = problem }
        )
    }

    private func stopSync() {
        node?.stop()
        discovery?.stop()
        discovery = nil
        discoveryProblem = nil
        nearbyChanged([])
    }

    private func nearbyChanged(_ devices: [NearbyDevice]) {
        allNearby = devices
        node?.setNearby(devices: devices)
        refreshDevices()
    }

    private func refreshDevices() {
        peers = node?.peers() ?? []
        let paired = Set(peers.map(\.id))
        nearby = allNearby.filter { !paired.contains($0.id) }.sorted { $0.id < $1.id }
    }

    fileprivate func handle(_ event: PairingEvent) {
        switch event {
        case .codeShown(let code):
            pairing = .showingCode(code)
        case .paired(_, let name):
            pairing = nil
            refreshDevices()
            showToast("Paired with \(name)")
        case .failed(let reason, let needsUpdate):
            // A failure after the user cancelled isn't news, unless one of
            // the devices needs updating.
            let wasPairing = pairing != nil
            pairing = nil
            if wasPairing || needsUpdate {
                showToast(reason, isError: true)
            }
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
