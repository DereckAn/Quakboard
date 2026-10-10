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

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{Frame, PeerKey, Sealed, SyncError};

const HASH_BUFFER_BYTES: usize = 1024 * 1024;

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

    const KEY: PeerKey = [6; 32];

    struct Fixture {
        file: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        std::fs::write(&file, b"quarterly numbers").unwrap();
        Fixture { file, _dir: dir }
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
