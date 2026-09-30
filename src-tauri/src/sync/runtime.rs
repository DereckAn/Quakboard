//! Turning sync on and off while the app runs. Registered as Tauri state once
//! at startup; the service inside exists only while sync is enabled.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use tauri::{async_runtime::JoinHandle, AppHandle, Emitter};
use tokio::{net::TcpListener, sync::oneshot};

use super::{
    discovery::{DiscoveredDevice, Discovery},
    service::{SyncHooks, SyncService},
    store::{SyncStore, STORE_FILE_NAME},
    transport::SYNC_PORT,
    ClipPayload,
};

pub const SETTING_KEY: &str = "syncEnabled";

pub struct SyncRuntime {
    app_data_dir: PathBuf,
    running: Mutex<Option<Running>>,
    /// Delivers the code the user types, for a pairing this device started.
    pending_code: Mutex<Option<oneshot::Sender<String>>>,
}

struct Running {
    service: Arc<SyncService>,
    /// None when mDNS couldn't start; connecting by IP still works then.
    discovery: Option<Discovery>,
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

    /// Whether the user turned sync on. `save_setting` stores values as strings.
    pub fn is_enabled(&self) -> bool {
        std::fs::read_to_string(self.app_data_dir.join("settings.json"))
            .ok()
            .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
            .and_then(|settings| settings.get(SETTING_KEY).cloned())
            .is_some_and(|value| value == "true" || value == true)
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
            .map(Discovery::devices)
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
        let device_id = store.device_id.clone();
        let service = SyncService::new(store, store_path, SYNC_PORT, hooks(app));

        let discovery = Discovery::start(&device_id, SYNC_PORT)
            .map_err(|e| eprintln!("Sync discovery unavailable, connect by IP instead: {e}"))
            .ok();
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
        let service = self.service().ok_or("Sync is off")?;
        let addr = self
            .address_of(device_id)
            .ok_or("That device is no longer on the network")?;

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

fn hooks(app: &AppHandle) -> SyncHooks {
    let event_app = app.clone();
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
        on_pairing: Arc::new(move |event| {
            if let Err(e) = event_app.emit("sync-pairing", event) {
                eprintln!("Failed to emit sync-pairing event: {e}");
            }
        }),
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
    fn submitting_a_code_without_a_pairing_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(SyncRuntime::new(dir.path()).submit_code("123456".into()).is_err());
    }

    #[test]
    fn stopped_runtime_has_no_service() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = SyncRuntime::new(dir.path());
        runtime.stop();
        assert!(runtime.service().is_none());
    }
}
