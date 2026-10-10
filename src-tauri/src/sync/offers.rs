//! Where this device keeps the files it offered: the `sync_offers` table.
//! What an offer is, and the `FileOffer` message, live in `quakboard_sync`.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};

pub use quakboard_sync::offers::{
    hash_file, open_file_offer, seal_file_offer, FileOfferInfo, FileStamp, HashedFile, Offer,
};

/// Creates the offers table. Offers are revoked by a trigger whenever their
/// clipboard item is deleted, so every delete path (single, clear all,
/// retention, item limit, duplicates) revokes without having to remember.
pub fn init_offers_table(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sync_offers (
             offer_id   TEXT PRIMARY KEY,
             item_id    TEXT NOT NULL,
             path       TEXT NOT NULL,
             size       INTEGER NOT NULL,
             mtime_ms   INTEGER NOT NULL,
             sha256     TEXT NOT NULL,
             offered_to TEXT NOT NULL,
             created_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_sync_offers_item ON sync_offers(item_id);
         CREATE TRIGGER IF NOT EXISTS sync_offers_revoke_with_item
             AFTER DELETE ON clipboard_items
         BEGIN
             DELETE FROM sync_offers WHERE item_id = old.id;
         END;",
    )
}

pub fn create_offer(
    conn: &Connection,
    item_id: &str,
    path: &Path,
    hashed: &HashedFile,
    offered_to: &[String],
) -> rusqlite::Result<Offer> {
    let offer = Offer {
        offer_id: uuid::Uuid::new_v4().to_string(),
        item_id: item_id.to_string(),
        path: path.to_path_buf(),
        stamp: hashed.stamp,
        sha256: hashed.sha256.clone(),
        offered_to: offered_to.to_vec(),
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    conn.execute(
        "INSERT INTO sync_offers
             (offer_id, item_id, path, size, mtime_ms, sha256, offered_to, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            offer.offer_id,
            offer.item_id,
            offer.path.to_string_lossy(),
            offer.stamp.size as i64,
            offer.stamp.mtime_ms,
            offer.sha256,
            serde_json::to_string(&offer.offered_to).unwrap_or_else(|_| "[]".into()),
            offer.created_at,
        ],
    )?;
    Ok(offer)
}

pub fn find_offer(conn: &Connection, offer_id: &str) -> rusqlite::Result<Option<Offer>> {
    conn.query_row(
        "SELECT offer_id, item_id, path, size, mtime_ms, sha256, offered_to, created_at
         FROM sync_offers WHERE offer_id = ?1",
        [offer_id],
        |row| {
            let offered_to: String = row.get(6)?;
            Ok(Offer {
                offer_id: row.get(0)?,
                item_id: row.get(1)?,
                path: PathBuf::from(row.get::<_, String>(2)?),
                stamp: FileStamp {
                    size: row.get::<_, i64>(3)? as u64,
                    mtime_ms: row.get(4)?,
                },
                sha256: row.get(5)?,
                // A malformed list offers to no one rather than to everyone.
                offered_to: serde_json::from_str(&offered_to).unwrap_or_default(),
                created_at: row.get(7)?,
            })
        },
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{models::CreateClipboardItemDto, repository::ClipboardRepository};

    struct Fixture {
        db_path: PathBuf,
        file: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"quarterly numbers").unwrap();
        Fixture {
            db_path: dir.path().join("clipboard.db"),
            file,
            _dir: dir,
        }
    }

    fn repo(fx: &Fixture) -> ClipboardRepository {
        ClipboardRepository::new(fx.db_path.to_str().unwrap()).unwrap()
    }

    fn file_item(repo: &ClipboardRepository) -> String {
        repo.create_item(CreateClipboardItemDto {
            content_type: "file".into(),
            content_text: "report.pdf".into(),
            content_metadata: None,
            source_app: None,
            code_language: None,
        })
        .unwrap()
        .id
    }

    fn offer(fx: &Fixture, repo: &ClipboardRepository, item_id: &str) -> Offer {
        let hashed = hash_file(&fx.file, |_, _| {}).unwrap();
        create_offer(&repo.conn, item_id, &fx.file, &hashed, &["mac".into()]).unwrap()
    }

    #[test]
    fn stored_offer_can_be_found() {
        let fx = fixture();
        let repo = repo(&fx);
        let created = offer(&fx, &repo, &file_item(&repo));

        assert_eq!(
            find_offer(&repo.conn, &created.offer_id).unwrap(),
            Some(created)
        );
    }

    #[test]
    fn offer_survives_a_restart() {
        let fx = fixture();
        let created = {
            let repo = repo(&fx);
            offer(&fx, &repo, &file_item(&repo))
        };

        assert_eq!(
            find_offer(&repo(&fx).conn, &created.offer_id).unwrap(),
            Some(created)
        );
    }

    #[test]
    fn deleting_the_item_revokes_its_offers() {
        let fx = fixture();
        let repo = repo(&fx);
        let item_id = file_item(&repo);
        let created = offer(&fx, &repo, &item_id);
        repo.delete_item(&item_id).unwrap();

        assert_eq!(find_offer(&repo.conn, &created.offer_id).unwrap(), None);
    }

    #[test]
    fn clearing_history_revokes_every_offer() {
        let fx = fixture();
        let repo = repo(&fx);
        let created = offer(&fx, &repo, &file_item(&repo));
        repo.clear_all().unwrap();

        assert_eq!(find_offer(&repo.conn, &created.offer_id).unwrap(), None);
    }

    #[test]
    fn deleting_one_item_keeps_other_items_offers() {
        let fx = fixture();
        let repo = repo(&fx);
        let (doomed, kept) = (file_item(&repo), file_item(&repo));
        offer(&fx, &repo, &doomed);
        let survivor = offer(&fx, &repo, &kept);
        repo.delete_item(&doomed).unwrap();

        assert!(find_offer(&repo.conn, &survivor.offer_id)
            .unwrap()
            .is_some());
    }

    #[test]
    fn offer_is_only_for_the_chosen_devices() {
        let fx = fixture();
        let repo = repo(&fx);
        let created = offer(&fx, &repo, &file_item(&repo));

        assert_eq!(
            (
                created.is_offered_to("mac"),
                created.is_offered_to("laptop")
            ),
            (true, false)
        );
    }

    #[test]
    fn offer_records_the_files_stamp() {
        let fx = fixture();
        let repo = repo(&fx);
        let created = offer(&fx, &repo, &file_item(&repo));

        assert_eq!(created.stamp, FileStamp::of(&fx.file).unwrap());
    }
}
