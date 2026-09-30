//! The running sync service: receives clips from paired devices, sends local
//! copies to them, and pairs new devices.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinSet,
};

use super::{
    pairing::{self, LocalDevice, PairError},
    store::{Peer, SyncStore, STORE_FILE_NAME},
    transport::{self, SYNC_PORT},
    ClipPayload, Frame,
};

/// Puts a received clip on the system clipboard.
pub type ApplyClip = Arc<dyn Fn(ClipPayload) + Send + Sync>;
/// Tells the UI what pairing is doing.
pub type OnPairing = Arc<dyn Fn(PairingEvent) + Send + Sync>;

/// The service's effects on the outside world, injected so tests don't touch
/// the real clipboard or UI.
pub struct SyncHooks {
    pub apply_clip: ApplyClip,
    pub on_pairing: OnPairing,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PairingEvent {
    /// Another device asked to pair; show this code so the user can type it there.
    #[serde(rename_all = "camelCase")]
    CodeShown { code: String },
    #[serde(rename_all = "camelCase")]
    Paired { peer_id: String, name: String },
    #[serde(rename_all = "camelCase")]
    Failed { reason: String },
}

pub struct SyncService {
    store: Mutex<SyncStore>,
    store_path: PathBuf,
    /// Sent to devices during pairing so they know where to reach us.
    listen_port: u16,
    /// One pairing at a time, so a device on the LAN can't stack up prompts.
    is_pairing: AtomicBool,
    /// Text of the last received clip. Writing it to the clipboard makes the
    /// monitor see it as a new copy; this stops that one copy being sent back.
    last_received: Mutex<Option<String>>,
    hooks: SyncHooks,
}

impl SyncService {
    pub fn new(
        store: SyncStore,
        store_path: PathBuf,
        listen_port: u16,
        hooks: SyncHooks,
    ) -> Arc<Self> {
        Arc::new(SyncService {
            store: Mutex::new(store),
            store_path,
            listen_port,
            is_pairing: AtomicBool::new(false),
            last_received: Mutex::new(None),
            hooks,
        })
    }

    pub async fn listen(self: Arc<Self>, listener: TcpListener) {
        transport::serve(
            listener,
            Arc::new(move |frame, stream, addr| {
                let service = self.clone();
                Box::pin(async move { service.handle_connection(frame, stream, addr).await })
            }),
        )
        .await;
    }

    /// Send a local copy to every paired device with a known address.
    /// Returns how many devices it reached.
    pub async fn broadcast(&self, clip: ClipPayload) -> usize {
        // take(): the guard suppresses only the echo itself, so copying the
        // same text again later still syncs.
        let is_echo = lock(&self.last_received).take().as_deref() == Some(clip.text.as_str());
        if is_echo {
            return 0;
        }

        let mut sends = JoinSet::new();
        {
            let store = lock(&self.store);
            for peer in &store.peers {
                let Some(addr) = peer.last_addr.clone() else {
                    continue;
                };
                let frame = match Frame::seal_clip(&store.device_id, &peer.key, &clip) {
                    Ok(frame) => frame,
                    Err(e) => {
                        eprintln!("Could not seal clip for {}: {e}", peer.id);
                        continue;
                    }
                };
                let peer_id = peer.id.clone();
                sends.spawn(async move {
                    let result = transport::send_frame(&addr, &frame).await;
                    if let Err(e) = &result {
                        eprintln!("Could not reach {peer_id} at {addr}: {e}");
                    }
                    result.is_ok()
                });
            }
        }

        sends
            .join_all()
            .await
            .into_iter()
            .filter(|reached| *reached)
            .count()
    }

    /// Pair with the device listening at `addr`. That device shows a code;
    /// `code` delivers what the user typed here.
    pub async fn pair_with(
        &self,
        addr: &str,
        code: oneshot::Receiver<String>,
    ) -> Result<Peer, PairError> {
        let _pairing = self.begin_pairing().ok_or(PairError::Busy)?;
        let mut stream = transport::connect(addr).await?;
        let peer_ip = stream.peer_addr()?.ip();

        let (id, name) = self.identity();
        let me = self.local_device(&id, &name);
        let result = pairing::initiate(&mut stream, &me, peer_ip, code).await;
        self.finish_pairing(result)
    }

    async fn handle_connection(&self, frame: Frame, mut stream: TcpStream, addr: SocketAddr) {
        match frame {
            Frame::Clip { .. } => self.handle_clip(frame, addr),
            Frame::PairRequest { from } => {
                let Some(_pairing) = self.begin_pairing() else {
                    eprintln!("Ignored pairing request from {addr}: already pairing");
                    return;
                };
                let (id, name) = self.identity();
                let me = self.local_device(&id, &name);
                let on_pairing = self.hooks.on_pairing.clone();
                let result = pairing::respond(&mut stream, &me, from, addr.ip(), |code| {
                    on_pairing(PairingEvent::CodeShown { code })
                })
                .await;
                // Errors are already reported to the UI through the hook.
                let _ = self.finish_pairing(result);
            }
            _ => eprintln!("Dropped unexpected first frame from {addr}"),
        }
    }

    fn handle_clip(&self, frame: Frame, from_addr: SocketAddr) {
        let Frame::Clip { from, .. } = &frame else {
            return;
        };

        let key = lock(&self.store).peer(from).map(|peer| peer.key);
        let Some(key) = key else {
            eprintln!("Dropped clip from unpaired device at {from_addr}");
            return;
        };

        match frame.open_clip(&key) {
            Ok(clip) => {
                *lock(&self.last_received) = Some(clip.text.clone());
                (self.hooks.apply_clip)(clip);
            }
            Err(e) => eprintln!("Dropped clip from {from} at {from_addr}: {e}"),
        }
    }

    /// Persist a successful pairing and report the outcome to the UI.
    fn finish_pairing(&self, result: Result<Peer, PairError>) -> Result<Peer, PairError> {
        let outcome = result.and_then(|peer| {
            let mut store = lock(&self.store);
            // Save before going live, so a failed write can't leave a pairing
            // that silently disappears on restart.
            let mut updated = store.clone();
            updated.upsert_peer(peer.clone());
            updated
                .save(&self.store_path)
                .map_err(|e| PairError::Io(std::io::Error::other(e.to_string())))?;
            *store = updated;
            Ok(peer)
        });

        (self.hooks.on_pairing)(match &outcome {
            Ok(peer) => PairingEvent::Paired {
                peer_id: peer.id.clone(),
                name: peer.name.clone(),
            },
            Err(e) => PairingEvent::Failed {
                reason: e.to_string(),
            },
        });
        outcome
    }

    fn begin_pairing(&self) -> Option<PairingGuard<'_>> {
        if self.is_pairing.swap(true, Ordering::SeqCst) {
            None
        } else {
            Some(PairingGuard(&self.is_pairing))
        }
    }

    fn identity(&self) -> (String, String) {
        let store = lock(&self.store);
        (store.device_id.clone(), store.device_name.clone())
    }

    fn local_device<'a>(&self, id: &'a str, name: &'a str) -> LocalDevice<'a> {
        LocalDevice {
            id,
            name,
            listen_port: self.listen_port,
        }
    }
}

/// Clears the in-progress flag however pairing ends, including early returns.
struct PairingGuard<'a>(&'a AtomicBool);

impl Drop for PairingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// A poisoned lock only means another thread panicked mid-update; the data is
/// still the best we have, so keep going rather than take sync down.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Whether the user turned sync on. `save_setting` stores values as strings.
pub fn is_enabled(app_data_dir: &Path) -> bool {
    std::fs::read_to_string(app_data_dir.join("settings.json"))
        .ok()
        .and_then(|contents| serde_json::from_str::<serde_json::Value>(&contents).ok())
        .and_then(|settings| settings.get("syncEnabled").cloned())
        .is_some_and(|value| value == "true" || value == true)
}

/// Load this device's identity, start listening, and register the service so
/// the clipboard monitor and commands can use it.
pub fn start(app: &AppHandle, app_data_dir: &Path) -> Result<(), String> {
    let store_path = app_data_dir.join(STORE_FILE_NAME);
    let store = SyncStore::load_or_create(&store_path).map_err(|e| e.to_string())?;

    let event_app = app.clone();
    let hooks = SyncHooks {
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
    };
    let service = SyncService::new(store, store_path, SYNC_PORT, hooks);
    app.manage(service.clone());

    tauri::async_runtime::spawn(async move {
        match TcpListener::bind(("0.0.0.0", SYNC_PORT)).await {
            Ok(listener) => {
                println!("Sync listening on port {SYNC_PORT}");
                service.listen(listener).await;
            }
            Err(e) => eprintln!("Sync could not listen on port {SYNC_PORT}: {e}"),
        }
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::sync::mpsc;

    use super::*;
    use crate::sync::PeerKey;

    const KEY: PeerKey = [1; 32];
    const A_ID: &str = "11111111-1111-4111-8111-111111111111";
    const B_ID: &str = "22222222-2222-4222-8222-222222222222";

    struct Device {
        service: Arc<SyncService>,
        received: mpsc::UnboundedReceiver<ClipPayload>,
        events: mpsc::UnboundedReceiver<PairingEvent>,
        addr: SocketAddr,
        store_path: PathBuf,
        _dir: tempfile::TempDir,
    }

    fn clip(text: &str) -> ClipPayload {
        ClipPayload {
            text: text.into(),
            content_type: "text".into(),
        }
    }

    fn peer(id: &str, addr: SocketAddr) -> Peer {
        Peer {
            id: id.into(),
            name: id.into(),
            key: KEY,
            last_addr: Some(addr.to_string()),
        }
    }

    /// Start a device listening on a random local port, paired with `peers`.
    async fn device(id: &str, listener: TcpListener, peers: Vec<Peer>) -> Device {
        let addr = listener.local_addr().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join(STORE_FILE_NAME);
        let (clip_tx, received) = mpsc::unbounded_channel();
        let (event_tx, events) = mpsc::unbounded_channel();
        let store = SyncStore {
            device_id: id.into(),
            device_name: format!("{id} name"),
            peers,
        };
        let hooks = SyncHooks {
            apply_clip: Arc::new(move |clip| {
                let _ = clip_tx.send(clip);
            }),
            on_pairing: Arc::new(move |event| {
                let _ = event_tx.send(event);
            }),
        };
        let service = SyncService::new(store, store_path.clone(), addr.port(), hooks);
        tokio::spawn(service.clone().listen(listener));
        Device {
            service,
            received,
            events,
            addr,
            store_path,
            _dir: dir,
        }
    }

    async fn unpaired_device(id: &str) -> Device {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        device(id, listener, vec![]).await
    }

    /// Two devices, "a" and "b", paired with each other.
    async fn paired_devices() -> (Device, Device) {
        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_a = listener_a.local_addr().unwrap();
        let addr_b = listener_b.local_addr().unwrap();

        let a = device("a", listener_a, vec![peer("b", addr_b)]).await;
        let b = device("b", listener_b, vec![peer("a", addr_a)]).await;
        (a, b)
    }

    async fn next_clip(device: &mut Device) -> Option<ClipPayload> {
        tokio::time::timeout(Duration::from_millis(500), device.received.recv())
            .await
            .ok()
            .flatten()
    }

    async fn next_event(device: &mut Device) -> PairingEvent {
        tokio::time::timeout(Duration::from_secs(5), device.events.recv())
            .await
            .expect("no pairing event in time")
            .expect("event channel closed")
    }

    /// Pair `a` with `b`, typing `typed(code b shows)` on `a`. Waits until `b`
    /// has finished too, and returns `a`'s result.
    async fn pair(
        a: &Device,
        b: &mut Device,
        typed: impl FnOnce(&str) -> String,
    ) -> Result<Peer, PairError> {
        let (code_tx, code_rx) = oneshot::channel();
        let service = a.service.clone();
        let addr = b.addr.to_string();
        let pairing = tokio::spawn(async move { service.pair_with(&addr, code_rx).await });

        let PairingEvent::CodeShown { code } = next_event(b).await else {
            panic!("expected the responder to show a code");
        };
        code_tx.send(typed(&code)).unwrap();
        let result = pairing.await.unwrap();
        next_event(b).await; // b's Paired / Failed
        result
    }

    #[tokio::test]
    async fn clip_sent_by_one_device_arrives_at_the_other() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast(clip("hello b")).await;

        assert_eq!(next_clip(&mut b).await, Some(clip("hello b")));
    }

    #[tokio::test]
    async fn broadcast_reports_the_devices_it_reached() {
        let (a, _b) = paired_devices().await;
        assert_eq!(a.service.broadcast(clip("hello b")).await, 1);
    }

    #[tokio::test]
    async fn received_clip_is_not_sent_back() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast(clip("ping")).await;
        next_clip(&mut b).await;

        assert_eq!(b.service.broadcast(clip("ping")).await, 0);
    }

    #[tokio::test]
    async fn same_text_syncs_again_after_the_echo_is_suppressed() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast(clip("ping")).await;
        next_clip(&mut b).await;
        b.service.broadcast(clip("ping")).await;

        assert_eq!(b.service.broadcast(clip("ping")).await, 1);
    }

    #[tokio::test]
    async fn clip_from_an_unpaired_device_is_dropped() {
        let (_a, mut b) = paired_devices().await;
        // Even holding the right key, a device b never paired with is ignored.
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stranger = device("stranger", listener, vec![peer("b", b.addr)]).await;
        stranger.service.broadcast(clip("let me in")).await;

        assert_eq!(next_clip(&mut b).await, None);
    }

    #[tokio::test]
    async fn unreachable_device_is_skipped() {
        let closed = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let lonely = device("a", listener, vec![peer("gone", closed)]).await;

        assert_eq!(lonely.service.broadcast(clip("anyone?")).await, 0);
    }

    #[tokio::test]
    async fn newly_paired_devices_can_sync() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        pair(&a, &mut b, str::to_string).await.unwrap();
        a.service.broadcast(clip("first synced clip")).await;

        assert_eq!(next_clip(&mut b).await, Some(clip("first synced clip")));
    }

    #[tokio::test]
    async fn newly_paired_devices_sync_the_other_way_too() {
        let mut a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        pair(&a, &mut b, str::to_string).await.unwrap();
        b.service.broadcast(clip("back to a")).await;

        assert_eq!(next_clip(&mut a).await, Some(clip("back to a")));
    }

    #[tokio::test]
    async fn pairing_is_saved_to_disk() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        pair(&a, &mut b, str::to_string).await.unwrap();
        let saved = SyncStore::load_or_create(&b.store_path).unwrap();

        assert!(saved.peer(A_ID).is_some());
    }

    #[tokio::test]
    async fn responder_reports_the_new_peer_by_name() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        let (code_tx, code_rx) = oneshot::channel();
        let (service, addr) = (a.service.clone(), b.addr.to_string());
        tokio::spawn(async move { service.pair_with(&addr, code_rx).await });
        let PairingEvent::CodeShown { code } = next_event(&mut b).await else {
            panic!("expected a code");
        };
        code_tx.send(code).unwrap();

        assert_eq!(
            next_event(&mut b).await,
            PairingEvent::Paired {
                peer_id: A_ID.into(),
                name: format!("{A_ID} name"),
            }
        );
    }

    #[tokio::test]
    async fn wrong_code_is_reported_to_the_initiator() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        let result = pair(&a, &mut b, |_| "000000x".into()).await;

        assert!(matches!(result, Err(PairError::WrongCode)));
    }

    #[tokio::test]
    async fn wrong_code_stores_no_peer() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        let _ = pair(&a, &mut b, |_| "000000x".into()).await;

        assert!(lock(&b.service.store).peers.is_empty());
    }

    #[tokio::test]
    async fn second_pairing_at_once_is_refused() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        let (_code_tx, code_rx) = oneshot::channel();
        let (service, addr) = (a.service.clone(), b.addr.to_string());
        tokio::spawn(async move { service.pair_with(&addr, code_rx).await });
        next_event(&mut b).await; // first pairing is waiting for its code

        let (_tx, rx) = oneshot::channel();
        let second = a.service.pair_with(&b.addr.to_string(), rx).await;
        assert!(matches!(second, Err(PairError::Busy)));
    }

    #[test]
    fn sync_is_off_without_a_setting() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_enabled(dir.path()));
    }

    #[test]
    fn sync_is_on_when_the_setting_is_the_string_true() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.json"), r#"{"syncEnabled":"true"}"#).unwrap();
        assert!(is_enabled(dir.path()));
    }
}
