use crate::clipboard::document_thumbnail::generate_document_thumbnail;
use crate::clipboard::file_handler::{prepare_file_metadata, store_prepared_file};
use crate::clipboard::image_handler::{
    copy_image_file_to_storage, detect_mime_type, generate_image_thumbnail, save_image_to_disk,
};
use crate::clipboard::listener::ClipboardEvent;
use crate::clipboard::operations::{read_clipboard_content, ClipboardContent};
use crate::clipboard::state;
use crate::clipboard::types::{detect_code_language, detect_content_type, get_source_app};
use crate::db::models::CreateClipboardItemDto;
use crate::db::repository::ClipboardRepository;
use crate::clipboard::image_handler::StoredImageInfo;
use crate::sync::{
    image::{encode_png, pixel_hash, ImageDetails},
    runtime::SyncRuntime,
    service::SyncService,
    ClipPayload,
};
use std::{
    fs,
    sync::Arc,
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::sync::Mutex;

pub struct ClipboardMonitor {
    event_rx: Mutex<Option<UnboundedReceiver<ClipboardEvent>>>,
    last_text_content: Arc<Mutex<String>>,
    last_image_hash: Arc<Mutex<Option<Vec<u8>>>>,
    last_file_path: Arc<Mutex<Option<(std::path::PathBuf, Instant)>>>,
    repo_path: String,
    images_dir: std::path::PathBuf,
    is_running: Arc<Mutex<bool>>,
    app_handle: tauri::AppHandle,
    files_dir: std::path::PathBuf,
    file_thumbs_dir: std::path::PathBuf,
}

impl ClipboardMonitor {
    pub fn new(
        repo_path: String,
        app_handle: tauri::AppHandle,
        event_rx: UnboundedReceiver<ClipboardEvent>,
    ) -> Self {
        let app_data_dir = app_handle
            .path()
            .app_data_dir()
            .expect("Failed to get app data dir");
        let images_dir = app_data_dir.join("images");
        let files_dir = app_data_dir.join("files");
        let file_thumbs_dir = app_data_dir.join("file_thumbnails");

        std::fs::create_dir_all(&images_dir).expect("Failed to create images directory");
        std::fs::create_dir_all(&files_dir).expect("Failed to create files directory");
        std::fs::create_dir_all(&file_thumbs_dir)
            .expect("Failed to create file thumbnails directory");

        Self {
            event_rx: Mutex::new(Some(event_rx)),
            last_text_content: Arc::new(Mutex::new(String::new())),
            last_image_hash: Arc::new(Mutex::new(None)),
            last_file_path: Arc::new(Mutex::new(None)),
            repo_path,
            images_dir,
            is_running: Arc::new(Mutex::new(false)),
            app_handle,
            files_dir,
            file_thumbs_dir,
        }
    }

    pub async fn start(&self) {
        {
            let mut running = self.is_running.lock().await;
            if *running {
                println!("Clipboard monitor already running");
                return;
            }
            *running = true;
        }

        let mut event_rx = self
            .event_rx
            .lock()
            .await
            .take()
            .expect("ClipboardMonitor::start called more than once");

        println!("Clipboard monitor started");

        while let Some(event) = event_rx.recv().await {
            match event {
                ClipboardEvent::Changed => self.handle_clipboard_change().await,
                ClipboardEvent::Error(err) => eprintln!("Clipboard listener error: {err}"),
            }
        }

        {
            let mut running = self.is_running.lock().await;
            *running = false;
        }

        println!("Clipboard monitor stopped");
    }

    async fn handle_clipboard_change(&self) {
        if state::take_skip_event() {
            return;
        }

        match read_clipboard_content() {
            Ok(ClipboardContent::ImageFile(file_path)) => {
                {
                    let mut last_path = self.last_file_path.lock().await;
                    if let Some((prev, ts)) = last_path.as_ref() {
                        if prev == &file_path && ts.elapsed() < Duration::from_millis(750) {
                            return;
                        }
                    }
                    *last_path = Some((file_path.clone(), Instant::now()));
                }

                println!(
                    "📁 Image file detected from Finder: {}",
                    file_path.display()
                );

                match copy_image_file_to_storage(&file_path, &self.images_dir) {
                    Ok(info) => {
                        if let Ok(repo) = ClipboardRepository::new(&self.repo_path) {
                            if let Ok(Some(existing_item)) = repo.find_by_file_hash(&info.file_hash)
                            {
                                println!(
                                    "🔁 Duplicate image detected (hash: {}), bumping existing item: {}",
                                    &info.file_hash[..12],
                                    existing_item.id
                                );

                                if let Ok(bumped_item) = repo.bump_item(&existing_item.id) {
                                    if let Err(e) =
                                        self.app_handle.emit("clipboard-item-added", &bumped_item)
                                    {
                                        eprintln!("Failed to emit clipboard-item-added event: {e}");
                                    }
                                }

                                // NOTE: never delete `info.full_path` here. In pointer mode
                                // (`copy_image_file_to_storage`) it IS the user's original
                                // file, not a copy of ours; nothing was created to clean up.
                                return;
                            }
                        }

                        let quicklook_thumbnail = match generate_document_thumbnail(
                            &info.full_path,
                            &self.file_thumbs_dir,
                        ) {
                            Ok(Some(path)) if path.exists() => Some(path),
                            Ok(Some(_)) => None, // generated path missing; fall back to image thumb
                            Ok(_none) => None,
                            Err(err) => {
                                eprintln!(
                                    "Failed to generate Quick Look thumbnail for Finder image: {err}"
                                );
                                None
                            }
                        };
                        let fallback_thumbnail = if info.thumb_path.exists() {
                            Some(info.thumb_path.clone())
                        } else {
                            None
                        };
                        // Bundled-crate thumbnail (works without system thumbnailers).
                        let image_thumbnail = generate_image_thumbnail(
                            &info.full_path,
                            &self.file_thumbs_dir,
                            &info.file_hash,
                        );
                        let chosen_thumbnail =
                            image_thumbnail.or(quicklook_thumbnail).or(fallback_thumbnail);

                        let mut metadata = serde_json::json!({
                            "width": info.width,
                            "height": info.height,
                            "original_path": info.full_path.to_string_lossy().to_string(),
                            "is_screenshot": info.is_screenshot,
                            "source": "file",
                        });

                        if let Some(obj) = metadata.as_object_mut() {
                            if let Some(path) = chosen_thumbnail {
                                obj.insert(
                                    "preview_type".to_string(),
                                    serde_json::Value::String("image".to_string()),
                                );
                                obj.insert(
                                    "thumbnail_path".to_string(),
                                    serde_json::Value::String(path.to_string_lossy().to_string()),
                                );
                            }

                            if let Some(ext) = info.original_extension.as_ref() {
                                obj.insert(
                                    "original_extension".to_string(),
                                    serde_json::Value::String(ext.clone()),
                                );
                            }

                            if let Some(name) = info.original_name.as_ref() {
                                obj.insert(
                                    "original_name".to_string(),
                                    serde_json::Value::String(name.clone()),
                                );
                            }
                        }

                        if let Ok(repo) = ClipboardRepository::new(&self.repo_path) {
                            let dto = CreateClipboardItemDto {
                                content_type: "image".to_string(),
                                content_text: format!("Image {}x{}", info.width, info.height),
                                content_metadata: Some(metadata.to_string()),
                                source_app: get_source_app(),
                                code_language: None,
                            };

                            match repo.create_item(dto) {
                                Ok(mut item) => {
                                    let mime_type = detect_mime_type(&info.file_name);

                                    item.file_url =
                                        Some(info.full_path.to_string_lossy().to_string());
                                    item.file_name = Some(info.file_name.clone());
                                    item.file_size_bytes = Some(info.file_size as i64);
                                    item.file_mime_type = Some(mime_type.clone());
                                    item.file_hash = Some(info.file_hash.clone());

                                    if let Err(e) = repo.update_file_info(
                                        &item.id,
                                        &info.full_path.to_string_lossy(),
                                        &info.file_name,
                                        info.file_size as i64,
                                        &mime_type,
                                        Some(&info.file_hash),
                                    ) {
                                        eprintln!("Failed to update file info: {}", e);
                                    }

                                    println!("✅ Saved image from Finder: {}", item.id);

                                    if let Err(e) =
                                        self.app_handle.emit("clipboard-item-added", &item)
                                    {
                                        eprintln!("Failed to emit clipboard-item-added event: {e}");
                                    }
                                }
                                Err(e) => eprintln!("Failed to save image from Finder: {e}"),
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to copy image file: {}", e);
                    }
                }
            }
            Ok(ClipboardContent::Text(current_text)) => {
                let mut last_text = self.last_text_content.lock().await;

                if !current_text.is_empty() && current_text != *last_text {
                    let preview: String = current_text.chars().take(50).collect();
                    println!("New clipboard text detected: {}", preview);

                    let content_type = detect_content_type(&current_text);
                    let source_app = get_source_app();
                    let code_language = if content_type == "code" {
                        detect_code_language(&current_text)
                    } else {
                        None
                    };

                    if let Ok(repo) = ClipboardRepository::new(&self.repo_path) {
                        let dto = CreateClipboardItemDto {
                            content_type,
                            content_text: current_text.clone(),
                            content_metadata: None,
                            source_app,
                            code_language,
                        };

                        match repo.upsert_item(dto) {
                            Ok(item) => {
                                println!("Upserted clipboard item: {}", item.id);

                                if let Err(e) = self.app_handle.emit("clipboard-item-added", &item)
                                {
                                    eprintln!("Failed to emit clipboard-item-added event: {e}");
                                }

                                // Only running while the user has sync turned on.
                                let sync = self
                                    .app_handle
                                    .try_state::<SyncRuntime>()
                                    .and_then(|runtime| runtime.service());
                                if let Some(sync) = sync {
                                    let clip = ClipPayload {
                                        text: current_text.clone(),
                                        content_type: item.content_type.clone(),
                                    };
                                    tauri::async_runtime::spawn(async move {
                                        sync.broadcast(clip).await;
                                    });
                                }
                            }
                            Err(e) => eprintln!("Failed to upsert clipboard item: {e}"),
                        }
                    }

                    *last_text = current_text;
                }
            }
            Ok(ClipboardContent::Image(image_data, screenshot_hint)) => {
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};

                let mut hasher = DefaultHasher::new();
                image_data.bytes.hash(&mut hasher);
                let current_hash = hasher.finish().to_le_bytes().to_vec();

                {
                    let mut last_hash = self.last_image_hash.lock().await;
                    if let Some(previous_hash) = last_hash.as_ref() {
                        if *previous_hash == current_hash {
                            return;
                        }
                    }
                    *last_hash = Some(current_hash.clone());
                }

                // An image a paired device just sent was already stored by the
                // receiver; our own clipboard write of it must not be stored or
                // sent again. Checked after `last_image_hash` is updated, so a
                // duplicate event for the same write is still filtered above.
                // None unless sync is on *and* the user lets images sync.
                let sync = self
                    .app_handle
                    .try_state::<SyncRuntime>()
                    .and_then(|runtime| runtime.service())
                    .filter(|service| service.is_syncing_images());
                if let Some(sync) = &sync {
                    let hash = pixel_hash(
                        image_data.width as u32,
                        image_data.height as u32,
                        &image_data.bytes,
                    );
                    if sync.take_image_echo(&hash) {
                        return;
                    }
                }

                // When the capture-folder watcher is active it stores screenshots
                // as pointers, so skip saving the raw bytes to avoid a duplicate.
                // Paired devices have no such file, so still send them the image.
                if state::should_suppress_screenshot_bytes() {
                    if let Some(sync) = sync {
                        sync_pixels(sync, image_data, screenshot_hint);
                    }
                    return;
                }

                match save_image_to_disk(&image_data, &self.images_dir, screenshot_hint) {
                    Ok(info) => {
                        if let Ok(repo) = ClipboardRepository::new(&self.repo_path) {
                            if let Ok(Some(existing_item)) = repo.find_by_file_hash(&info.file_hash)
                            {
                                println!(
                                    "🔁 Duplicate clipboard image detected (hash: {}), bumping existing item: {}",
                                    &info.file_hash[..12],
                                    existing_item.id
                                );

                                if let Ok(bumped_item) = repo.bump_item(&existing_item.id) {
                                    if let Err(e) =
                                        self.app_handle.emit("clipboard-item-added", &bumped_item)
                                    {
                                        eprintln!("Failed to emit clipboard-item-added event: {e}");
                                    }
                                }

                                // Copying it again is a copy: sync it, from the
                                // stored file (the fresh one is deleted below).
                                if let Some(stored) = existing_item.file_url.as_deref() {
                                    sync_image(sync.clone(), stored.into(), image_details(&info));
                                }

                                let _ = std::fs::remove_file(&info.full_path);
                                let _ = std::fs::remove_file(&info.thumb_path);
                                return;
                            }
                        }

                        let mut metadata = serde_json::json!({
                            "width": info.width,
                            "height": info.height,
                            "thumbnail_path": info.thumb_path.to_string_lossy().to_string(),
                            "original_path": info.full_path.to_string_lossy().to_string(),
                            "is_screenshot": info.is_screenshot,
                            "source": "clipboard",
                        });

                        if let Some(obj) = metadata.as_object_mut() {
                            obj.insert(
                                "preview_type".to_string(),
                                serde_json::Value::String("image".to_string()),
                            );

                            if let Some(ext) = info.original_extension.as_ref() {
                                obj.insert(
                                    "original_extension".to_string(),
                                    serde_json::Value::String(ext.clone()),
                                );
                            }
                        }

                        if let Ok(repo) = ClipboardRepository::new(&self.repo_path) {
                            let dto = CreateClipboardItemDto {
                                content_type: "image".to_string(),
                                content_text: format!("Image {}x{}", info.width, info.height),
                                content_metadata: Some(metadata.to_string()),
                                source_app: get_source_app(),
                                code_language: None,
                            };

                            match repo.create_item(dto) {
                                Ok(mut item) => {
                                    let mime_type = detect_mime_type(&info.file_name);

                                    item.file_url =
                                        Some(info.full_path.to_string_lossy().to_string());
                                    item.file_name = Some(info.file_name.clone());
                                    item.file_size_bytes = Some(info.file_size as i64);
                                    item.file_mime_type = Some(mime_type.clone());
                                    item.file_hash = Some(info.file_hash.clone());

                                    if let Err(e) = repo.update_file_info(
                                        &item.id,
                                        &info.full_path.to_string_lossy(),
                                        &info.file_name,
                                        info.file_size as i64,
                                        &mime_type,
                                        Some(&info.file_hash),
                                    ) {
                                        eprintln!("Failed to update file info: {}", e);
                                    }

                                    println!("Saved clipboard image: {}", item.id);
                                    sync_image(sync.clone(), info.full_path.clone(), image_details(&info));

                                    if let Err(e) =
                                        self.app_handle.emit("clipboard-item-added", &item)
                                    {
                                        eprintln!(
                                            "Failed to emit clipboard-item-added event: {}",
                                            e
                                        );
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Failed to save clipboard image: {}", e)
                                }
                            }
                        }
                    }
                    Err(e) => eprintln!("Failed to save image to disk: {}", e),
                }
            }
            Ok(ClipboardContent::Empty) => {
                // No-op
            }
            Ok(ClipboardContent::File(file_path)) => {
                {
                    let mut last_path = self.last_file_path.lock().await;
                    if let Some((prev, ts)) = last_path.as_ref() {
                        if prev == &file_path && ts.elapsed() < Duration::from_millis(750) {
                            return;
                        }
                    }
                    *last_path = Some((file_path.clone(), Instant::now()));
                }

                match prepare_file_metadata(&file_path) {
                    Ok(prepared) => {
                        if let Ok(repo) = ClipboardRepository::new(&self.repo_path) {
                            if let Ok(Some(existing_item)) = repo.find_by_file_hash(&prepared.hash)
                            {
                                println!(
                                    "🔁 Duplicate file detected (hash: {}), bumping existing item: {}",
                                    &prepared.hash[..12],
                                    existing_item.id
                                );
                                if let Ok(bumped_item) = repo.bump_item(&existing_item.id) {
                                    let _ =
                                        self.app_handle.emit("clipboard-item-added", &bumped_item);
                                }
                                return;
                            }

                            match store_prepared_file(&prepared, &file_path, &self.files_dir) {
                                Ok(info) => {
                                    let is_textual = is_textual_mime(
                                        &info.file_mime_type,
                                        info.original_extension.as_ref(),
                                    );
                                    let mut thumbnail_path: Option<String> = None;
                                    let mut text_preview: Option<String> = None;
                                    let mut preview_language: Option<String> = None;

                                    if is_textual {
                                        match extract_text_preview(&info.full_path, 32_768) {
                                            Ok(preview_text) => {
                                                preview_language =
                                                    detect_code_language(&preview_text);
                                                text_preview = Some(preview_text);
                                            }
                                            Err(err) => eprintln!(
                                                "Failed to read text preview for {}: {}",
                                                info.file_name, err
                                            ),
                                        }
                                    } else {
                                        match generate_document_thumbnail(
                                            &info.full_path,
                                            &self.file_thumbs_dir,
                                        ) {
                                            Ok(result) => {
                                                thumbnail_path = result
                                                    .map(|path| path.to_string_lossy().to_string());
                                            }
                                            Err(err) => {
                                                eprintln!(
                                                    "Failed to generate document thumbnail: {}",
                                                    err
                                                );
                                            }
                                        }
                                    }

                                    let mut metadata = serde_json::json!({
                                        "source": "file",
                                        "original_extension": info.original_extension,
                                        "original_name": info.original_name.clone(),
                                        "external_path": info.original_path.as_ref()
                                            .unwrap_or(&info.full_path)
                                            .to_string_lossy(),
                                        "external_missing": false,
                                    });

                                    if let Some(obj) = metadata.as_object_mut() {
                                        if let Some(path) = thumbnail_path {
                                            obj.insert(
                                                "preview_type".to_string(),
                                                serde_json::Value::String("image".to_string()),
                                            );
                                            obj.insert(
                                                "thumbnail_path".to_string(),
                                                serde_json::Value::String(path),
                                            );
                                        }

                                        if let Some(preview) = text_preview {
                                            obj.insert(
                                                "preview_type".to_string(),
                                                serde_json::Value::String("text".to_string()),
                                            );
                                            obj.insert(
                                                "text_preview".to_string(),
                                                serde_json::Value::String(preview),
                                            );

                                            if let Some(lang) = preview_language {
                                                obj.insert(
                                                    "preview_language".to_string(),
                                                    serde_json::Value::String(lang),
                                                );
                                            }
                                        }
                                    }

                                    let dto = CreateClipboardItemDto {
                                        content_type: "file".to_string(),
                                        content_text: info.file_name.clone(),
                                        content_metadata: Some(metadata.to_string()),
                                        source_app: get_source_app(),
                                        code_language: None,
                                    };

                                    match repo.create_item(dto) {
                                        Ok(mut item) => {
                                            item.file_url =
                                                Some(info.full_path.to_string_lossy().to_string());
                                            item.file_name = Some(info.file_name.clone());
                                            item.file_size_bytes = Some(info.file_size as i64);
                                            item.file_mime_type = Some(info.file_mime_type.clone());
                                            item.file_hash = Some(info.file_hash.clone());

                                            if let Err(e) = repo.update_file_info(
                                                &item.id,
                                                &info.full_path.to_string_lossy(),
                                                &info.file_name,
                                                info.file_size as i64,
                                                &info.file_mime_type,
                                                Some(&info.file_hash),
                                            ) {
                                                eprintln!("Failed to update file info: {}", e);
                                            }

                                            let _ =
                                                self.app_handle.emit("clipboard-item-added", &item);
                                        }
                                        Err(e) => eprintln!("Failed to save file item: {}", e),
                                    }
                                }
                                Err(e) => eprintln!("Failed to store file: {}", e),
                            }
                        }
                    }
                    Err(e) => eprintln!("Failed to read file metadata: {}", e),
                }
            }
            Err(e) => {
                eprintln!("Failed to read clipboard: {}", e);
            }
        }
    }
}

fn is_textual_mime(mime: &str, extension: Option<&String>) -> bool {
    if mime.starts_with("text/") {
        return true;
    }

    let normalized_mime = mime.to_lowercase();
    let ext_match = extension
        .map(|ext| ext.as_str())
        .unwrap_or_default()
        .to_lowercase();

    matches!(
        normalized_mime.as_str(),
        "application/json"
            | "application/xml"
            | "application/javascript"
            | "application/x-javascript"
            | "application/x-sh"
            | "application/x-shellscript"
            | "application/x-yaml"
            | "application/x-toml"
            | "application/x-ruby"
            | "application/x-python"
            | "application/x-httpd-php"
    ) || matches!(
        ext_match.as_str(),
        "md" | "markdown"
            | "txt"
            | "csv"
            | "ts"
            | "tsx"
            | "js"
            | "jsx"
            | "rs"
            | "go"
            | "py"
            | "rb"
            | "java"
            | "kt"
            | "swift"
            | "c"
            | "h"
            | "hpp"
            | "cpp"
            | "cs"
            | "toml"
            | "yaml"
            | "yml"
            | "json"
            | "log"
            | "sh"
            | "zsh"
            | "bash"
    )
}

fn extract_text_preview(path: &std::path::Path, max_bytes: usize) -> Result<String, String> {
    let data = fs::read(path).map_err(|e| format!("Failed to read file: {}", e))?;
    let take = std::cmp::min(max_bytes, data.len());
    let mut text = String::from_utf8_lossy(&data[..take]).to_string();
    text = text.trim_start_matches('\u{feff}').to_string(); // drop BOM if present
    if data.len() > max_bytes {
        text.push_str("\n…");
    }
    Ok(text)
}

fn image_details(info: &StoredImageInfo) -> ImageDetails {
    ImageDetails {
        width: info.width,
        height: info.height,
        is_screenshot: info.is_screenshot,
    }
}

/// Send a stored image to paired devices in the background. `sync` is None
/// while sync is off.
fn sync_image(sync: Option<Arc<SyncService>>, png_path: std::path::PathBuf, details: ImageDetails) {
    let Some(sync) = sync else {
        return;
    };
    tauri::async_runtime::spawn(async move {
        match tokio::fs::read(&png_path).await {
            Ok(png) => {
                sync.broadcast_image(&png, details).await;
            }
            Err(e) => eprintln!("Could not read image to sync {}: {e}", png_path.display()),
        }
    });
}

/// Send an image that isn't saved locally (the folder watcher stores it as a
/// file instead): encode its pixels to PNG in memory, then broadcast.
fn sync_pixels(sync: Arc<SyncService>, image: arboard::ImageData<'static>, is_screenshot: bool) {
    let details = ImageDetails {
        width: image.width as u32,
        height: image.height as u32,
        is_screenshot,
    };
    tauri::async_runtime::spawn(async move {
        // PNG encoding takes real CPU for big screenshots.
        let encoded = tauri::async_runtime::spawn_blocking(move || {
            encode_png(details.width, details.height, &image.bytes)
        })
        .await;
        match encoded {
            Ok(Ok(png)) => {
                sync.broadcast_image(&png, details).await;
            }
            Ok(Err(e)) => eprintln!("Could not encode image to sync: {e}"),
            Err(e) => eprintln!("Image encoding for sync failed to run: {e}"),
        }
    });
}
