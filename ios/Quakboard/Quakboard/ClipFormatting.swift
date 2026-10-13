import SwiftUI

/// How a clip is described on screen, shared by its card and its detail view.
extension HistoryItem {
    /// Without the trailing newline most copied text ends with, which would
    /// show as an empty line.
    var trimmedText: String {
        String((text ?? "").reversed().drop { $0.isNewline }.reversed())
    }

    var host: String? {
        URL(string: (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines))?.host
    }

    /// `#RGB` or `#RRGGBB`. The desktop also detects rgb(), hsl() and
    /// oklch(); those show no swatch.
    var swatch: Color? {
        var hex = (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        guard kind == .color, hex.hasPrefix("#") else { return nil }
        hex.removeFirst()
        if hex.count == 3 {
            hex = hex.map { "\($0)\($0)" }.joined()
        }
        guard hex.count == 6, let value = UInt32(hex, radix: 16) else { return nil }
        return Color(hex: value)
    }

    /// Clips have no names, so this is their type, or what identifies them.
    var title: String {
        switch kind {
        case .text: "Text"
        case .link: host ?? "Link"
        case .code: "Code"
        case .color: (text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
        case .image: "Image"
        case .file: fileName ?? "File"
        }
    }

    var formatName: String {
        switch kind {
        case .text: "Plain text"
        case .link: "Link"
        case .code: "Code"
        case .color: "Color"
        case .image: "PNG image"
        case .file: fileMime ?? "File"
        }
    }

    var systemImage: String {
        kind.systemImage
    }

    var sizeDescription: String {
        switch kind {
        case .image:
            return "PNG"
        case .file:
            return ByteCountFormatter.string(fromByteCount: fileSize ?? 0, countStyle: .file)
        case .code:
            let lines = trimmedText.split(separator: "\n", omittingEmptySubsequences: false).count
            return "\(lines) lines • \((text ?? "").utf8.count) bytes"
        case .text, .link, .color:
            return "\((text ?? "").count) chars"
        }
    }
}

extension HistoryItem.Kind {
    /// The order of the filter chips and the iPad sidebar's types.
    static let filterOrder: [Self] = [.link, .code, .color, .image, .file, .text]

    var pluralTitle: String {
        switch self {
        case .text: "Plain Text"
        case .link: "Links"
        case .code: "Code"
        case .color: "Colors"
        case .image: "Images"
        case .file: "Files"
        }
    }

    var systemImage: String {
        switch self {
        case .text: "text.alignleft"
        case .link: "link"
        case .code: "curlybraces"
        case .color: "paintpalette"
        case .image: "photo"
        case .file: "doc"
        }
    }
}

extension SyncModel.FileState {
    var actionTitle: String {
        switch self {
        case .remote: "Fetch"
        case .fetching: "Cancel"
        case .saved: "Open"
        }
    }

    var actionImage: String {
        switch self {
        case .remote: "arrow.down.circle"
        case .fetching: "xmark"
        case .saved: "eye"
        }
    }

    func status(origin: String?) -> String {
        switch self {
        case .remote: "On \(origin ?? "another device")"
        case .fetching(let fraction): "Downloading… \(Int(fraction * 100))%"
        case .saved: origin == nil ? "Sent from this device · in Files" : "Saved in Files"
        }
    }
}
