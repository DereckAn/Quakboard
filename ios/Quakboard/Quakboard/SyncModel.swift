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
        static let seenPasteboardChange = "seenPasteboardChange"
        static let isSendingOnOpen = "isSendingOnOpen"
    }

    /// How long sync keeps running after the app leaves the screen, so a
    /// transfer can finish and a quick copy elsewhere still arrives. iOS
    /// grants roughly 30 seconds of background time; this stays under it.
    private static let backgroundGrace = Duration.seconds(25)

    @Published private(set) var identity: DeviceIdentity?
    @Published private(set) var peers: [PeerInfo] = []
    @Published private(set) var nearby: [NearbyDevice] = []
    @Published private(set) var discoveryProblem: String?
    @Published private(set) var syncProblem: String?
    @Published private(set) var isSyncEnabled = UserDefaults.standard.object(forKey: Keys.isSyncEnabled) as? Bool ?? true
    @Published private(set) var isSyncingImages = UserDefaults.standard.object(forKey: Keys.isSyncingImages) as? Bool ?? true
    /// Send what was copied elsewhere as soon as the app opens. Off by
    /// default: it sends without a tap, and iOS asks "Allow Paste?" each time
    /// unless the user allows pasting for the app in iOS Settings.
    @Published private(set) var isSendingOnOpen = UserDefaults.standard.bool(forKey: Keys.isSendingOnOpen)
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
    /// Something new is on the clipboard that the user might want to send.
    @Published private(set) var hasNewClipboard = false

    // Simulators share the Mac's network, where the desktop app already
    // listens on the default port; iPhone and iPad simulators get their own,
    // so one of each can run at once.
    #if targetEnvironment(simulator)
    static let port = defaultSyncPort() + (UIDevice.current.userInterfaceIdiom == .pad ? 2 : 1)
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
    /// False while every window is in the background. iOS suspends the app
    /// there, so sync stops instead of leaving a listener nobody answers.
    private var isAppActive = true
    private var backgroundTask = UIBackgroundTaskIdentifier.invalid
    private var graceTask: Task<Void, Never>?
    /// The clipboard's change count last offered, sent or written by us.
    private var seenPasteboardChange = UserDefaults.standard.integer(forKey: Keys.seenPasteboardChange)
    /// The copy last tried automatically, so a "Don't Allow" isn't asked again
    /// every time the app comes back.
    private var autoSendAttempt: Int?

    func start() async {
        guard node == nil else { return }
        do {
            let dir = URL.applicationSupportDirectory
            try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
            let store = try History(dir: dir)
            self.store = store
            reloadHistory()
            let syncDir = try Self.syncDirectory(movingFrom: dir)
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
            syncProblem = Self.message(error)
            return
        }
        if isSyncEnabled {
            applySyncSetting()
        }
        checkClipboard()
    }

    func setSyncEnabled(_ isEnabled: Bool) {
        isSyncEnabled = isEnabled
        UserDefaults.standard.set(isEnabled, forKey: Keys.isSyncEnabled)
        applySyncSetting()
    }

    /// Takes effect the next time the app opens, not right away.
    func setSendingOnOpen(_ isEnabled: Bool) {
        isSendingOnOpen = isEnabled
        UserDefaults.standard.set(isEnabled, forKey: Keys.isSendingOnOpen)
        if isEnabled {
            hasNewClipboard = false
        }
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
            showToast(Self.message(error), isError: true)
        }
    }

    func pair(withDevice id: String) {
        do {
            try node?.pairWithDevice(deviceId: id)
            pairing = .enteringCode
        } catch {
            showToast(Self.message(error), isError: true)
        }
    }

    /// The outcome arrives as a pairing event.
    func submit(code: String) {
        do {
            try node?.submitPairingCode(code: code)
        } catch {
            pairing = nil
            showToast(Self.message(error), isError: true)
        }
    }

    /// The pairing keys' folder, in the App Group so the Share Extension sends
    /// as this device. Pairings made before it existed move there once.
    /// History stays in the app: SQLite in a shared folder can get a suspended
    /// app killed for holding a lock.
    private static func syncDirectory(movingFrom appDir: URL) throws -> URL {
        let files = FileManager.default
        guard let group = files.containerURL(forSecurityApplicationGroupIdentifier: "group.com.dereckan.quakboard") else {
            throw CocoaError(.fileNoSuchFile)
        }
        let dir = group.appending(path: "sync", directoryHint: .isDirectory)
        let old = appDir.appending(path: "sync", directoryHint: .isDirectory)
        if !files.fileExists(atPath: dir.path(percentEncoded: false)),
           files.fileExists(atPath: old.path(percentEncoded: false)) {
            try files.moveItem(at: old, to: dir)
        }
        try files.createDirectory(at: dir, withIntermediateDirectories: true)
        // Rust saves through a temporary file and a rename, and new files take
        // their folder's protection class.
        try files.setAttributes([.protectionKey: FileProtectionType.complete], ofItemAtPath: dir.path(percentEncoded: false))
        return dir
    }

    /// After a short grace period in the background, sync stops and desktops
    /// see this device as offline, like a closed laptop. Coming back starts
    /// sync and discovery again right away.
    func setAppActive(_ isActive: Bool) {
        guard isActive != isAppActive else { return }
        isAppActive = isActive
        if isActive {
            endGracePeriod()
            applySyncSetting()
            checkClipboard()
        } else {
            if pairing != nil {
                cancelPairing()
            }
            beginGracePeriod()
        }
    }

    /// Without reading it: iOS only says whether the clipboard changed and
    /// holds text or an image, which needs no "Allow Paste?" prompt.
    func checkClipboard() {
        let pasteboard = UIPasteboard.general
        let isNew = pasteboard.changeCount != seenPasteboardChange
            && (pasteboard.hasStrings || pasteboard.hasImages)
            && isSyncEnabled
            && !peers.isEmpty
        // With sending on open, the banner would only repeat what just happened.
        hasNewClipboard = isNew && !isSendingOnOpen
        if isNew && isSendingOnOpen && autoSendAttempt != pasteboard.changeCount {
            autoSendAttempt = pasteboard.changeCount
            sendClipboard()
        }
    }

    /// Reads the clipboard in code, which is what makes iOS ask "Allow
    /// Paste?". On "Don't Allow" nothing is read or sent.
    private func sendClipboard() {
        let pasteboard = UIPasteboard.general
        if pasteboard.hasImages, let image = pasteboard.image {
            markClipboardSeen()
            Task {
                // Encoded off the main thread.
                guard let png = await Task.detached(operation: { image.pngData() }).value else { return }
                await send(png: png)
            }
        } else if pasteboard.hasStrings, let text = pasteboard.string {
            markClipboardSeen()
            Task { await send(text: text) }
        }
    }

    func dismissClipboardOffer() {
        markClipboardSeen()
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
            showToast(Self.message(error), isError: true)
        }
    }

    /// From the system Paste button, which doesn't ask "Allow Paste?".
    func pasteAndSend(_ providers: [NSItemProvider]) {
        guard isSyncEnabled else {
            showToast("Turn on Sync in Settings to send", isError: true)
            return
        }
        guard let provider = providers.first else { return }
        markClipboardSeen()
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
        markClipboardSeen()
        showToast("Copied to clipboard")
    }

    func togglePin(_ item: HistoryItem) {
        guard let store, let id = item.id else { return }
        do {
            try store.setPinned(id, !item.isPinned)
            reloadHistory()
        } catch {
            showToast(Self.message(error), isError: true)
        }
    }

    func delete(_ item: HistoryItem) {
        guard let store, let id = item.id else { return }
        do {
            try store.delete(id)
            reloadHistory()
            showToast("Deleted")
        } catch {
            showToast(Self.message(error), isError: true)
        }
    }

    /// The clip as currently loaded, after a pin or an edit.
    func item(withID id: Int64?) -> HistoryItem? {
        guard let id else { return nil }
        return (page.pinned + page.recent).first { $0.id == id }
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
            showToast(Self.message(error), isError: true)
        }
    }

    private func send(text: String) async {
        guard let node, let store else { return }
        do {
            try store.saveText(text, kind: HistoryItem.Kind(contentType: detectContentType(text: text)))
            reloadHistory()
            showToast(Self.sent(to: await node.sendText(text: text)))
        } catch {
            showToast(Self.message(error), isError: true)
        }
    }

    private func send(png: Data) async {
        guard let node, let store else { return }
        do {
            try await Task.detached { try store.saveImage(png: png, fromName: nil) }.value
            reloadHistory()
            showToast(Self.sent(to: try await node.sendImage(png: png)))
        } catch {
            showToast(Self.message(error), isError: true)
        }
    }

    private static func sent(to reached: UInt32) -> String {
        switch reached {
        case 0: "Saved, but no paired device was reached"
        case 1: "Sent to 1 device"
        default: "Sent to \(reached) devices"
        }
    }

    /// The text to show for an error. The generated bindings describe sync
    /// errors with their debug form (`SyncNodeError.Network(message: …)`), so
    /// those show just their message.
    private static func message(_ error: Error) -> String {
        switch error as? SyncNodeError {
        case .Storage(let message), .Network(let message), .InvalidInput(let message), .Fetch(let message):
            message
        case nil:
            error.localizedDescription
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

    private func markClipboardSeen() {
        seenPasteboardChange = UIPasteboard.general.changeCount
        UserDefaults.standard.set(seenPasteboardChange, forKey: Keys.seenPasteboardChange)
        hasNewClipboard = false
    }

    /// Keeps sync running for `backgroundGrace`, with iOS's permission, then
    /// stops it.
    private func beginGracePeriod() {
        backgroundTask = UIApplication.shared.beginBackgroundTask(withName: "Keep receiving") { [weak self] in
            // iOS is ending the background time early: stop right now.
            MainActor.assumeIsolated {
                self?.stopSync()
                self?.endGracePeriod()
            }
        }
        graceTask = Task {
            try? await Task.sleep(for: Self.backgroundGrace)
            guard !Task.isCancelled else { return }
            applySyncSetting()
            await syncTask?.value
            endGracePeriod()
        }
    }

    private func endGracePeriod() {
        graceTask?.cancel()
        graceTask = nil
        if backgroundTask != .invalid {
            UIApplication.shared.endBackgroundTask(backgroundTask)
            backgroundTask = .invalid
        }
    }

    /// Starts and stops run one at a time, each applying the Sync switch and
    /// whether the app is in front as they are when its turn comes, so quick
    /// toggles and app switches end in the latest state.
    private func applySyncSetting() {
        let previous = syncTask
        syncTask = Task {
            await previous?.value
            if isSyncEnabled && isAppActive {
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
            syncProblem = "Couldn't start sync: \(Self.message(error))"
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
        // Our own write: not something to offer to send back.
        markClipboardSeen()
        reloadHistory()
    }

    fileprivate func receivedImage() {
        reloadHistory()
    }

    fileprivate func failedToSave(_ error: Error) {
        showToast("Couldn't save to history: \(Self.message(error))", isError: true)
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
