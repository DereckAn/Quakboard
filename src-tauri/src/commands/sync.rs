use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::settings::save_setting;
use crate::sync::{
    body::MAX_IMAGE_BYTES,
    discovery::short_id,
    runtime::{
        local_addresses, SyncRuntime, DOWNLOAD_DIR_SETTING_KEY, IMAGES_SETTING_KEY, SETTING_KEY,
    },
    sharing,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    enabled: bool,
    running: bool,
    device_name: Option<String>,
    short_id: Option<String>,
    /// This device's LAN IPs, for connecting by IP from the other device.
    addresses: Vec<String>,
    sync_images: bool,
    /// So the UI shows the real limit, not a copy that can drift.
    max_image_bytes: usize,
}

/// A paired device as the UI sees it. Deliberately has no key.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPeer {
    id: String,
    name: String,
    last_addr: Option<String>,
    /// Seen on the network right now (mDNS), for the Send picker.
    is_online: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NearbyDevice {
    id: String,
    short_id: String,
    addr: String,
}

fn status(runtime: &SyncRuntime) -> SyncStatus {
    let identity = runtime.service().map(|service| service.identity());
    SyncStatus {
        enabled: runtime.is_enabled(),
        running: identity.is_some(),
        sync_images: runtime.syncs_images(),
        max_image_bytes: MAX_IMAGE_BYTES,
        addresses: if identity.is_some() { local_addresses() } else { Vec::new() },
        short_id: identity.as_ref().map(|(id, _)| short_id(id).to_string()),
        device_name: identity.map(|(_, name)| name),
    }
}

#[tauri::command]
pub fn sync_get_status(runtime: State<'_, SyncRuntime>) -> SyncStatus {
    status(&runtime)
}

#[tauri::command]
pub async fn sync_set_enabled(
    app: AppHandle,
    runtime: State<'_, SyncRuntime>,
    enabled: bool,
) -> Result<SyncStatus, String> {
    if enabled {
        // Only remember "on" once it actually started (e.g. the port was free).
        runtime.start(&app).await?;
    } else {
        runtime.stop();
    }
    save_setting(app, SETTING_KEY.into(), enabled.to_string())?;
    Ok(status(&runtime))
}

/// Works while sync is off too, so the choice is remembered for later.
#[tauri::command]
pub fn sync_set_images_enabled(
    app: AppHandle,
    runtime: State<'_, SyncRuntime>,
    enabled: bool,
) -> Result<SyncStatus, String> {
    save_setting(app, IMAGES_SETTING_KEY.into(), enabled.to_string())?;
    if let Some(service) = runtime.service() {
        service.set_syncing_images(enabled);
    }
    Ok(status(&runtime))
}

#[tauri::command]
pub fn sync_list_peers(runtime: State<'_, SyncRuntime>) -> Vec<SyncPeer> {
    let Some(service) = runtime.service() else {
        return Vec::new();
    };
    service
        .peers()
        .into_iter()
        .map(|peer| SyncPeer {
            is_online: runtime.address_of(&peer.id).is_some(),
            id: peer.id,
            name: peer.name,
            last_addr: peer.last_addr,
        })
        .collect()
}

/// Devices on the network that aren't paired yet.
#[tauri::command]
pub fn sync_list_nearby(runtime: State<'_, SyncRuntime>) -> Vec<NearbyDevice> {
    let paired: Vec<String> = runtime
        .service()
        .map(|service| service.peers().into_iter().map(|peer| peer.id).collect())
        .unwrap_or_default();
    runtime
        .nearby_devices()
        .into_iter()
        .filter(|device| !paired.contains(&device.id))
        .map(|device| NearbyDevice {
            short_id: short_id(&device.id).to_string(),
            id: device.id,
            addr: device.addr.to_string(),
        })
        .collect()
}

#[tauri::command]
pub fn sync_set_accepting_pairing(runtime: State<'_, SyncRuntime>, accepting: bool) {
    if let Some(service) = runtime.service() {
        service.set_accepting_pairing(accepting);
    }
}

#[tauri::command]
pub fn sync_pair_start(runtime: State<'_, SyncRuntime>, device_id: String) -> Result<(), String> {
    runtime.start_pairing(&device_id)
}

#[tauri::command]
pub fn sync_pair_by_address(runtime: State<'_, SyncRuntime>, address: String) -> Result<(), String> {
    runtime.start_pairing_by_address(&address)
}

#[tauri::command]
pub fn sync_pair_submit_code(runtime: State<'_, SyncRuntime>, code: String) -> Result<(), String> {
    runtime.submit_code(code)
}

#[tauri::command]
pub fn sync_pair_cancel(runtime: State<'_, SyncRuntime>) {
    runtime.cancel_pairing();
}

#[tauri::command]
pub fn sync_unpair(runtime: State<'_, SyncRuntime>, peer_id: String) -> Result<(), String> {
    let service = runtime.service().ok_or("Sync is off")?;
    service.unpair(&peer_id).map(|_| ()).map_err(|e| e.to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendFileResult {
    /// Chosen devices that got the offer right now.
    reached: usize,
    chosen: usize,
}

#[tauri::command]
pub async fn sync_send_file(
    app: AppHandle,
    runtime: State<'_, SyncRuntime>,
    item_id: String,
    device_ids: Vec<String>,
) -> Result<SendFileResult, String> {
    let reached = sharing::send_file(&app, &runtime, &item_id, &device_ids).await?;
    Ok(SendFileResult {
        reached,
        chosen: device_ids.len(),
    })
}

/// Starts the fetch; progress and the outcome arrive as events.
#[tauri::command]
pub fn sync_fetch_file(app: AppHandle, runtime: State<'_, SyncRuntime>, item_id: String) -> Result<(), String> {
    sharing::start_fetch(&app, &runtime, &item_id)
}

#[tauri::command]
pub fn sync_cancel_fetch(app: AppHandle, runtime: State<'_, SyncRuntime>, item_id: String) {
    sharing::cancel_fetch(&app, &runtime, &item_id);
}

#[tauri::command]
pub fn sync_get_download_dir(app: AppHandle, runtime: State<'_, SyncRuntime>) -> Result<String, String> {
    sharing::download_dir(&app, &runtime).map(|dir| dir.to_string_lossy().to_string())
}

/// Show a folder picker for the download folder. Returns the folder saved,
/// or None if the user closed the picker.
#[tauri::command]
pub async fn sync_choose_download_dir(
    app: AppHandle,
    runtime: State<'_, SyncRuntime>,
) -> Result<Option<String>, String> {
    let current = sharing::download_dir(&app, &runtime).ok();
    let picked = crate::commands::dialogs::pick_folder(&app, "Where should fetched files go?", current).await?;
    match picked {
        Some(path) => use_download_dir(app, &path.to_string_lossy()).map(Some),
        None => Ok(None),
    }
}

fn use_download_dir(app: AppHandle, path: &str) -> Result<String, String> {
    let dir = std::path::PathBuf::from(path.trim());
    if !dir.is_absolute() {
        return Err("Choose a full folder path".into());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Can't use that folder: {e}"))?;
    if !dir.is_dir() {
        return Err("That path isn't a folder".into());
    }
    let dir = dir.to_string_lossy().to_string();
    save_setting(app, DOWNLOAD_DIR_SETTING_KEY.into(), dir.clone())?;
    Ok(dir)
}
