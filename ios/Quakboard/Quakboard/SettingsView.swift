import QuakboardSync
import SwiftUI

/// Sync switches, this device, paired and nearby devices, and pairing.
struct SettingsView: View {
    @ObservedObject var model: SyncModel
    /// On iPad the system navigation bar stays, since it holds the button
    /// that brings the sidebar back.
    var isInSplitView = false

    @State private var address = ""
    @State private var unpairing: PeerInfo?
    @AppStorage(Appearance.storageKey) private var appearance = Appearance.system

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 12) {
                    thisDevice
                    SectionHeader(title: "Appearance", systemImage: "circle.lefthalf.filled")
                    Picker("Appearance", selection: $appearance) {
                        ForEach(Appearance.allCases) { Text($0.title).tag($0) }
                    }
                    .pickerStyle(.segmented)
                    .card()
                    SectionHeader(title: "Paired devices", systemImage: "link", count: model.peers.count)
                    if model.peers.isEmpty {
                        hint("No paired devices yet. Pair with one nearby, or by IP.")
                    }
                    ForEach(model.peers, id: \.id) { peerRow($0) }
                    SectionHeader(title: "Nearby", systemImage: "dot.radiowaves.left.and.right")
                    nearbySection
                    SectionHeader(title: "Connect by IP", systemImage: "network")
                    connectByIP
                }
                .padding(16)
                // Room for the tab bar.
                .padding(.bottom, isInSplitView ? 0 : 72)
                // Readable on a wide iPad screen.
                .frame(maxWidth: 680)
                .frame(maxWidth: .infinity)
            }
            .safeAreaInset(edge: .top, spacing: 0) {
                if !isInSplitView {
                    AppTitle(subtitle: "Settings").headerBar()
                }
            }
            .background(Theme.background)
            .navigationTitle("Devices & Sync")
            .navigationBarTitleDisplayMode(.inline)
            .toolbarBackground(Theme.background, for: .navigationBar)
            .toolbar(isInSplitView ? .automatic : .hidden, for: .navigationBar)
            .confirmationDialog(
                "Unpair \(unpairing?.name ?? "")?",
                isPresented: Binding(get: { unpairing != nil }, set: { if !$0 { unpairing = nil } }),
                titleVisibility: .visible,
                presenting: unpairing
            ) { peer in
                Button("Unpair", role: .destructive) { model.unpair(peer) }
            } message: { peer in
                Text("Clips stop syncing with \(peer.name). Pair again to resume.")
            }
            .sheet(item: $model.pairing, onDismiss: { model.cancelPairing() }) { pairing in
                PairingSheet(model: model, pairing: pairing)
            }
            .onAppear { model.setAcceptingPairing(true) }
            .onDisappear { model.setAcceptingPairing(false) }
        }
    }

    private var thisDevice: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(spacing: 10) {
                Image(systemName: UIDevice.current.userInterfaceIdiom == .pad ? "ipad" : "iphone")
                    .font(.system(size: 17))
                    .foregroundStyle(Theme.accentText)
                    .frame(width: 36, height: 36)
                    .background(Theme.inset, in: RoundedRectangle(cornerRadius: 10))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(model.identity?.name ?? "This device")
                        .font(.subheadline.weight(.semibold))
                        .foregroundStyle(Theme.text)
                    Text(addressLine)
                        .font(.caption.monospaced())
                        .foregroundStyle(Theme.textMuted)
                        .textSelection(.enabled)
                }
                Spacer()
                if let shortId = model.identity?.shortId {
                    Text("#\(shortId)")
                        .font(.caption2.monospaced().weight(.semibold))
                        .foregroundStyle(Theme.accentText)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .overlay(RoundedRectangle(cornerRadius: 5).stroke(Theme.border))
                        .accessibilityLabel("Short id \(shortId)")
                }
            }
            Theme.border.frame(height: 0.5)
            toggle(
                "Sync",
                detail: "Receive from and send to paired devices while Quakboard is open.",
                isOn: Binding(get: { model.isSyncEnabled }, set: { model.setSyncEnabled($0) })
            )
            toggle(
                "Also sync images",
                detail: "Send and receive copied images, up to 20 MB.",
                isOn: Binding(get: { model.isSyncingImages }, set: { model.setSyncingImages($0) })
            )
            if let problem = model.syncProblem {
                Label(problem, systemImage: "exclamationmark.triangle")
                    .font(.caption)
                    .foregroundStyle(Theme.textSecondary)
            }
        }
        .card()
    }

    private var addressLine: String {
        guard model.isSyncEnabled else { return "Sync is off" }
        guard let ip = Discovery.localIPv4() else { return "Not on a network" }
        return SyncModel.port == defaultSyncPort() ? ip : "\(ip):\(SyncModel.port)"
    }

    private func toggle(_ title: String, detail: String, isOn: Binding<Bool>) -> some View {
        Toggle(isOn: isOn) {
            VStack(alignment: .leading, spacing: 2) {
                Text(title)
                    .font(.subheadline.weight(.medium))
                    .foregroundStyle(Theme.text)
                Text(detail)
                    .font(.caption)
                    .foregroundStyle(Theme.textMuted)
            }
        }
        .tint(Theme.accent)
    }

    private func peerRow(_ peer: PeerInfo) -> some View {
        HStack(spacing: 10) {
            Circle()
                .fill(peer.isOnline ? Theme.accent : Theme.textMuted.opacity(0.5))
                .frame(width: 8, height: 8)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(peer.name)
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(Theme.text)
                Text(status(of: peer))
                    .font(.caption)
                    .foregroundStyle(peer.updateNeeded == nil ? Theme.textMuted : Theme.accentText)
            }
            Spacer()
            Button("Unpair") { unpairing = peer }
                .buttonStyle(ThemeButtonStyle())
        }
        .card()
    }

    private func status(of peer: PeerInfo) -> String {
        switch peer.updateNeeded {
        case .thisDevice: "Not syncing: update Quakboard on this device"
        case .otherDevice: "Not syncing: update Quakboard on \(peer.name)"
        case nil: peer.isOnline ? "Online" : "Offline"
        }
    }

    @ViewBuilder
    private var nearbySection: some View {
        if !model.isSyncEnabled {
            hint("Turn on Sync to find nearby devices.")
        } else if model.nearby.isEmpty {
            hint(model.discoveryProblem ?? "Looking for Quakboard on this network…")
        } else {
            ForEach(model.nearby, id: \.id) { device in
                HStack(spacing: 10) {
                    Image(systemName: "desktopcomputer")
                        .foregroundStyle(Theme.textMuted)
                        .accessibilityHidden(true)
                    VStack(alignment: .leading, spacing: 2) {
                        Text("Quakboard #\(device.id.suffix(4))")
                            .font(.subheadline.weight(.semibold))
                            .foregroundStyle(Theme.text)
                        Text("Not paired")
                            .font(.caption)
                            .foregroundStyle(Theme.textMuted)
                    }
                    Spacer()
                    Button("Pair") { model.pair(withDevice: device.id) }
                        .buttonStyle(ThemeButtonStyle(isPrimary: true))
                }
                .card()
            }
            if let problem = model.discoveryProblem {
                hint(problem)
            }
        }
    }

    private var connectByIP: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack(spacing: 8) {
                TextField("", text: $address, prompt: Text("192.168.1.20").foregroundColor(Theme.textMuted))
                    .font(.subheadline.monospaced())
                    .foregroundStyle(Theme.text)
                    .keyboardType(.numbersAndPunctuation)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .padding(.horizontal, 10)
                    .frame(height: 36)
                    .background(Theme.inset, in: RoundedRectangle(cornerRadius: 10))
                    .overlay(RoundedRectangle(cornerRadius: 10).stroke(Theme.border))
                    .accessibilityLabel("IP address")
                Button("Connect") { model.pair(withAddress: address) }
                    .buttonStyle(ThemeButtonStyle(isPrimary: true))
                    .disabled(address.trimmingCharacters(in: .whitespaces).isEmpty || !model.isSyncEnabled)
            }
            Text("The address the desktop shows in Devices, when it isn't listed above.")
                .font(.caption)
                .foregroundStyle(Theme.textMuted)
        }
        .card()
    }

    private func hint(_ text: String) -> some View {
        Text(text)
            .font(.footnote)
            .foregroundStyle(Theme.textMuted)
            .frame(maxWidth: .infinity, alignment: .leading)
            .card(Theme.background)
    }
}
