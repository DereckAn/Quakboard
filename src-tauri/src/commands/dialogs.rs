use std::path::PathBuf;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

use crate::clipboard::state::KeepWindowOpen;

/// Show a folder picker. Runs from the backend rather than the web view so
/// the window can be kept from auto-hiding while the dialog has the focus;
/// otherwise the window hides as the picker opens and the picker is
/// orphaned behind it. Returns None if the user closed the picker.
pub async fn pick_folder(
    app: &AppHandle,
    title: &str,
    start_in: Option<PathBuf>,
) -> Result<Option<PathBuf>, String> {
    let (app, title) = (app.clone(), title.to_string());
    let picked = tauri::async_runtime::spawn_blocking(move || {
        let _keep_open = KeepWindowOpen::new();
        let mut dialog = app.dialog().file().set_title(title);
        if let Some(dir) = start_in.filter(|dir| dir.is_dir()) {
            dialog = dialog.set_directory(dir);
        }
        dialog.blocking_pick_folder()
    })
    .await
    .map_err(|e| e.to_string())?;

    match picked {
        None => Ok(None),
        Some(picked) => picked
            .as_path()
            .map(|path| Some(path.to_path_buf()))
            .ok_or_else(|| "Choose a folder on this computer".to_string()),
    }
}

#[tauri::command]
pub async fn choose_folder(
    app: AppHandle,
    title: String,
    start_in: Option<String>,
) -> Result<Option<String>, String> {
    let picked = pick_folder(&app, &title, start_in.map(PathBuf::from)).await?;
    Ok(picked.map(|path| path.to_string_lossy().to_string()))
}
