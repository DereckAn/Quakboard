use crate::clipboard::operations::write_clipboard_image;
use crate::clipboard::state::take_previous_app_pid;
use crate::clipboard::{
    asset_cleanup, detect_backend, read_clipboard, write_clipboard, ClipboardBackend,
};
use crate::db::models::{ClipboardItem, CreateClipboardItemDto, UpdateClipboardItemDto};
use crate::db::repository::ClipboardRepository;
use enigo::{Enigo, Key, Keyboard, Settings};
use std::sync::Mutex;
use tauri::State;

pub struct AppState {
    pub db_path: String,
    pub images_dir: String,
    pub app_data_dir: String,
}

// Removed: Use get_clipboard_items_paginated() instead for better performance

#[tauri::command]
pub fn get_clipboard_item(
    id: String,
    state: State<Mutex<AppState>>,
) -> Result<Option<ClipboardItem>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.get_item(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn create_clipboard_item(
    dto: CreateClipboardItemDto,
    state: State<Mutex<AppState>>,
) -> Result<ClipboardItem, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.create_item(dto).map_err(|e| e.to_string())
}

/// Create or update item - prevents duplicates by bumping existing items
#[tauri::command]
pub fn upsert_clipboard_item(
    dto: CreateClipboardItemDto,
    state: State<Mutex<AppState>>,
) -> Result<ClipboardItem, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.upsert_item(dto).map_err(|e| e.to_string())
}

/// Bump an existing item to the top (update timestamp)
#[tauri::command]
pub fn bump_clipboard_item(
    id: String,
    state: State<Mutex<AppState>>,
) -> Result<ClipboardItem, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.bump_item(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn update_clipboard_item(
    id: String,
    dto: UpdateClipboardItemDto,
    state: State<Mutex<AppState>>,
) -> Result<ClipboardItem, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.update_item(&id, dto).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_clipboard_item(id: String, state: State<Mutex<AppState>>) -> Result<(), String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    if let Some(item) = repo.get_item(&id).map_err(|e| e.to_string())? {
        if let Some(file_url) = item.file_url.as_deref() {
            asset_cleanup::delete_file_url(&item.content_type, file_url, &item.content_metadata);
        }
    }
    repo.delete_item(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_all_clipboard_items(state: State<Mutex<AppState>>) -> Result<(), String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;

    let mut stmt = repo
        .conn
        .prepare(
            "SELECT content_type, content_metadata, file_url
             FROM clipboard_items
             WHERE file_url IS NOT NULL",
        )
        .map_err(|e| e.to_string())?;
    let file_assets: Vec<(String, Option<String>, String)> = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .filter_map(|res| res.ok())
        .collect();

    repo.clear_all().map_err(|e| e.to_string())?;

    for (content_type, metadata, path) in file_assets {
        asset_cleanup::delete_file_url(&content_type, &path, metadata.as_deref().unwrap_or("{}"));
    }
    Ok(())
}

#[tauri::command]
pub fn read_from_clipboard() -> Result<String, String> {
    read_clipboard()
}

#[tauri::command]
pub fn write_to_clipboard(text: String) -> Result<(), String> {
    write_clipboard(&text)
}

#[tauri::command]
pub fn remove_duplicate_items(state: State<Mutex<AppState>>) -> Result<usize, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.remove_duplicates().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn cleanup_missing_clipboard_files(
    state: State<Mutex<AppState>>,
) -> Result<Vec<String>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.cleanup_missing_file_records()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_clipboard_items_paginated(
    limit: i64,
    offset: i64,
    state: State<Mutex<AppState>>,
) -> Result<Vec<ClipboardItem>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.get_items_paginated(limit, offset)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn count_clipboard_items(state: State<Mutex<AppState>>) -> Result<i64, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.count_items().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_clipboard_items_fts(
    query: String,
    limit: i64,
    offset: i64,
    state: State<Mutex<AppState>>,
) -> Result<Vec<ClipboardItem>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.search_items_fts(&query, limit, offset)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn count_search_results_fts(
    query: String,
    state: State<Mutex<AppState>>,
) -> Result<i64, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let repo = ClipboardRepository::new(&app_state.db_path).map_err(|e| e.to_string())?;
    repo.count_search_results_fts(&query)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn write_image_to_clipboard(image_path: String) -> Result<(), String> {
    write_clipboard_image(&image_path)
}

#[tauri::command]
pub fn write_file_to_clipboard(path: String) -> Result<(), String> {
    crate::clipboard::operations::write_file_list(std::path::Path::new(&path))
}

/// Terminals paste with Ctrl+Shift+V; plain Ctrl+V reaches the shell as `^V`.
/// ponytail: known-class list; unknown terminals get Ctrl+V. Add a per-app
/// paste-shortcut setting if users hit that.
#[cfg(not(target_os = "macos"))]
const TERMINAL_CLASSES: &[&str] = &[
    "kitty",
    "alacritty",
    "foot",
    "footclient",
    "org.wezfurlong.wezterm",
    "com.mitchellh.ghostty",
    "dev.warp.warp",
    "org.kde.konsole",
    "org.gnome.terminal",
    "org.gnome.console",
    "org.gnome.ptyxis",
    "xterm",
    "tilix",
    "terminator",
    "xfce4-terminal",
];

#[cfg(not(target_os = "macos"))]
struct HyprWindow {
    address: String,
    class: String,
}

/// On Hyprland, find the window that was focused right before ours
/// (focusHistoryID == 1) — i.e. the app the user wants to paste into.
#[cfg(not(target_os = "macos"))]
fn hypr_previous_window() -> Option<HyprWindow> {
    let output = std::process::Command::new("hyprctl")
        .args(["clients", "-j"])
        .output()
        .ok()?;
    let clients: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let client = clients
        .as_array()?
        .iter()
        .find(|c| c.get("focusHistoryID").and_then(|v| v.as_i64()) == Some(1))?;
    Some(HyprWindow {
        address: client.get("address")?.as_str()?.to_string(),
        class: client
            .get("class")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase(),
    })
}

/// Run `hyprctl dispatch` with each argument form in turn until one succeeds.
/// Hyprland 0.55+ evaluates `dispatch` arguments as Lua and rejects the legacy
/// syntax, so callers pass the Lua form first and the legacy form as fallback.
#[cfg(not(target_os = "macos"))]
fn hypr_dispatch(forms: &[&[&str]]) -> bool {
    forms.iter().any(|args| {
        std::process::Command::new("hyprctl")
            .arg("dispatch")
            .args(*args)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "ok")
            .unwrap_or(false)
    })
}

#[cfg(not(target_os = "macos"))]
fn hypr_focus_window(addr: &str) {
    let lua = format!("hl.dsp.focus({{ window = \"address:{addr}\" }})");
    let legacy = format!("address:{addr}");
    if !hypr_dispatch(&[&[&lua], &["focuswindow", &legacy]]) {
        eprintln!("Failed to focus paste target window {addr}");
    }
}

/// Send the paste shortcut through Hyprland's real keyboard. wtype uses its own
/// virtual keymap, so Electron apps (VS Code) read a different physical key
/// than `v` and trigger unrelated shortcuts.
#[cfg(not(target_os = "macos"))]
fn hypr_send_paste(target: &HyprWindow) -> bool {
    let mods = if TERMINAL_CLASSES.contains(&target.class.as_str()) {
        "CTRL SHIFT"
    } else {
        "CTRL"
    };
    let addr = &target.address;
    let lua = format!(
        "hl.dsp.send_shortcut({{ mods = \"{mods}\", key = \"v\", window = \"address:{addr}\" }})"
    );
    let legacy = format!("{mods}, V, address:{addr}");
    hypr_dispatch(&[&[&lua], &["sendshortcut", &legacy]])
}

#[tauri::command]
pub fn paste_item(text: String, window: tauri::WebviewWindow) -> Result<(), String> {
    // On Wayland, remember which window to paste into BEFORE we hide ours
    // (right now our app is focused; the target is the previous window).
    #[cfg(not(target_os = "macos"))]
    let wayland_paste_target = if detect_backend() == ClipboardBackend::Wayland {
        hypr_previous_window()
    } else {
        None
    };

    // 1. Write the text to the clipboard
    write_clipboard(&text)?;

    // Hide the window so the previous app gets docus bacj
    window.hide().map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::NSWorkspace;

        let pid = take_previous_app_pid();

        if pid != -1 {
            let workspace = NSWorkspace::sharedWorkspace();
            let apps = workspace.runningApplications();
            for app in apps.iter() {
                if app.processIdentifier() == pid {
                    app.activateWithOptions(objc2_app_kit::NSApplicationActivationOptions::empty());
                    break;
                }
            }
        }
        // 3. Wait a moment for the OS to switch focus
        std::thread::sleep(std::time::Duration::from_millis(150));
    }

    #[cfg(not(target_os = "macos"))]
    std::thread::sleep(std::time::Duration::from_millis(150));

    // On Wayland, enigo can't inject keystrokes (Wayland blocks synthetic input).
    // On Hyprland the compositor sends the shortcut; elsewhere fall back to
    // `wtype`. Either way return early before the enigo path.
    #[cfg(not(target_os = "macos"))]
    {
        if detect_backend() == ClipboardBackend::Wayland {
            // window.hide() doesn't reliably transfer focus here, so explicitly
            // focus the target window before sending the paste shortcut.
            if let Some(target) = &wayland_paste_target {
                hypr_focus_window(&target.address);
                std::thread::sleep(std::time::Duration::from_millis(80));
                if hypr_send_paste(target) {
                    return Ok(());
                }
                eprintln!("Hyprland send_shortcut failed; falling back to wtype");
            }

            let status = std::process::Command::new("wtype")
                .args(["-s", "80", "-M", "ctrl", "-k", "v", "-m", "ctrl"])
                .status()
                .map_err(|e| format!("Failed to run wtype: {e}"))?;

            return if status.success() {
                Ok(())
            } else {
                Err(format!("wtype exited with status: {status}"))
            };
        }
    }

    // 4. Simulate Cmd+V to paste
    let mut enigo = Enigo::new(&Settings::default()).map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    enigo
        .key(Key::Meta, enigo::Direction::Press)
        .map_err(|e| e.to_string())?;

    #[cfg(not(target_os = "macos"))]
    enigo
        .key(Key::Control, enigo::Direction::Press)
        .map_err(|e| e.to_string())?;

    enigo
        .key(Key::Unicode('v'), enigo::Direction::Click)
        .map_err(|e| e.to_string())?;

    #[cfg(target_os = "macos")]
    enigo
        .key(Key::Meta, enigo::Direction::Release)
        .map_err(|e| e.to_string())?;

    #[cfg(not(target_os = "macos"))]
    enigo
        .key(Key::Control, enigo::Direction::Release)
        .map_err(|e| e.to_string())?;

    Ok(())
}
