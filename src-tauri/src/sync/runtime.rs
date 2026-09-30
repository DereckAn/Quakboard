//! Turning sync on and off while the app runs. Registered as Tauri state once
//! at startup; the service inside exists only while sync is enabled.

use std::{
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tauri::{async_runtime::JoinHandle, AppHandle, Emitter, Manager};
use tokio::{net::TcpListener, sync::oneshot};

use super::{
    discovery::{DiscoveredDevice, Discovery},
    received::store_received_image,
    service::{ReceivedImage, SyncHooks, SyncService},
    store::{SyncStore, STORE_FILE_NAME},
    transport::SYNC_PORT,
    ClipPayload,
};
use crate::{
    clipboard::operations::write_clipboard_image, commands::AppState,
    db::repository::ClipboardRepository,
};

pub const SETTING_KEY: &str = "syncEnabled";
pub const IMAGES_SETTING_KEY: &str = "syncImages";

pub struct SyncRuntime {
    app_data_dir: PathBuf,
    running: Mutex<Option<Running>>,
    /// Delivers the code the user types, for a pairing this device started.
    pending_code: Mutex<Option<oneshot::Sender<String>>>,
}

struct Running {
    service: Arc<SyncService>,
    /// None when mDNS couldn't start; connecting by IP still works then.
    /// Shared with the service, which looks up devices whose address changed.
    discovery: Option<Arc<Discovery>>,
    listener: JoinHandle<()>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Frees the port; dropping `discovery` sends mDNS goodbyes.
        self.listener.abort();
    }
}

impl SyncRuntime {
    pub fn new(app_data_dir: &Path) -> Self {
        SyncRuntime {
            app_data_dir: app_data_dir.to_path_buf(),
            running: Mutex::new(None),
            pending_code: Mutex::new(None),
        }
    }

    /// Whether the user turned sync on. Off unless set.
    pub fn is_enabled(&self) -> bool {
        self.flag(SETTING_KEY).unwrap_or(false)
    }

    /// Whether images sync too, once sync is on. On unless turned off.
    pub fn syncs_images(&self) -> bool {
        self.flag(IMAGES_SETTING_KEY).unwrap_or(true)
    }

    /// A true/false setting. `save_setting` stores values as strings.
    fn flag(&self, key: &str) -> Option<bool> {
        let contents = std::fs::read_to_string(self.app_data_dir.join("settings.json")).ok()?;
        let settings: serde_json::Value = serde_json::from_str(&contents).ok()?;
        match settings.get(key)? {
            serde_json::Value::Bool(value) => Some(*value),
            serde_json::Value::String(value) => Some(value == "true"),
            _ => None,
        }
    }

    pub fn service(&self) -> Option<Arc<SyncService>> {
        lock(&self.running)
            .as_ref()
            .map(|running| running.service.clone())
    }

    pub fn nearby_devices(&self) -> Vec<DiscoveredDevice> {
        lock(&self.running)
            .as_ref()
            .and_then(|running| running.discovery.as_ref())
            .map(|discovery| discovery.devices())
            .unwrap_or_default()
    }

    pub fn address_of(&self, device_id: &str) -> Option<String> {
        lock(&self.running)
            .as_ref()
            .and_then(|running| running.discovery.as_ref())
            .and_then(|discovery| discovery.address_of(device_id))
            .map(|addr| addr.to_string())
    }

    /// Start listening and advertising. Does nothing if already running.
    pub async fn start(&self, app: &AppHandle) -> Result<(), String> {
        if self.service().is_some() {
            return Ok(());
        }

        // Bind first: if the port is taken, report it instead of half-starting.
        let listener = TcpListener::bind(("0.0.0.0", SYNC_PORT))
            .await
            .map_err(|e| format!("Could not listen on port {SYNC_PORT}: {e}"))?;

        let store_path = self.app_data_dir.join(STORE_FILE_NAME);
        let store = SyncStore::load_or_create(&store_path).map_err(|e| e.to_string())?;
        let discovery = Discovery::start(&store.device_id, SYNC_PORT)
            .map(Arc::new)
            .map_err(|e| eprintln!("Sync discovery unavailable, connect by IP instead: {e}"))
            .ok();
        let service = SyncService::new(store, store_path, SYNC_PORT, hooks(app, discovery.clone()));
        service.set_syncing_images(self.syncs_images());
        let listener = tauri::async_runtime::spawn(service.clone().listen(listener));
        println!("Sync listening on port {SYNC_PORT}");

        *lock(&self.running) = Some(Running {
            service,
            discovery,
            listener,
        });
        Ok(())
    }

    pub fn stop(&self) {
        self.cancel_pairing();
        if lock(&self.running).take().is_some() {
            println!("Sync stopped");
        }
    }

    /// Pair with a device found nearby. The other device then shows a code,
    /// which the user submits with `submit_code`; the outcome arrives as a
    /// `sync-pairing` event.
    pub fn start_pairing(&self, device_id: &str) -> Result<(), String> {
        let addr = self
            .address_of(device_id)
            .ok_or("That device is no longer on the network")?;
        self.start_pairing_at(addr)
    }

    /// Pair with a device by the IP the user typed, for networks that block
    /// mDNS. Same flow as `start_pairing` from there.
    pub fn start_pairing_by_address(&self, input: &str) -> Result<(), String> {
        let addr = parse_peer_address(input)?;
        self.start_pairing_at(addr.to_string())
    }

    fn start_pairing_at(&self, addr: String) -> Result<(), String> {
        let service = self.service().ok_or("Sync is off")?;
        let (code_tx, code_rx) = oneshot::channel();
        // Replacing an older sender cancels that attempt's code entry.
        *lock(&self.pending_code) = Some(code_tx);
        tauri::async_runtime::spawn(async move {
            // The outcome reaches the UI through the pairing hook.
            let _ = service.pair_with(&addr, code_rx).await;
        });
        Ok(())
    }

    pub fn submit_code(&self, code: String) -> Result<(), String> {
        let code_tx = lock(&self.pending_code)
            .take()
            .ok_or("No pairing is waiting for a code")?;
        code_tx
            .send(code)
            .map_err(|_| "That pairing already ended".to_string())
    }

    pub fn cancel_pairing(&self) {
        // Dropping the sender makes the waiting pairing end as cancelled.
        lock(&self.pending_code).take();
    }
}

/// Accept `192.168.1.20` or `192.168.1.20:47823`. IPs only, no hostnames:
/// looking a name up would leak it to DNS and isn't needed on a LAN.
pub fn parse_peer_address(input: &str) -> Result<SocketAddr, String> {
    let input = input.trim();
    if let Ok(addr) = input.parse::<SocketAddr>() {
        return Ok(addr);
    }
    input
        .parse::<IpAddr>()
        .map(|ip| SocketAddr::new(ip, SYNC_PORT))
        .map_err(|_| format!("\"{input}\" isn't an IP address, like 192.168.1.20"))
}

/// This device's LAN addresses, to read off and type on the other device.
pub fn local_addresses() -> Vec<String> {
    let mut addresses: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|iface| !iface.is_loopback() && !iface.is_link_local())
        // ponytail: IPv4 only, matching discovery.
        .filter(|iface| iface.ip().is_ipv4())
        .map(|iface| iface.ip().to_string())
        .collect();
    addresses.sort();
    addresses.dedup();
    addresses
}

fn hooks(app: &AppHandle, discovery: Option<Arc<Discovery>>) -> SyncHooks {
    let event_app = app.clone();
    let image_app = app.clone();
    SyncHooks {
        apply_clip: Arc::new(|clip: ClipPayload| {
            // Writing spawns wl-copy / talks to the OS clipboard; keep it off
            // the async workers.
            tauri::async_runtime::spawn_blocking(move || {
                if let Err(e) = crate::clipboard::operations::write_clipboard(&clip.text) {
                    eprintln!("Failed to apply synced clip: {e}");
                }
            });
        }),
        apply_image: Arc::new(move |image: ReceivedImage| {
            let app = image_app.clone();
            // File and database work; keep it off the async workers.
            tauri::async_runtime::spawn_blocking(move || apply_received_image(&app, image));
        }),
        on_pairing: Arc::new(move |event| {
            if let Err(e) = event_app.emit("sync-pairing", event) {
                eprintln!("Failed to emit sync-pairing event: {e}");
            }
        }),
        resolve_addr: Arc::new(move |device_id| {
            discovery.as_ref().and_then(|d| d.address_of(device_id))
        }),
    }
}

/// Store a received image in history, show it, and put it on the clipboard.
fn apply_received_image(app: &AppHandle, image: ReceivedImage) {
    let (db_path, images_dir) = {
        let state = app.state::<Mutex<AppState>>();
        let state = lock(&state);
        (state.db_path.clone(), state.images_dir.clone())
    };
    let stored = ClipboardRepository::new(&db_path)
        .map_err(|e| format!("Failed to open history: {e}"))
        .and_then(|repo| {
            store_received_image(
                &repo,
                Path::new(&images_dir),
                &image.png,
                &image.meta,
                &image.from_name,
            )
        });
    let stored = match stored {
        Ok(stored) => stored,
        Err(e) => {
            eprintln!("Failed to store image from {}: {e}", image.from_name);
            return;
        }
    };

    if let Err(e) = app.emit("clipboard-item-added", &stored.item) {
        eprintln!("Failed to emit clipboard-item-added event: {e}");
    }
    // write_clipboard_image asks the monitor to skip its own write, so the
    // image isn't stored again here or sent back (see docs/IMAGE_SYNC_PLAN.md).
    if let Err(e) = write_clipboard_image(&stored.path.to_string_lossy()) {
        eprintln!("Failed to put synced image on the clipboard: {e}");
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_with_settings(json: &str) -> (tempfile::TempDir, SyncRuntime) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), json).unwrap();
        let runtime = SyncRuntime::new(dir.path());
        (dir, runtime)
    }

    #[test]
    fn sync_is_off_without_a_setting() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!SyncRuntime::new(dir.path()).is_enabled());
    }

    #[test]
    fn sync_is_on_when_the_setting_is_the_string_true() {
        let (_dir, runtime) = runtime_with_settings(r#"{"syncEnabled":"true"}"#);
        assert!(runtime.is_enabled());
    }

    #[test]
    fn sync_is_off_when_the_setting_is_the_string_false() {
        let (_dir, runtime) = runtime_with_settings(r#"{"syncEnabled":"false"}"#);
        assert!(!runtime.is_enabled());
    }

    #[test]
    fn images_sync_by_default() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SyncRuntime::new(dir.path()).syncs_images());
    }

    #[test]
    fn images_stop_syncing_when_turned_off() {
        let (_dir, runtime) = runtime_with_settings(r#"{"syncImages":"false"}"#);
        assert!(!runtime.syncs_images());
    }

    #[test]
    fn a_malformed_image_setting_falls_back_to_on() {
        let (_dir, runtime) = runtime_with_settings(r#"{"syncImages":42}"#);
        assert!(runtime.syncs_images());
    }

    #[test]
    fn submitting_a_code_without_a_pairing_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SyncRuntime::new(dir.path()).submit_code("123456".into()).is_err());
    }

    #[test]
    fn bare_ip_gets_the_sync_port() {
        assert_eq!(
            parse_peer_address(" 192.168.1.20 "),
            Ok("192.168.1.20:47823".parse().unwrap())
        );
    }

    #[test]
    fn ip_with_port_is_kept_as_is() {
        assert_eq!(
            parse_peer_address("192.168.1.20:5000"),
            Ok("192.168.1.20:5000".parse().unwrap())
        );
    }

    #[test]
    fn hostnames_are_rejected() {
        assert!(parse_peer_address("my-laptop.local").is_err());
    }

    #[test]
    fn local_addresses_exclude_loopback() {
        assert!(!local_addresses().contains(&"127.0.0.1".to_string()));
    }

    #[test]
    fn stopped_runtime_has_no_service() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = SyncRuntime::new(dir.path());
        runtime.stop();
        assert!(runtime.service().is_none());
    }
}
