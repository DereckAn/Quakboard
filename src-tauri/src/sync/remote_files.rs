//! Files that paired devices offered to this one, kept in history as a "remote" item until it's fetched.
//!
//! A remote item is a `file` item with no `file_url` (its bytes are still on
//! the other device) and `content_metadata.remote` describing the offer. Its
//! `file_hash` is the offer's full SHA-256, the same hash fetched files and
//! clipboard images store, so the same file arriving again is recognized.

use serde_json::{json, Value};

use super::{fetching::RemoteFile, offers::FileOfferInfo};
use crate::db::{
    models::{ClipboardItem, CreateClipboardItemDto},
    repository::ClipboardRepository,
};

/// How an offer ended up in history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferOutcome {
    /// A new remote item, waiting to be fetched.
    New,
    /// The same file was already offered; its item was refreshed.
    Refreshed,
    /// This device already has the file; nothing to fetch.
    AlreadyHere,
}

/// Store a validated offer from `origin_id` / `origin_name`.
pub fn store_file_offer(
    repo: &ClipboardRepository,
    info: &FileOfferInfo,
    origin_id: &str,
    origin_name: &str,
) -> Result<(ClipboardItem, OfferOutcome), String> {
    let existing = repo
        .find_by_file_hash(&info.sha256)
        .map_err(|e| format!("Failed to look up file by hash: {e}"))?;
    let Some(existing) = existing else {
        let item = create_remote_item(repo, info, origin_id, origin_name)?;
        return Ok((item, OfferOutcome::New));
    };

    let mut metadata = parse_metadata(&existing.content_metadata);
    let outcome = if metadata.get("remote").is_some() {
        metadata["remote"] = remote_info(info, origin_id, origin_name);
        OfferOutcome::Refreshed
    } else {
        note_also_on(&mut metadata, origin_id, origin_name);
        OfferOutcome::AlreadyHere
    };
    repo.update_metadata(&existing.id, &metadata.to_string())
        .map_err(|e| format!("Failed to update offered item: {e}"))?;
    let item = repo
        .bump_item(&existing.id)
        .map_err(|e| format!("Failed to bump offered item: {e}"))?;
    Ok((item, outcome))
}

fn create_remote_item(
    repo: &ClipboardRepository,
    info: &FileOfferInfo,
    origin_id: &str,
    origin_name: &str,
) -> Result<ClipboardItem, String> {
    let metadata = json!({
        "remote": remote_info(info, origin_id, origin_name),
        "original_name": info.name,
        "source": "remote",
    });
    let mut item = repo
        .create_item(CreateClipboardItemDto {
            content_type: "file".into(),
            content_text: info.name.clone(),
            content_metadata: Some(metadata.to_string()),
            source_app: Some(format!("Shared from {origin_name}")),
            code_language: None,
        })
        .map_err(|e| format!("Failed to save offered file: {e}"))?;

    // No file_url: the bytes are still on the other device. That also keeps
    // the item out of the missing-file cleanup, which only looks at items
    // that have one.
    repo.conn
        .execute(
            "UPDATE clipboard_items
             SET file_name = ?1, file_size_bytes = ?2, file_mime_type = ?3, file_hash = ?4
             WHERE id = ?5",
            rusqlite::params![info.name, info.size as i64, info.mime, info.sha256, item.id],
        )
        .map_err(|e| format!("Failed to record offered file: {e}"))?;

    item.file_name = Some(info.name.clone());
    item.file_size_bytes = Some(info.size as i64);
    item.file_mime_type = Some(info.mime.clone());
    item.file_hash = Some(info.sha256.clone());
    Ok(item)
}

/// The offer a remote item records, or None for a local item.
pub fn remote_file_of(item: &ClipboardItem) -> Option<RemoteFile> {
    let metadata: Value = serde_json::from_str(&item.content_metadata).ok()?;
    let remote = metadata.get("remote")?;
    let text = |key: &str| remote.get(key)?.as_str().map(str::to_string);
    Some(RemoteFile {
        offer_id: text("offer_id")?,
        origin_id: text("origin_device_id")?,
        origin_name: text("origin_name")?,
        name: item.file_name.clone()?,
        size: remote.get("size")?.as_u64()?,
        sha256: text("sha256")?,
    })
}

fn remote_info(info: &FileOfferInfo, origin_id: &str, origin_name: &str) -> Value {
    json!({
        "offer_id": info.offer_id,
        "origin_device_id": origin_id,
        "origin_name": origin_name,
        "sha256": info.sha256,
        "size": info.size,
        "mime": info.mime,
    })
}

/// Remember that another device has this file too, once per device.
fn note_also_on(metadata: &mut Value, origin_id: &str, origin_name: &str) {
    let entry = json!({ "device_id": origin_id, "name": origin_name });
    match metadata.get_mut("also_on").and_then(Value::as_array_mut) {
        Some(list) => {
            list.retain(|e| e["device_id"] != origin_id);
            list.push(entry);
        }
        None => metadata["also_on"] = json!([entry]),
    }
}

fn parse_metadata(raw: &str) -> Value {
    match serde_json::from_str::<Value>(raw) {
        Ok(value @ Value::Object(_)) => value,
        _ => json!({}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const OFFER: &str = "33333333-3333-4333-8333-333333333333";
    const LAPTOP: &str = "11111111-1111-4111-8111-111111111111";

    fn info() -> FileOfferInfo {
        FileOfferInfo {
            offer_id: OFFER.into(),
            name: "report.pdf".into(),
            size: 2048,
            mime: "application/pdf".into(),
            sha256: SHA.into(),
        }
    }

    fn repo() -> (ClipboardRepository, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let repo =
            ClipboardRepository::new(dir.path().join("clipboard.db").to_str().unwrap()).unwrap();
        (repo, dir)
    }

    fn store(repo: &ClipboardRepository, info: &FileOfferInfo) -> (ClipboardItem, OfferOutcome) {
        store_file_offer(repo, info, LAPTOP, "Laptop").unwrap()
    }

    fn metadata(item: &ClipboardItem) -> Value {
        serde_json::from_str(&item.content_metadata).unwrap()
    }

    // ---- storing offers ----

    #[test]
    fn new_offer_becomes_a_remote_file_item() {
        let (repo, _dir) = repo();
        let (item, outcome) = store(&repo, &info());

        assert_eq!(
            (
                item.content_type.as_str(),
                item.file_url.as_deref(),
                outcome
            ),
            ("file", None, OfferOutcome::New)
        );
    }

    #[test]
    fn remote_item_records_the_offer() {
        let (repo, _dir) = repo();
        let (item, _) = store(&repo, &info());

        assert_eq!(metadata(&item)["remote"]["offer_id"], OFFER);
    }

    #[test]
    fn remote_item_says_which_device_has_the_file() {
        let (repo, _dir) = repo();
        let (item, _) = store(&repo, &info());

        assert_eq!(item.source_app.as_deref(), Some("Shared from Laptop"));
    }

    #[test]
    fn remote_item_shows_name_size_and_type() {
        let (repo, _dir) = repo();
        let id = store(&repo, &info()).0.id;
        let saved = repo.get_item(&id).unwrap().unwrap();

        assert_eq!(
            (
                saved.file_name.as_deref(),
                saved.file_size_bytes,
                saved.file_mime_type.as_deref()
            ),
            (Some("report.pdf"), Some(2048), Some("application/pdf"))
        );
    }

    #[test]
    fn same_file_offered_again_refreshes_the_existing_item() {
        let (repo, _dir) = repo();
        let first = store(&repo, &info()).0;
        let (second, outcome) = store(&repo, &info());

        assert_eq!((second.id, outcome), (first.id, OfferOutcome::Refreshed));
    }

    #[test]
    fn same_file_offered_again_adds_no_second_item() {
        let (repo, _dir) = repo();
        store(&repo, &info());
        store(&repo, &info());

        assert_eq!(repo.count_items().unwrap(), 1);
    }

    #[test]
    fn a_newer_offer_replaces_the_older_one() {
        let (repo, _dir) = repo();
        store(&repo, &info());
        let newer = FileOfferInfo {
            offer_id: "44444444-4444-4444-8444-444444444444".into(),
            ..info()
        };
        let (item, _) = store(&repo, &newer);

        assert_eq!(metadata(&item)["remote"]["offer_id"], newer.offer_id);
    }

    #[test]
    fn file_already_here_is_not_added_again() {
        let (repo, _dir) = repo();
        let local = repo
            .create_item(CreateClipboardItemDto {
                content_type: "image".into(),
                content_text: "Image 4x3".into(),
                content_metadata: Some(r#"{"source":"clipboard"}"#.into()),
                source_app: None,
                code_language: None,
            })
            .unwrap();
        repo.update_file_info(
            &local.id,
            "/tmp/x.png",
            "x.png",
            2048,
            "image/png",
            Some(SHA),
        )
        .unwrap();
        let (item, outcome) = store(&repo, &info());

        assert_eq!((item.id, outcome), (local.id, OfferOutcome::AlreadyHere));
    }

    #[test]
    fn file_already_here_notes_the_other_device_once() {
        let (repo, _dir) = repo();
        let local = repo
            .create_item(CreateClipboardItemDto {
                content_type: "file".into(),
                content_text: "report.pdf".into(),
                content_metadata: None,
                source_app: None,
                code_language: None,
            })
            .unwrap();
        repo.update_file_info(
            &local.id,
            "/tmp/report.pdf",
            "report.pdf",
            2048,
            "application/pdf",
            Some(SHA),
        )
        .unwrap();
        store(&repo, &info());
        let (item, _) = store(&repo, &info());

        assert_eq!(
            metadata(&item)["also_on"],
            json!([{ "device_id": LAPTOP, "name": "Laptop" }])
        );
    }

    #[test]
    fn remote_item_survives_the_missing_file_cleanup() {
        // The cleanup runs whenever the window loses focus; a remote item has
        // no local file by design and must not be pruned.
        let (repo, _dir) = repo();
        store(&repo, &info());
        repo.cleanup_missing_file_records().unwrap();

        assert_eq!(repo.count_items().unwrap(), 1);
    }

    #[test]
    fn remote_item_describes_its_offer() {
        let (repo, _dir) = repo();
        let (item, _) = store(&repo, &info());

        assert_eq!(
            remote_file_of(&item),
            Some(RemoteFile {
                offer_id: OFFER.into(),
                origin_id: LAPTOP.into(),
                origin_name: "Laptop".into(),
                name: "report.pdf".into(),
                size: 2048,
                sha256: SHA.into(),
            })
        );
    }

    #[test]
    fn local_item_has_no_offer() {
        let (repo, _dir) = repo();
        let local = repo
            .create_item(CreateClipboardItemDto {
                content_type: "file".into(),
                content_text: "report.pdf".into(),
                content_metadata: Some(r#"{"source":"file"}"#.into()),
                source_app: None,
                code_language: None,
            })
            .unwrap();

        assert_eq!(remote_file_of(&local), None);
    }
}
