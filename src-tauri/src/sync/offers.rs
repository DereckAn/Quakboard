//! Files this device offered to paired devices, and the `FileOffer` message
//! that tells them. Bytes only move later, when a device fetches an offer.
//!
//! An offer is the only thing a fetch can name: requests carry an offer id,
//! never a path, so a paired device can only ever read files the user chose
//! to send, and only if it was among the devices they were sent to.

use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Frame, PeerKey, Sealed, SyncError};

const HASH_BUFFER_BYTES: usize = 1024 * 1024;

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

/// A file's size and modification time: cheap to read, and enough to notice
/// that it changed since it was hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub mtime_ms: i64,
}

impl FileStamp {
    pub fn of(path: &Path) -> io::Result<FileStamp> {
        let metadata = std::fs::metadata(path)?;
        let mtime_ms = metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis() as i64)
            .unwrap_or(0);
        Ok(FileStamp {
            size: metadata.len(),
            mtime_ms,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct HashedFile {
    pub stamp: FileStamp,
    /// Lowercase hex SHA-256 of the whole file.
    pub sha256: String,
}

/// Hash the whole file, reporting `(bytes hashed, total)` as it goes. Fails
/// if the file changes while being read, so an offer never records a hash
/// for bytes that no longer exist. Blocking: run it off the async workers.
pub fn hash_file(path: &Path, mut on_progress: impl FnMut(u64, u64)) -> io::Result<HashedFile> {
    let before = FileStamp::of(path)?;
    // Read at most the size it had when we started: a file still being
    // written (a log, a download) would otherwise keep this reading forever.
    let mut file = File::open(path)?.take(before.size);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; HASH_BUFFER_BYTES];
    let mut hashed = 0u64;

    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        hashed += read as u64;
        on_progress(hashed, before.size);
    }

    if FileStamp::of(path)? != before || hashed != before.size {
        return Err(io::Error::other("file changed while it was being hashed"));
    }
    Ok(HashedFile {
        stamp: before,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    pub offer_id: String,
    pub item_id: String,
    pub path: PathBuf,
    pub stamp: FileStamp,
    pub sha256: String,
    /// Paired device ids chosen in the Send picker. Only they may fetch.
    pub offered_to: Vec<String>,
    pub created_at: String,
}

impl Offer {
    pub fn is_offered_to(&self, device_id: &str) -> bool {
        self.offered_to.iter().any(|id| id == device_id)
    }
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

/// What a `FileOffer` tells the receiving device. Sealed, so names and sizes
/// aren't visible on the network. Everything here is untrusted on arrival.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileOfferInfo {
    pub offer_id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub sha256: String,
}

pub fn seal_file_offer(
    from: &str,
    key: &PeerKey,
    info: &FileOfferInfo,
) -> Result<Frame, SyncError> {
    Ok(Frame::FileOffer {
        from: from.to_string(),
        offer: Sealed::seal(key, &offer_aad(from), info)?,
    })
}

pub fn open_file_offer(frame: &Frame, key: &PeerKey) -> Result<FileOfferInfo, SyncError> {
    match frame {
        Frame::FileOffer { from, offer } => offer.open(key, &offer_aad(from)),
        _ => Err(SyncError::Malformed("not a file offer".into())),
    }
}

fn offer_aad(from: &str) -> String {
    format!("file-offer:{from}")
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::db::{models::CreateClipboardItemDto, repository::ClipboardRepository};

    const KEY: PeerKey = [6; 32];

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
    fn hash_is_the_sha256_of_the_whole_file() {
        let fx = fixture();
        let expected = format!("{:x}", Sha256::digest(b"quarterly numbers"));
        assert_eq!(hash_file(&fx.file, |_, _| {}).unwrap().sha256, expected);
    }

    #[test]
    fn hashing_reports_progress_up_to_the_total() {
        let fx = fixture();
        let mut last = (0, 0);
        hash_file(&fx.file, |done, total| last = (done, total)).unwrap();
        assert_eq!(last, (17, 17));
    }

    #[test]
    fn empty_file_hashes_like_empty_input() {
        let fx = fixture();
        std::fs::write(&fx.file, b"").unwrap();
        let expected = format!("{:x}", Sha256::digest(b""));

        assert_eq!(hash_file(&fx.file, |_, _| {}).unwrap().sha256, expected);
    }

    #[test]
    fn hashing_a_file_that_keeps_growing_ends_with_an_error() {
        // A file still being written must not keep the hash reading forever.
        let fx = fixture();
        let path = fx.file.clone();
        let mut reads = 0;
        let result = hash_file(&fx.file, |_, _| {
            reads += 1;
            let mut file = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
            file.write_all(b" and more").unwrap();
        });

        assert_eq!((result.is_err(), reads), (true, 1));
    }

    #[test]
    fn missing_file_cannot_be_hashed() {
        let fx = fixture();
        std::fs::remove_file(&fx.file).unwrap();
        assert!(hash_file(&fx.file, |_, _| {}).is_err());
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

    fn info() -> FileOfferInfo {
        FileOfferInfo {
            offer_id: "offer-1".into(),
            name: "report.pdf".into(),
            size: 17,
            mime: "application/pdf".into(),
            sha256: "abc".into(),
        }
    }

    #[test]
    fn sealed_offer_opens_with_the_same_key() {
        let frame = seal_file_offer("device-a", &KEY, &info()).unwrap();
        assert_eq!(open_file_offer(&frame, &KEY), Ok(info()));
    }

    #[test]
    fn sealed_offer_hides_the_file_name() {
        let frame = seal_file_offer("device-a", &KEY, &info()).unwrap();
        assert!(!serde_json::to_string(&frame)
            .unwrap()
            .contains("report.pdf"));
    }

    #[test]
    fn offer_relabelled_with_another_sender_is_rejected() {
        let Frame::FileOffer { offer, .. } = seal_file_offer("device-a", &KEY, &info()).unwrap()
        else {
            panic!("expected a file offer");
        };
        let spoofed = Frame::FileOffer {
            from: "device-b".into(),
            offer,
        };

        assert_eq!(open_file_offer(&spoofed, &KEY), Err(SyncError::Decrypt));
    }

    #[test]
    fn non_offer_frame_is_rejected() {
        assert!(matches!(
            open_file_offer(&Frame::PairDone, &KEY),
            Err(SyncError::Malformed(_))
        ));
    }
}
