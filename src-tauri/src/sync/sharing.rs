//! Sending and fetching files from the app: the glue between the UI
//! commands, history, and the sync service. Progress and results reach the
//! UI as `sync-file-progress` and `sync-file-done` events.

use std::{
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

use super::{
    fetching::RemoteFile,
    offers::{create_offer, hash_file, FileOfferInfo},
    remote_files::remote_file_of,
    runtime::{SyncRuntime, DOWNLOAD_DIR_SETTING_KEY},
};
use crate::{
    clipboard::operations::write_file_list,
    commands::AppState,
    db::{models::ClipboardItem, repository::ClipboardRepository},
};

/// At most this often per transfer, so a fast fetch doesn't flood the UI.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const DOWNLOAD_SUBFOLDER: &str = "Quakboard";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileProgress<'a> {
    item_id: &'a str,
    /// "hashing" (preparing to send) or "fetching".
    phase: &'a str,
    done: u64,
    total: u64,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileDone<'a> {
    item_id: &'a str,
    ok: bool,
    cancelled: bool,
    error: Option<String>,
}

/// Where fetched files go: the user's chosen folder, or `Downloads/Quakboard`.
pub fn download_dir(app: &AppHandle, runtime: &SyncRuntime) -> Result<PathBuf, String> {
    if let Some(chosen) = runtime.setting_text(DOWNLOAD_DIR_SETTING_KEY) {
        return Ok(PathBuf::from(chosen));
    }
    app.path()
        .download_dir()
        .map(|downloads| downloads.join(DOWNLOAD_SUBFOLDER))
        .map_err(|e| format!("Could not find the Downloads folder: {e}"))
}

/// Offer a history item's file to `device_ids`. Returns how many were reached.
pub async fn send_file(
    app: &AppHandle,
    runtime: &SyncRuntime,
    item_id: &str,
    device_ids: &[String],
) -> Result<usize, String> {
    if device_ids.is_empty() {
        return Err("Choose at least one device".into());
    }
    let service = runtime.service().ok_or("Sync is off")?;
    let item = load_item(app, item_id)?;
    if remote_file_of(&item).is_some() {
        return Err("This file is on another device; fetch it first".into());
    }
    let path = item
        .file_url
        .as_deref()
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .ok_or("The file isn't on this device anymore")?;

    // Hashing a big file takes a while: off the async workers, with progress.
    let hashed = {
        let (app, item_id, path) = (app.clone(), item_id.to_string(), path.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let mut progress = Throttle::new();
            hash_file(&path, |done, total| {
                progress.emit(&app, &item_id, "hashing", done, total);
            })
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not read the file: {e}"))?
    };

    let offer = with_repo(app, |repo| {
        create_offer(&repo.conn, item_id, &path, &hashed, device_ids)
            .map_err(|e| format!("Could not record the offer: {e}"))
    })?;
    let info = FileOfferInfo {
        offer_id: offer.offer_id,
        name: item
            .file_name
            .clone()
            .or_else(|| file_name_of(&path))
            .unwrap_or_else(|| "file".into()),
        size: hashed.stamp.size,
        mime: item
            .file_mime_type
            .clone()
            .unwrap_or_else(|| mime_guess::from_path(&path).first_or_octet_stream().to_string()),
        sha256: hashed.sha256,
    };
    Ok(service.send_offer(&info, device_ids).await)
}

/// Start fetching a remote item's file. Returns right away; the outcome
/// arrives as a `sync-file-done` event.
pub fn start_fetch(app: &AppHandle, runtime: &SyncRuntime, item_id: &str) -> Result<(), String> {
    let service = runtime.service().ok_or("Sync is off")?;
    let item = load_item(app, item_id)?;
    let remote = remote_file_of(&item).ok_or("This file is already on this device")?;
    let dest = download_dir(app, runtime)?;

    let mut fetches = lock(&runtime.fetches);
    if fetches.get(item_id).is_some_and(|task| !task.inner().is_finished()) {
        return Err("Already fetching this file".into());
    }

    let (task_app, task_item, slots) = (app.clone(), item_id.to_string(), runtime.fetch_slots.clone());
    let task = tauri::async_runtime::spawn(async move {
        // Waits here if the maximum number of fetches is already running.
        let Ok(_slot) = slots.acquire_owned().await else {
            return;
        };
        let outcome = match tokio::fs::create_dir_all(&dest).await {
            Err(e) => Err(format!("Could not create {}: {e}", dest.display())),
            Ok(()) => {
                let mut progress = Throttle::new();
                service
                    .fetch_file(&remote, &dest, |done| {
                        progress.emit(&task_app, &task_item, "fetching", done, remote.size);
                    })
                    .await
                    .map_err(|e| e.to_string())
            }
        };

        let outcome = match outcome {
            Ok(path) => {
                let (app, item_id, remote) = (task_app.clone(), task_item.clone(), remote.clone());
                tauri::async_runtime::spawn_blocking(move || finish_fetch(&app, &item_id, &remote, &path))
                    .await
                    .map_err(|e| e.to_string())
                    .and_then(|done| done)
            }
            Err(e) => Err(e),
        };
        emit_done(&task_app, &task_item, outcome.err(), false);
        lock(&task_app.state::<SyncRuntime>().fetches).remove(&task_item);
    });
    fetches.insert(item_id.to_string(), task);
    Ok(())
}

/// Stop a fetch. Its partial download deletes itself (see `fetching`).
pub fn cancel_fetch(app: &AppHandle, runtime: &SyncRuntime, item_id: &str) {
    if let Some(task) = lock(&runtime.fetches).remove(item_id) {
        task.abort();
        emit_done(app, item_id, None, true);
    }
}

/// Turn the remote item into a normal local file item and put the file on
/// the clipboard, as copying it on the other device would have.
fn finish_fetch(app: &AppHandle, item_id: &str, remote: &RemoteFile, path: &Path) -> Result<(), String> {
    let item = with_repo(app, |repo| {
        let item = repo
            .get_item(item_id)
            .map_err(|e| e.to_string())?
            .ok_or("The item was deleted during the fetch")?;
        let mime = item
            .file_mime_type
            .clone()
            .unwrap_or_else(|| mime_guess::from_path(path).first_or_octet_stream().to_string());
        let name = file_name_of(path).unwrap_or_else(|| remote.name.clone());
        let path_text = path.to_string_lossy().to_string();
        // Keeps the full SHA-256: the same file offered again is recognized.
        repo.update_file_info(item_id, &path_text, &name, remote.size as i64, &mime, Some(&remote.sha256))
            .map_err(|e| e.to_string())?;
        repo.update_metadata(item_id, &fetched_metadata(&item, remote, &path_text).to_string())
            .map_err(|e| e.to_string())?;
        repo.get_item(item_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "The item was deleted during the fetch".to_string())
    })?;

    if let Err(e) = app.emit("clipboard-item-added", &item) {
        eprintln!("Failed to emit clipboard-item-added event: {e}");
    }
    write_file_list(path).map_err(|e| format!("Fetched, but couldn't copy it: {e}"))
}

/// A fetched file's metadata: no longer remote, pointing at the local copy.
fn fetched_metadata(item: &ClipboardItem, remote: &RemoteFile, path: &str) -> Value {
    let mut metadata = match serde_json::from_str::<Value>(&item.content_metadata) {
        Ok(value @ Value::Object(_)) => value,
        _ => json!({}),
    };
    if let Some(object) = metadata.as_object_mut() {
        object.remove("remote");
        object.insert("source".into(), json!("file"));
        object.insert("external_path".into(), json!(path));
        object.insert("fetched_from".into(), json!(remote.origin_name));
    }
    metadata
}

fn load_item(app: &AppHandle, item_id: &str) -> Result<ClipboardItem, String> {
    with_repo(app, |repo| {
        repo.get_item(item_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "That item no longer exists".to_string())
    })
}

fn with_repo<T>(app: &AppHandle, f: impl FnOnce(&ClipboardRepository) -> Result<T, String>) -> Result<T, String> {
    let db_path = {
        let state = app.state::<Mutex<AppState>>();
        let state = lock(&state);
        state.db_path.clone()
    };
    let repo = ClipboardRepository::new(&db_path).map_err(|e| format!("Could not open history: {e}"))?;
    f(&repo)
}

fn file_name_of(path: &Path) -> Option<String> {
    path.file_name().map(|name| name.to_string_lossy().to_string())
}

fn emit_done(app: &AppHandle, item_id: &str, error: Option<String>, cancelled: bool) {
    let done = FileDone {
        item_id,
        ok: error.is_none() && !cancelled,
        cancelled,
        error,
    };
    if let Err(e) = app.emit("sync-file-done", done) {
        eprintln!("Failed to emit sync-file-done event: {e}");
    }
}

/// Emits progress at most every `PROGRESS_INTERVAL`, plus the final value.
struct Throttle {
    last: Option<Instant>,
}

impl Throttle {
    fn new() -> Self {
        Throttle { last: None }
    }

    fn emit(&mut self, app: &AppHandle, item_id: &str, phase: &str, done: u64, total: u64) {
        let is_due = self.last.is_none_or(|last| last.elapsed() >= PROGRESS_INTERVAL);
        if !is_due && done < total {
            return;
        }
        self.last = Some(Instant::now());
        let progress = FileProgress {
            item_id,
            phase,
            done,
            total,
        };
        if let Err(e) = app.emit("sync-file-progress", progress) {
            eprintln!("Failed to emit sync-file-progress event: {e}");
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote() -> RemoteFile {
        RemoteFile {
            offer_id: "o".into(),
            origin_id: "d".into(),
            origin_name: "Laptop".into(),
            name: "report.pdf".into(),
            size: 17,
            sha256: "h".into(),
        }
    }

    fn item_with(metadata: &str) -> ClipboardItem {
        ClipboardItem {
            id: "item".into(),
            content_type: "file".into(),
            content_text: Some("report.pdf".into()),
            content_metadata: metadata.into(),
            source_app: None,
            code_language: None,
            file_url: None,
            file_name: Some("report.pdf".into()),
            file_size_bytes: Some(17),
            file_mime_type: None,
            file_hash: None,
            is_favorite: false,
            is_snippet: false,
            snippet_name: None,
            created_at: String::new(),
            updated_at: String::new(),
            synced: false,
            server_id: None,
        }
    }

    #[test]
    fn fetched_item_is_no_longer_remote() {
        let item = item_with(r#"{"remote":{"offer_id":"o"},"original_name":"report.pdf"}"#);
        let metadata = fetched_metadata(&item, &remote(), "/dl/report.pdf");

        assert!(metadata.get("remote").is_none());
    }

    #[test]
    fn fetched_item_points_at_the_local_copy() {
        let item = item_with(r#"{"remote":{"offer_id":"o"}}"#);
        let metadata = fetched_metadata(&item, &remote(), "/dl/report.pdf");

        assert_eq!(
            (metadata["external_path"].as_str(), metadata["fetched_from"].as_str()),
            (Some("/dl/report.pdf"), Some("Laptop"))
        );
    }

    #[test]
    fn fetched_item_keeps_its_other_details() {
        let item = item_with(r#"{"remote":{"offer_id":"o"},"original_name":"report.pdf"}"#);
        let metadata = fetched_metadata(&item, &remote(), "/dl/report.pdf");

        assert_eq!(metadata["original_name"], "report.pdf");
    }

    #[test]
    fn fetched_item_counts_as_a_local_file() {
        let item = item_with(r#"{"remote":{"offer_id":"o"},"source":"remote"}"#);
        let metadata = fetched_metadata(&item, &remote(), "/dl/report.pdf");

        assert_eq!(metadata["source"], "file");
    }
}
