import Foundation
import QuakboardSync

/// Bonjour the way desktops do it: the service is named after the device id,
/// which the TXT record also carries.
// ponytail: NetService is marked "to be deprecated"; move to dns_sd if Apple
// deprecates it. NWListener can't advertise a port the Rust listener owns.
@MainActor
final class Discovery: NSObject {
    // A resolve only listens for a few seconds, so addresses are looked up
    // again on this interval; it also retries a failed publish or browse
    // (for example after the user allows Local Network in Settings).
    private static let refreshInterval: TimeInterval = 30
    // kDNSServiceErr_PolicyDenied: Local Network access is off.
    private static let policyDenied = -65570

    private let ownId: String
    private let type = discoveryServiceType() + "."
    private let onChange: ([NearbyDevice]) -> Void
    private let onProblem: (String?) -> Void
    private let advertised: NetService
    private let browser = NetServiceBrowser()
    // NetService only resolves while something holds it. Keyed by service name.
    private var services: [String: NetService] = [:]
    private var found: [String: NearbyDevice] = [:]
    private var isPublished = false
    private var isBrowsing = false

    init(
        deviceId: String,
        port: UInt16,
        onChange: @escaping ([NearbyDevice]) -> Void,
        onProblem: @escaping (String?) -> Void
    ) {
        ownId = deviceId
        self.onChange = onChange
        self.onProblem = onProblem
        advertised = NetService(domain: "local.", type: type, name: deviceId, port: Int32(port))
        super.init()
        advertised.delegate = self
        advertised.setTXTRecord(NetService.data(fromTXTRecord: [discoveryIdKey(): Data(deviceId.utf8)]))
        browser.delegate = self
        refresh()
        Timer.scheduledTimer(withTimeInterval: Self.refreshInterval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    private func refresh() {
        if !isPublished {
            advertised.stop()
            advertised.publish()
        }
        if !isBrowsing {
            browser.stop()
            browser.searchForServices(ofType: type, inDomain: "local.")
        }
        for service in services.values {
            service.stop()
            service.resolve(withTimeout: 5)
        }
    }

    private func resolved(_ service: NetService) {
        let txt = service.txtRecordData().map(NetService.dictionary(fromTXTRecord:)) ?? [:]
        guard
            let id = txt[discoveryIdKey()].flatMap({ String(data: $0, encoding: .utf8) }),
            id != ownId,
            UUID(uuidString: id) != nil,
            let ip = Self.pickIPv4(service.addresses ?? [])
        else { return }
        let device = NearbyDevice(id: id, address: "\(ip):\(service.port)")
        guard found[service.name] != device else { return }
        found[service.name] = device
        onChange(Array(found.values))
    }

    private func failed(_ action: String, _ errorDict: [String: NSNumber]) {
        let code = errorDict[NetService.errorCode]?.intValue ?? 0
        onProblem(code == Self.policyDenied
            ? "Allow Local Network for Quakboard in Settings > Privacy & Security to find nearby devices."
            : "Couldn't \(action) on the network (error \(code)). Retrying…")
    }

    /// A LAN address over loopback, like the desktop.
    private static func pickIPv4(_ addresses: [Data]) -> String? {
        let ips = addresses.compactMap { data -> String? in
            data.withUnsafeBytes { raw in
                guard let sockaddr = raw.baseAddress?.assumingMemoryBound(to: sockaddr.self),
                      sockaddr.pointee.sa_family == sa_family_t(AF_INET) else { return nil }
                var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
                guard getnameinfo(sockaddr, socklen_t(data.count), &host, socklen_t(host.count),
                                  nil, 0, NI_NUMERICHOST) == 0 else { return nil }
                return String(decoding: host.prefix { $0 != 0 }.map(UInt8.init(bitPattern:)), as: UTF8.self)
            }
        }
        return ips.first { !$0.hasPrefix("127.") } ?? ips.first
    }
}

extension Discovery: @preconcurrency NetServiceBrowserDelegate {
    func netServiceBrowserWillSearch(_ browser: NetServiceBrowser) {
        isBrowsing = true
        onProblem(nil)
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didNotSearch errorDict: [String: NSNumber]) {
        isBrowsing = false
        failed("look for devices", errorDict)
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didFind service: NetService, moreComing: Bool) {
        guard service.name != ownId else { return }
        services[service.name] = service
        service.delegate = self
        service.resolve(withTimeout: 5)
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didRemove service: NetService, moreComing: Bool) {
        services.removeValue(forKey: service.name)?.stop()
        if found.removeValue(forKey: service.name) != nil {
            onChange(Array(found.values))
        }
    }
}

extension Discovery: @preconcurrency NetServiceDelegate {
    func netServiceDidPublish(_ sender: NetService) {
        isPublished = true
    }

    func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        isPublished = false
        failed("announce this device", errorDict)
    }

    func netServiceDidResolveAddress(_ sender: NetService) {
        resolved(sender)
    }

    // A resolve that times out keeps the last known address: the next refresh
    // tries again, and the browser reports a device that actually left.
    func netService(_ sender: NetService, didNotResolve errorDict: [String: NSNumber]) {}
}
