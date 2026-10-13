import Foundation
import GRDB
import QuakboardSync

struct HistoryItem: Codable, Identifiable, Sendable, FetchableRecord, MutablePersistableRecord {
    /// The desktop's content types for text, plus images and offered files.
    enum Kind: String, Codable, Sendable, CaseIterable {
        case text, link, code, color, image, file

        /// What the sender called some text; anything unknown is plain text.
        init(contentType: String) {
            let kind = Kind(rawValue: contentType) ?? .text
            self = kind == .image || kind == .file ? .text : kind
        }
    }

    var id: Int64?
    var kind: Kind
    var text: String?
    /// A PNG in the images folder, for `.image`.
    var imageFile: String?
    /// Who sent it. Text doesn't say yet: the sync core's text hook has no sender.
    var fromName: String?
    var createdAt: Date
    var isPinned = false
    /// For `.file`: the offer, fetched only when the user asks.
    var offerId: String?
    var originId: String?
    var fileName: String?
    var fileSize: Int64?
    var fileMime: String?
    var sha256: String?
    /// For `.file`: its name in Documents once fetched.
    var savedFile: String?

    /// What `fetchFile` needs, for a `.file` item.
    var remoteFile: RemoteFileInfo? {
        guard kind == .file, let offerId, let originId, let fileName, let fileSize, let sha256 else { return nil }
        return RemoteFileInfo(
            offerId: offerId,
            originId: originId,
            originName: fromName ?? "",
            name: fileName,
            size: UInt64(fileSize),
            mime: fileMime ?? "application/octet-stream",
            sha256: sha256
        )
    }

    static let databaseTableName = "item"

    mutating func didInsert(_ inserted: InsertionSuccess) {
        id = inserted.rowID
    }
}

/// What the clips screens show.
struct HistoryPage: Sendable {
    var pinned: [HistoryItem] = []
    var recent: [HistoryItem] = []
    /// Over the whole history, for the filter chips and the iPad sidebar.
    var counts: [HistoryItem.Kind: Int] = [:]
    var pinnedCount = 0
}

/// Received and sent clips. Safe to use from any thread: the delegate saves
/// from Rust's background thread.
final class History: Sendable {
    // ponytail: newest 500 unpinned only, no paging; add paging when phones keep more.
    private static let recentLimit = 500

    let imagesDir: URL
    private let db: DatabaseQueue

    init(dir: URL) throws {
        imagesDir = dir.appending(path: "images", directoryHint: .isDirectory)
        try FileManager.default.createDirectory(at: imagesDir, withIntermediateDirectories: true)
        db = try DatabaseQueue(path: dir.appending(path: "history.sqlite").path(percentEncoded: false))

        var migrator = DatabaseMigrator()
        migrator.registerMigration("v1") { db in
            try db.create(table: HistoryItem.databaseTableName) { t in
                t.autoIncrementedPrimaryKey("id")
                t.column("kind", .text).notNull()
                t.column("text", .text)
                t.column("imageFile", .text)
                t.column("fromName", .text)
                t.column("createdAt", .datetime).notNull().indexed()
            }
        }
        migrator.registerMigration("v2: pinning") { db in
            try db.alter(table: HistoryItem.databaseTableName) { t in
                t.add(column: "isPinned", .boolean).notNull().defaults(to: false)
            }
        }
        migrator.registerMigration("v3: detect text types") { db in
            // Clips sent from this device used to be saved as plain text.
            let rows = try Row.fetchAll(db, sql: "SELECT id, text FROM item WHERE kind = 'text'")
            for row in rows {
                guard let text: String = row["text"] else { continue }
                let kind = HistoryItem.Kind(contentType: detectContentType(text: text))
                if kind != .text {
                    try db.execute(sql: "UPDATE item SET kind = ? WHERE id = ?", arguments: [kind.rawValue, row["id"]])
                }
            }
        }
        migrator.registerMigration("v4: file offers") { db in
            try db.alter(table: HistoryItem.databaseTableName) { t in
                t.add(column: "offerId", .text)
                t.add(column: "originId", .text)
                t.add(column: "fileName", .text)
                t.add(column: "fileSize", .integer)
                t.add(column: "fileMime", .text)
                t.add(column: "sha256", .text)
                t.add(column: "savedFile", .text)
            }
        }
        try migrator.migrate(db)
    }

    /// The same text again moves to the top instead of being stored twice,
    /// like the desktop, and keeps its pin.
    func saveText(_ text: String, kind: HistoryItem.Kind) throws {
        try db.write { db in
            let textKinds = HistoryItem.Kind.allCases.filter { $0 != .image }.map(\.rawValue)
            if var item = try HistoryItem
                .filter(textKinds.contains(Column("kind")) && Column("text") == text)
                .fetchOne(db) {
                item.createdAt = .now
                item.kind = kind
                try item.update(db)
            } else {
                var item = HistoryItem(kind: kind, text: text, createdAt: .now)
                try item.insert(db)
            }
        }
    }

    /// The same offer again moves to the top instead of being stored twice.
    func saveOffer(_ file: RemoteFileInfo) throws {
        try db.write { db in
            if var item = try HistoryItem.filter(Column("offerId") == file.offerId).fetchOne(db) {
                item.createdAt = .now
                try item.update(db)
            } else {
                var item = HistoryItem(
                    kind: .file,
                    fromName: file.originName,
                    createdAt: .now,
                    offerId: file.offerId,
                    originId: file.originId,
                    fileName: file.name,
                    fileSize: Int64(clamping: file.size),
                    fileMime: file.mime,
                    sha256: file.sha256
                )
                try item.insert(db)
            }
        }
    }

    /// A file this device offered: already "saved", since it's in Documents.
    func saveSentFile(name: String, size: Int64, mime: String) throws {
        try db.write { db in
            var item = HistoryItem(
                kind: .file,
                createdAt: .now,
                fileName: name,
                fileSize: size,
                fileMime: mime,
                savedFile: name
            )
            try item.insert(db)
        }
    }

    func setSavedFile(_ id: Int64, name: String) throws {
        try db.write { db in
            _ = try HistoryItem.filter(id: id).updateAll(db, Column("savedFile").set(to: name))
        }
    }

    func saveImage(png: Data, fromName: String?) throws {
        let file = "\(UUID().uuidString).png"
        let url = imagesDir.appending(path: file)
        try png.write(to: url, options: .completeFileProtection)
        do {
            try db.write { db in
                var item = HistoryItem(kind: .image, imageFile: file, fromName: fromName, createdAt: .now)
                try item.insert(db)
            }
        } catch {
            try? FileManager.default.removeItem(at: url)
            throw error
        }
    }

    /// Removes the clip and, for an image, its file.
    func delete(_ id: Int64) throws {
        let file = try db.write { db -> String? in
            let file = try HistoryItem.fetchOne(db, id: id)?.imageFile
            _ = try HistoryItem.deleteOne(db, id: id)
            return file
        }
        if let file {
            try? FileManager.default.removeItem(at: imagesDir.appending(path: file))
        }
    }

    func setPinned(_ id: Int64, _ isPinned: Bool) throws {
        try db.write { db in
            _ = try HistoryItem.filter(id: id).updateAll(db, Column("isPinned").set(to: isPinned))
        }
    }

    /// Newest first. `search` matches text and file names, case-insensitively;
    /// `kind` nil is all.
    func page(search: String, kind: HistoryItem.Kind?) throws -> HistoryPage {
        try db.read { db in
            var request = HistoryItem.order(Column("createdAt").desc)
            let search = search.trimmingCharacters(in: .whitespaces)
            if !search.isEmpty {
                let pattern = "%\(search)%"
                request = request.filter(Column("text").like(pattern) || Column("fileName").like(pattern))
            }
            if let kind {
                request = request.filter(Column("kind") == kind.rawValue)
            }
            let counts = try Row.fetchAll(db, sql: "SELECT kind, COUNT(*) FROM item GROUP BY kind")
                .reduce(into: [HistoryItem.Kind: Int]()) { counts, row in
                    if let kind = HistoryItem.Kind(rawValue: row[0]) { counts[kind] = row[1] }
                }
            return HistoryPage(
                pinned: try request.filter(Column("isPinned")).fetchAll(db),
                recent: try request.filter(!Column("isPinned")).limit(Self.recentLimit).fetchAll(db),
                counts: counts,
                pinnedCount: try HistoryItem.filter(Column("isPinned")).fetchCount(db)
            )
        }
    }
}
