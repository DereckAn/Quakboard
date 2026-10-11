import Foundation
import GRDB

struct HistoryItem: Codable, Identifiable, Sendable, FetchableRecord, MutablePersistableRecord {
    enum Kind: String, Codable, Sendable {
        case text, image
    }

    var id: Int64?
    var kind: Kind
    var text: String?
    /// A PNG in the images folder, for `.image`.
    var imageFile: String?
    /// Who sent it. Text doesn't say yet: the sync core's text hook has no sender.
    var fromName: String?
    var createdAt: Date

    static let databaseTableName = "item"

    mutating func didInsert(_ inserted: InsertionSuccess) {
        id = inserted.rowID
    }
}

/// Received text and images. Safe to use from any thread: the delegate saves
/// from Rust's background thread.
final class History: Sendable {
    // ponytail: newest 500 only, no paging; add paging when phones keep more.
    private static let listLimit = 500

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
        try migrator.migrate(db)
    }

    /// The same text again moves to the top instead of being stored twice,
    /// like the desktop.
    func saveText(_ text: String) throws {
        try db.write { db in
            try HistoryItem.filter(Column("kind") == HistoryItem.Kind.text.rawValue && Column("text") == text)
                .deleteAll(db)
            var item = HistoryItem(kind: .text, text: text, createdAt: .now)
            try item.insert(db)
        }
    }

    func saveImage(png: Data, fromName: String) throws {
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

    /// Newest first. `search` matches text, case-insensitively.
    func items(matching search: String) throws -> [HistoryItem] {
        try db.read { db in
            var request = HistoryItem.order(Column("createdAt").desc)
            let search = search.trimmingCharacters(in: .whitespaces)
            if !search.isEmpty {
                request = request.filter(Column("text").like("%\(search)%"))
            }
            return try request.limit(Self.listLimit).fetchAll(db)
        }
    }
}
