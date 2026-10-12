//! The files this device offered, so it can serve one when a paired device
//! fetches it. A small JSON file beside the pairing keys, since phones have no
//! SQLite history like the desktop's.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use quakboard_sync::offers::{FileStamp, Offer};
use serde::{Deserialize, Serialize};

pub const OFFERS_FILE_NAME: &str = "offers.json";

// ponytail: offers are never removed; a few hundred bytes each. Prune them if
// phones end up offering thousands of files.
#[derive(Serialize, Deserialize)]
struct StoredOffer {
    offer_id: String,
    path: PathBuf,
    size: u64,
    mtime_ms: i64,
    sha256: String,
    offered_to: Vec<String>,
    created_at: String,
}

/// Two offers made at once would otherwise both read the old list and one
/// would be lost.
static WRITING: Mutex<()> = Mutex::new(());

pub fn find(file: &Path, offer_id: &str) -> Option<Offer> {
    load(file)
        .into_iter()
        .find(|stored| stored.offer_id == offer_id)
        .map(|stored| Offer {
            item_id: stored.offer_id.clone(),
            offer_id: stored.offer_id,
            path: stored.path,
            stamp: FileStamp {
                size: stored.size,
                mtime_ms: stored.mtime_ms,
            },
            sha256: stored.sha256,
            offered_to: stored.offered_to,
            created_at: stored.created_at,
        })
}

pub fn add(file: &Path, offer: &Offer) -> io::Result<()> {
    let _writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut offers = load(file);
    offers.push(StoredOffer {
        offer_id: offer.offer_id.clone(),
        path: offer.path.clone(),
        size: offer.stamp.size,
        mtime_ms: offer.stamp.mtime_ms,
        sha256: offer.sha256.clone(),
        offered_to: offer.offered_to.clone(),
        created_at: offer.created_at.clone(),
    });
    // A temporary file and a rename, so a fetch never reads half a list.
    let tmp = file.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec(&offers)?)?;
    fs::rename(&tmp, file)
}

/// A missing or unreadable file is no offers: fetches are then refused.
fn load(file: &Path) -> Vec<StoredOffer> {
    fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}
