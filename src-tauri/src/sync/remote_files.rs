//! Files that paired devices offered to this one: checking an offer, and
//! keeping it in history as a "remote" item until it's fetched.
//!
//! A remote item is a `file` item with no `file_url` (its bytes are still on
//! the other device) and `content_metadata.remote` describing the offer. Its
//! `file_hash` is the offer's full SHA-256, the same hash fetched files and
//! clipboard images store, so the same file arriving again is recognized.

use serde_json::{json, Value};

use super::offers::FileOfferInfo;
use crate::db::{
    models::{ClipboardItem, CreateClipboardItemDto},
    repository::ClipboardRepository,
};

const MAX_NAME_BYTES: usize = 200;
const MAX_MIME_CHARS: usize = 100;
const FALLBACK_NAME: &str = "file";
const FALLBACK_MIME: &str = "application/octet-stream";
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Check an offer from the network and normalize what's safe to keep.
pub fn validate_offer(info: FileOfferInfo) -> Result<FileOfferInfo, String> {
    uuid::Uuid::parse_str(&info.offer_id).map_err(|_| "offer id isn't a UUID".to_string())?;
    let is_sha256 = info.sha256.len() == 64
        && info
            .sha256
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !is_sha256 {
        return Err("offer hash isn't a SHA-256".into());
    }

    let is_plain_mime = !info.mime.is_empty()
        && info.mime.len() <= MAX_MIME_CHARS
        && info.mime.chars().all(|c| c.is_ascii_graphic());
    Ok(FileOfferInfo {
        name: clean_file_name(&info.name),
        mime: if is_plain_mime {
            info.mime
        } else {
            FALLBACK_MIME.into()
        },
        ..info
    })
}

/// A file name from another device, made safe to show and to save as: only
/// its last path part, nothing that can climb out of a folder or that
/// Windows refuses, and not too long.
pub fn clean_file_name(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control())
        .map(|c| if "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    // Windows drops trailing dots and spaces, which can turn a name into
    // something else entirely.
    let cleaned = cleaned.trim().trim_end_matches(['.', ' ']);
    if cleaned.is_empty() || cleaned == "." || cleaned == ".." {
        return FALLBACK_NAME.into();
    }

    let stem = cleaned.split('.').next().unwrap_or("");
    let name = if WINDOWS_RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        format!("_{cleaned}")
    } else {
        cleaned.to_string()
    };
    truncate_keeping_extension(&name)
}

fn truncate_keeping_extension(name: &str) -> String {
    if name.len() <= MAX_NAME_BYTES {
        return name.to_string();
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if name.len() - dot <= 16 => name.split_at(dot),
        _ => (name, ""),
    };
    let mut budget = MAX_NAME_BYTES - extension.len();
    while !stem.is_char_boundary(budget) {
        budget -= 1;
    }
    format!("{}{extension}", &stem[..budget])
}

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

    // ---- file names ----

    #[test]
    fn plain_name_is_kept() {
        assert_eq!(clean_file_name("report.pdf"), "report.pdf");
    }

    #[test]
    fn unicode_name_is_kept() {
        assert_eq!(clean_file_name("café menú.pdf"), "café menú.pdf");
    }

    #[test]
    fn path_climbing_out_keeps_only_the_last_part() {
        assert_eq!(clean_file_name("../../.bashrc"), ".bashrc");
    }

    #[test]
    fn windows_path_keeps_only_the_last_part() {
        assert_eq!(clean_file_name("C:\\Users\\me\\evil.exe"), "evil.exe");
    }

    #[test]
    fn dot_names_fall_back_to_a_safe_name() {
        assert_eq!(
            (clean_file_name(".."), clean_file_name(".")),
            ("file".into(), "file".into())
        );
    }

    #[test]
    fn empty_name_falls_back_to_a_safe_name() {
        assert_eq!(clean_file_name(""), "file");
    }

    #[test]
    fn control_characters_are_removed() {
        assert_eq!(clean_file_name("bad\u{0}na\nme.txt"), "badname.txt");
    }

    #[test]
    fn characters_windows_forbids_are_replaced() {
        assert_eq!(clean_file_name("a<b>c:d\"e|f?g*.txt"), "a_b_c_d_e_f_g_.txt");
    }

    #[test]
    fn trailing_dots_and_spaces_are_trimmed() {
        assert_eq!(clean_file_name("report.pdf. . "), "report.pdf");
    }

    #[test]
    fn windows_reserved_names_are_prefixed() {
        assert_eq!(
            (clean_file_name("CON.txt"), clean_file_name("nul")),
            ("_CON.txt".into(), "_nul".into())
        );
    }

    #[test]
    fn long_names_are_cut_but_keep_their_extension() {
        let cleaned = clean_file_name(&format!("{}.pdf", "a".repeat(500)));
        assert!(cleaned.len() <= MAX_NAME_BYTES && cleaned.ends_with(".pdf"));
    }

    #[test]
    fn long_unicode_names_are_cut_on_a_character_boundary() {
        // Would panic if cut mid-character.
        let cleaned = clean_file_name(&format!("{}.pdf", "é".repeat(300)));
        assert!(cleaned.ends_with(".pdf"));
    }

    // ---- validating offers ----

    #[test]
    fn valid_offer_passes() {
        assert_eq!(validate_offer(info()), Ok(info()));
    }

    #[test]
    fn offer_name_is_cleaned() {
        let offer = FileOfferInfo {
            name: "../../etc/passwd".into(),
            ..info()
        };
        assert_eq!(validate_offer(offer).unwrap().name, "passwd");
    }

    #[test]
    fn offer_with_a_non_uuid_id_is_rejected() {
        let offer = FileOfferInfo {
            offer_id: "../../x".into(),
            ..info()
        };
        assert!(validate_offer(offer).is_err());
    }

    #[test]
    fn offer_with_a_bad_hash_is_rejected() {
        let offer = FileOfferInfo {
            sha256: "not-a-hash".into(),
            ..info()
        };
        assert!(validate_offer(offer).is_err());
    }

    #[test]
    fn offer_with_an_uppercase_hash_is_rejected() {
        let offer = FileOfferInfo {
            sha256: SHA.to_uppercase(),
            ..info()
        };
        assert!(validate_offer(offer).is_err());
    }

    #[test]
    fn odd_mime_type_falls_back_to_binary() {
        let offer = FileOfferInfo {
            mime: "text/html\n<script>".into(),
            ..info()
        };
        assert_eq!(validate_offer(offer).unwrap().mime, FALLBACK_MIME);
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
}
