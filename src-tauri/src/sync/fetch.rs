//! Fetching an offered file: the request, the owner's answer, and the
//! owner's decision whether to serve it.
//!
//! ```text
//! requester -> owner   FileFetch { from, request: sealed { offer_id } }
//! owner -> requester   FileReply { reply: sealed Start { salt, size } | Refused { reason } }
//! owner -> requester   the file, as `stream` chunks
//! ```
//!
//! A request names an offer id, never a path, and the owner serves only
//! files the user sent to that very device, unchanged since they were sent.

use std::{io, path::Path, time::Duration};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};

use super::{
    offers::{FileStamp, HashedFile, Offer},
    stream::SALT_BYTES,
    Frame, PeerKey, Sealed, SyncError,
};

/// Longest pause between chunks before a transfer is given up. Generous:
/// files come off disks that can be slow, and there's no total cap.
pub const FILE_IDLE_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetchRequest {
    pub offer_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RefusalReason {
    /// No such offer, or not offered to this device. One answer for both, so
    /// a device can't probe which offers exist.
    NotShared,
    /// The file was moved or deleted since it was sent.
    Missing,
    /// The file's content changed since it was sent.
    Changed,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FetchReply {
    /// The stream follows, encrypted under this salt.
    Start {
        salt: String,
        size: u64,
    },
    Refused {
        reason: RefusalReason,
    },
}

pub fn seal_fetch_request(from: &str, key: &PeerKey, offer_id: &str) -> Result<Frame, SyncError> {
    let request = FetchRequest {
        offer_id: offer_id.to_string(),
    };
    Ok(Frame::FileFetch {
        from: from.to_string(),
        request: Sealed::seal(key, &request_aad(from), &request)?,
    })
}

pub fn open_fetch_request(frame: &Frame, key: &PeerKey) -> Result<FetchRequest, SyncError> {
    match frame {
        Frame::FileFetch { from, request } => request.open(key, &request_aad(from)),
        _ => Err(SyncError::Malformed("not a fetch request".into())),
    }
}

/// The reply is bound to its offer, so it can't answer a different fetch.
pub fn seal_fetch_reply(
    key: &PeerKey,
    offer_id: &str,
    reply: &FetchReply,
) -> Result<Frame, SyncError> {
    Ok(Frame::FileReply {
        reply: Sealed::seal(key, &reply_aad(offer_id), reply)?,
    })
}

pub fn open_fetch_reply(
    frame: &Frame,
    key: &PeerKey,
    offer_id: &str,
) -> Result<FetchReply, SyncError> {
    match frame {
        Frame::FileReply { reply } => reply.open(key, &reply_aad(offer_id)),
        _ => Err(SyncError::Malformed("not a fetch reply".into())),
    }
}

/// What the stream's chunks are bound to; both sides must compute the same.
pub fn stream_context(offer_id: &str, owner_id: &str) -> String {
    format!("file:{offer_id}:{owner_id}")
}

pub fn encode_salt(salt: &[u8; SALT_BYTES]) -> String {
    BASE64.encode(salt)
}

pub fn decode_salt(salt: &str) -> Result<[u8; SALT_BYTES], SyncError> {
    BASE64
        .decode(salt)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| SyncError::Malformed("bad stream salt".into()))
}

/// Should the owner serve `offer` to `requester`? `rehash` reads the whole
/// file, and only runs when its size or modification time changed since it
/// was offered. Blocking: run it off the async workers.
pub fn decide(
    offer: Option<Offer>,
    requester: &str,
    rehash: impl FnOnce(&Path) -> io::Result<HashedFile>,
) -> Result<Offer, RefusalReason> {
    let offer = offer
        .filter(|offer| offer.is_offered_to(requester))
        .ok_or(RefusalReason::NotShared)?;

    let current = FileStamp::of(&offer.path).map_err(|_| RefusalReason::Missing)?;
    if current == offer.stamp {
        return Ok(offer);
    }

    // Touched (e.g. copied or synced) but maybe not changed: only the
    // content decides.
    let rehashed = rehash(&offer.path).map_err(|_| RefusalReason::Missing)?;
    if rehashed.sha256 == offer.sha256 && rehashed.stamp.size == offer.stamp.size {
        Ok(Offer {
            stamp: rehashed.stamp,
            ..offer
        })
    } else {
        Err(RefusalReason::Changed)
    }
}

fn request_aad(from: &str) -> String {
    format!("file-fetch:{from}")
}

fn reply_aad(offer_id: &str) -> String {
    format!("file-reply:{offer_id}")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, SystemTime},
    };

    use super::*;
    use crate::sync::offers::hash_file;

    const KEY: PeerKey = [5; 32];

    struct Fixture {
        file: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("report.pdf");
        fs::write(&file, b"original content").unwrap();
        Fixture { file, _dir: dir }
    }

    fn offer_of(fx: &Fixture, offered_to: &[&str]) -> Offer {
        let hashed = hash_file(&fx.file, |_, _| {}).unwrap();
        Offer {
            offer_id: "offer-1".into(),
            item_id: "item-1".into(),
            path: fx.file.clone(),
            stamp: hashed.stamp,
            sha256: hashed.sha256,
            offered_to: offered_to.iter().map(|id| id.to_string()).collect(),
            created_at: String::new(),
        }
    }

    fn rehash(path: &Path) -> io::Result<HashedFile> {
        hash_file(path, |_, _| {})
    }

    /// Change the modification time without touching the content.
    fn touch(path: &Path) {
        let later = SystemTime::now() + Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(later)
            .unwrap();
    }

    #[test]
    fn unchanged_file_is_served_to_its_device() {
        let fx = fixture();
        let offer = offer_of(&fx, &["mac"]);
        assert_eq!(decide(Some(offer.clone()), "mac", rehash), Ok(offer));
    }

    #[test]
    fn unchanged_file_is_served_without_rehashing() {
        let fx = fixture();
        let result = decide(Some(offer_of(&fx, &["mac"])), "mac", |_| {
            panic!("an unchanged file must not be re-read")
        });
        assert!(result.is_ok());
    }

    #[test]
    fn unknown_offer_is_not_shared() {
        assert_eq!(decide(None, "mac", rehash), Err(RefusalReason::NotShared));
    }

    #[test]
    fn device_it_was_not_offered_to_is_refused() {
        let fx = fixture();
        assert_eq!(
            decide(Some(offer_of(&fx, &["mac"])), "laptop", rehash),
            Err(RefusalReason::NotShared)
        );
    }

    #[test]
    fn deleted_file_is_missing() {
        let fx = fixture();
        let offer = offer_of(&fx, &["mac"]);
        fs::remove_file(&fx.file).unwrap();

        assert_eq!(
            decide(Some(offer), "mac", rehash),
            Err(RefusalReason::Missing)
        );
    }

    #[test]
    fn edited_file_is_changed() {
        let fx = fixture();
        let offer = offer_of(&fx, &["mac"]);
        // Same size, so only the timestamp shows the edit; set it explicitly,
        // since two writes inside the filesystem's time resolution share one.
        fs::write(&fx.file, b"edited content!!").unwrap();
        touch(&fx.file);

        assert_eq!(
            decide(Some(offer), "mac", rehash),
            Err(RefusalReason::Changed)
        );
    }

    #[test]
    fn file_that_grew_is_changed() {
        let fx = fixture();
        let offer = offer_of(&fx, &["mac"]);
        fs::write(&fx.file, b"original content, plus more").unwrap();

        assert_eq!(
            decide(Some(offer), "mac", rehash),
            Err(RefusalReason::Changed)
        );
    }

    #[test]
    fn touched_but_identical_file_is_still_served() {
        let fx = fixture();
        let offer = offer_of(&fx, &["mac"]);
        touch(&fx.file);

        assert!(decide(Some(offer), "mac", rehash).is_ok());
    }

    #[test]
    fn request_round_trips() {
        let frame = seal_fetch_request("device-b", &KEY, "offer-1").unwrap();
        assert_eq!(
            open_fetch_request(&frame, &KEY),
            Ok(FetchRequest {
                offer_id: "offer-1".into()
            })
        );
    }

    #[test]
    fn request_relabelled_with_another_sender_is_rejected() {
        let Frame::FileFetch { request, .. } =
            seal_fetch_request("device-b", &KEY, "offer-1").unwrap()
        else {
            panic!("expected a fetch request");
        };
        let spoofed = Frame::FileFetch {
            from: "device-c".into(),
            request,
        };

        assert_eq!(open_fetch_request(&spoofed, &KEY), Err(SyncError::Decrypt));
    }

    #[test]
    fn reply_round_trips() {
        let reply = FetchReply::Start {
            salt: encode_salt(&[3; SALT_BYTES]),
            size: 16,
        };
        let frame = seal_fetch_reply(&KEY, "offer-1", &reply).unwrap();

        assert_eq!(open_fetch_reply(&frame, &KEY, "offer-1"), Ok(reply));
    }

    #[test]
    fn reply_for_another_offer_is_rejected() {
        let reply = FetchReply::Refused {
            reason: RefusalReason::Missing,
        };
        let frame = seal_fetch_reply(&KEY, "offer-1", &reply).unwrap();

        assert_eq!(
            open_fetch_reply(&frame, &KEY, "offer-2"),
            Err(SyncError::Decrypt)
        );
    }

    #[test]
    fn salt_round_trips() {
        let salt = [7; SALT_BYTES];
        assert_eq!(decode_salt(&encode_salt(&salt)), Ok(salt));
    }

    #[test]
    fn salt_of_the_wrong_length_is_rejected() {
        assert!(decode_salt(&BASE64.encode([1u8; 4])).is_err());
    }
}
