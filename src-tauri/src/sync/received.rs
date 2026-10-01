//! Storing images received from paired devices in the local history, the
//! same way the clipboard monitor stores an image copied here.
//!
//! The original PNG bytes are stored as-is rather than re-encoded, so the
//! stored hash equals the sender's and the same image arriving again (or
//! already here) is recognized as a duplicate.

use std::{
    fs,
    path::{Path, PathBuf},
};

use uuid::Uuid;

use super::image::ImageMeta;
use crate::{
    clipboard::image_handler::{detect_mime_type, generate_image_thumbnail},
    db::{
        models::{ClipboardItem, CreateClipboardItemDto},
        repository::ClipboardRepository,
    },
};

pub struct StoredImage {
    pub item: ClipboardItem,
    /// Where the PNG is on disk, to put it on the clipboard.
    pub path: PathBuf,
}

/// Store a received PNG that already passed `open_image` and `decode_png`.
pub fn store_received_image(
    repo: &ClipboardRepository,
    images_dir: &Path,
    png: &[u8],
    meta: &ImageMeta,
    origin_name: &str,
) -> Result<StoredImage, String> {
    let existing = repo
        .find_by_file_hash(&meta.sha256)
        .map_err(|e| format!("Failed to look up image by hash: {e}"))?;
    if let Some(existing) = existing {
        return bump_existing(repo, &existing, png);
    }

    fs::create_dir_all(images_dir).map_err(|e| format!("Failed to create images dir: {e}"))?;
    let id = Uuid::new_v4().to_string();
    let file_name = format!("{id}.png");
    let full_path = images_dir.join(&file_name);
    write_atomically(&full_path, png)?;
    // Same `<id>_thumb.png` naming the monitor uses. Safe to decode without
    // limits: these bytes already passed the limited decode.
    let thumb_path = generate_image_thumbnail(&full_path, images_dir, &id);

    let created = create_item(
        repo,
        meta,
        &full_path,
        &file_name,
        thumb_path.as_deref(),
        origin_name,
    );
    match created {
        Ok(item) => Ok(StoredImage {
            item,
            path: full_path,
        }),
        Err(e) => {
            // Don't leave files behind that no history item points to.
            let _ = fs::remove_file(&full_path);
            if let Some(thumb) = thumb_path {
                let _ = fs::remove_file(thumb);
            }
            Err(e)
        }
    }
}

fn bump_existing(
    repo: &ClipboardRepository,
    existing: &ClipboardItem,
    png: &[u8],
) -> Result<StoredImage, String> {
    let path = existing
        .file_url
        .as_deref()
        .map(PathBuf::from)
        .ok_or("Existing image item has no file")?;
    // Its file may have been deleted from disk; the bytes just arrived, so
    // restore it rather than keep a history item with a broken image.
    if !path.exists() {
        write_atomically(&path, png)?;
    }
    let item = repo
        .bump_item(&existing.id)
        .map_err(|e| format!("Failed to bump existing image: {e}"))?;
    Ok(StoredImage { item, path })
}

fn create_item(
    repo: &ClipboardRepository,
    meta: &ImageMeta,
    full_path: &Path,
    file_name: &str,
    thumb_path: Option<&Path>,
    origin_name: &str,
) -> Result<ClipboardItem, String> {
    let full_path_str = full_path.to_string_lossy().to_string();
    let mut metadata = serde_json::json!({
        "width": meta.width,
        "height": meta.height,
        "original_path": full_path_str,
        "is_screenshot": meta.is_screenshot,
        // "clipboard", like a local copy, so the viewer treats it the same.
        "source": "clipboard",
        "preview_type": "image",
        "synced_from": origin_name,
    });
    if let Some(thumb) = thumb_path {
        metadata["thumbnail_path"] = thumb.to_string_lossy().to_string().into();
    }

    let dto = CreateClipboardItemDto {
        content_type: "image".to_string(),
        content_text: format!("Image {}x{}", meta.width, meta.height),
        content_metadata: Some(metadata.to_string()),
        source_app: Some(format!("Synced from {origin_name}")),
        code_language: None,
    };
    let mut item = repo
        .create_item(dto)
        .map_err(|e| format!("Failed to save received image: {e}"))?;

    let mime_type = detect_mime_type(file_name);
    repo.update_file_info(
        &item.id,
        &full_path_str,
        file_name,
        meta.byte_len as i64,
        &mime_type,
        Some(&meta.sha256),
    )
    .map_err(|e| format!("Failed to record received image file: {e}"))?;

    item.file_url = Some(full_path_str);
    item.file_name = Some(file_name.to_string());
    item.file_size_bytes = Some(meta.byte_len as i64);
    item.file_mime_type = Some(mime_type);
    item.file_hash = Some(meta.sha256.clone());
    Ok(item)
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("png.tmp");
    fs::write(&tmp, bytes).map_err(|e| format!("Failed to write received image: {e}"))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("Failed to move received image into place: {e}")
    })
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use sha2::{Digest, Sha256};

    use super::*;

    struct Fixture {
        repo: ClipboardRepository,
        images_dir: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let repo =
            ClipboardRepository::new(dir.path().join("clipboard.db").to_str().unwrap()).unwrap();
        Fixture {
            repo,
            images_dir: dir.path().join("images"),
            _dir: dir,
        }
    }

    fn png() -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(4, 3, image::Rgba([200, 30, 90, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::from(image)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn meta(png: &[u8]) -> ImageMeta {
        ImageMeta {
            width: 4,
            height: 3,
            byte_len: png.len() as u64,
            sha256: format!("{:x}", Sha256::digest(png)),
            is_screenshot: false,
        }
    }

    fn store(fx: &Fixture) -> StoredImage {
        let png = png();
        store_received_image(&fx.repo, &fx.images_dir, &png, &meta(&png), "Laptop").unwrap()
    }

    #[test]
    fn received_image_becomes_an_image_item() {
        let fx = fixture();
        assert_eq!(store(&fx).item.content_type, "image");
    }

    #[test]
    fn stored_file_holds_the_exact_bytes_received() {
        let fx = fixture();
        assert_eq!(fs::read(store(&fx).path).unwrap(), png());
    }

    #[test]
    fn item_keeps_the_senders_hash() {
        let fx = fixture();
        assert_eq!(store(&fx).item.file_hash, Some(meta(&png()).sha256));
    }

    #[test]
    fn item_says_which_device_it_came_from() {
        let fx = fixture();
        assert_eq!(
            store(&fx).item.source_app.as_deref(),
            Some("Synced from Laptop")
        );
    }

    #[test]
    fn a_thumbnail_is_generated() {
        let fx = fixture();
        let item = store(&fx).item;
        let metadata: serde_json::Value = serde_json::from_str(&item.content_metadata).unwrap();

        assert!(Path::new(metadata["thumbnail_path"].as_str().unwrap()).exists());
    }

    #[test]
    fn item_is_findable_in_history() {
        let fx = fixture();
        let stored = store(&fx);
        let found = fx
            .repo
            .find_by_file_hash(&meta(&png()).sha256)
            .unwrap()
            .unwrap();

        assert_eq!(found.id, stored.item.id);
    }

    #[test]
    fn same_image_again_bumps_the_existing_item() {
        let fx = fixture();
        let first = store(&fx);
        let second = store(&fx);

        assert_eq!(second.item.id, first.item.id);
    }

    #[test]
    fn same_image_again_does_not_add_a_second_item() {
        let fx = fixture();
        store(&fx);
        store(&fx);

        assert_eq!(fx.repo.count_items().unwrap(), 1);
    }

    #[test]
    fn duplicate_whose_file_was_deleted_gets_it_back() {
        let fx = fixture();
        let first = store(&fx);
        fs::remove_file(&first.path).unwrap();
        store(&fx);

        assert_eq!(fs::read(&first.path).unwrap(), png());
    }

    #[test]
    fn no_temporary_file_is_left_behind() {
        let fx = fixture();
        store(&fx);
        let leftovers = fs::read_dir(&fx.images_dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();

        assert_eq!(leftovers, 0);
    }
}
