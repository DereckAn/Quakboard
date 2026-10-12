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
    private var timer: Timer?
    private var isPublished = false
    private var isBrowsing = false
    // Separate, so one recovering doesn't hide the other still failing.
    private var publishProblem: String?
    private var browseProblem: String?

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
        timer = Timer.scheduledTimer(withTimeInterval: Self.refreshInterval, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.refresh() }
        }
    }

    /// Stop advertising and looking. A stopped Discovery isn't restarted;
    /// make a new one.
    func stop() {
        timer?.invalidate()
        advertised.stop()
        browser.stop()
        services.values.forEach { $0.stop() }
        services = [:]
    }

    /// This device's LAN IPv4, Wi-Fi first, for "connect by IP" on a desktop.
    static func localIPv4() -> String? {
        var list: UnsafeMutablePointer<ifaddrs>?
        guard getifaddrs(&list) == 0, let first = list else { return nil }
        defer { freeifaddrs(list) }
        let found = sequence(first: first) { $0.pointee.ifa_next }.compactMap { entry -> (name: String, ip: String)? in
            guard let address = entry.pointee.ifa_addr,
                  address.pointee.sa_family == sa_family_t(AF_INET),
                  let ip = numericHost(address, length: socklen_t(address.pointee.sa_len)),
                  !ip.hasPrefix("127.") else { return nil }
            return (String(cString: entry.pointee.ifa_name), ip)
        }
        return (found.first { $0.name == "en0" } ?? found.first)?.ip
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
        reportFound()
    }

    /// One entry per device. Bonjour can show a device under several names
    /// (it renames an announcement that collides with an older one, like
    /// "id (2)"); the id in its TXT record is what identifies it.
    private func reportFound() {
        let byID = Dictionary(found.values.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        onChange(Array(byID.values))
    }

    private static func problem(_ action: String, _ errorDict: [String: NSNumber]) -> String {
        let code = errorDict[NetService.errorCode]?.intValue ?? 0
        return code == Self.policyDenied
            ? "Allow Local Network for Quakboard in Settings > Privacy & Security to find nearby devices."
            : "Couldn't \(action) on the network (error \(code)). Retrying…"
    }

    private func reportProblems() {
        onProblem(browseProblem ?? publishProblem)
    }

    /// A LAN address first, like the desktop.
    private static func pickIPv4(_ addresses: [Data]) -> String? {
        let ips = addresses.compactMap { data -> String? in
            data.withUnsafeBytes { raw in
                guard let address = raw.baseAddress?.assumingMemoryBound(to: sockaddr.self),
                      address.pointee.sa_family == sa_family_t(AF_INET) else { return nil }
                return numericHost(address, length: socklen_t(data.count))
            }
        }
        // 169.254.x.x only works over a direct link between two devices, and
        // 127.x only from the same machine.
        let rank = { (ip: String) in ip.hasPrefix("127.") ? 2 : ip.hasPrefix("169.254.") ? 1 : 0 }
        return ips.min { rank($0) < rank($1) }
    }

    private static func numericHost(_ address: UnsafePointer<sockaddr>, length: socklen_t) -> String? {
        var host = [CChar](repeating: 0, count: Int(NI_MAXHOST))
        guard getnameinfo(address, length, &host, socklen_t(host.count), nil, 0, NI_NUMERICHOST) == 0 else {
            return nil
        }
        return String(decoding: host.prefix { $0 != 0 }.map(UInt8.init(bitPattern:)), as: UTF8.self)
    }
}

extension Discovery: @preconcurrency NetServiceBrowserDelegate {
    func netServiceBrowserWillSearch(_ browser: NetServiceBrowser) {
        isBrowsing = true
        browseProblem = nil
        reportProblems()
    }

    func netServiceBrowser(_ browser: NetServiceBrowser, didNotSearch errorDict: [String: NSNumber]) {
        isBrowsing = false
        browseProblem = Self.problem("look for devices", errorDict)
        reportProblems()
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
            reportFound()
        }
    }
}

extension Discovery: @preconcurrency NetServiceDelegate {
    func netServiceDidPublish(_ sender: NetService) {
        isPublished = true
        publishProblem = nil
        reportProblems()
    }

    func netService(_ sender: NetService, didNotPublish errorDict: [String: NSNumber]) {
        isPublished = false
        publishProblem = Self.problem("announce this device", errorDict)
        reportProblems()
    }

    func netServiceDidResolveAddress(_ sender: NetService) {
        resolved(sender)
    }

    // A resolve that times out keeps the last known address: the next refresh
    // tries again, and the browser reports a device that actually left.
    func netService(_ sender: NetService, didNotResolve errorDict: [String: NSNumber]) {}
}
