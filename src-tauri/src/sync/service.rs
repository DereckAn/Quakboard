//! The running sync service: receives clips from paired devices, sends local
//! copies to them, and pairs new devices.

use std::{
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinSet,
};

use super::{
    body::{read_body, write_body, IMAGE_BODY_LIMITS, MAX_IMAGE_BYTES},
    image::{decode_png, open_image, seal_image, ImageDetails, ImageMeta},
    pairing::{self, LocalDevice, PairError},
    store::{Peer, StoreError, SyncStore},
    transport, ClipPayload, Frame, PeerKey, SyncError,
};

/// How long after receiving an image the monitor may still see our own write
/// of it. Duplicate clipboard events arrive within a second; this leaves room
/// for slow machines without blocking a deliberate re-copy for long.
const IMAGE_ECHO_WINDOW: Duration = Duration::from_secs(5);

/// Puts a received clip on the system clipboard.
pub type ApplyClip = Arc<dyn Fn(ClipPayload) + Send + Sync>;
/// Stores a received, verified image and puts it on the system clipboard.
pub type ApplyImage = Arc<dyn Fn(ReceivedImage) + Send + Sync>;
/// Tells the UI what pairing is doing.
pub type OnPairing = Arc<dyn Fn(PairingEvent) + Send + Sync>;
/// Looks a device up on the network by id (mDNS), for when its last address fails.
pub type ResolveAddr = Arc<dyn Fn(&str) -> Option<SocketAddr> + Send + Sync>;

/// The service's effects on the outside world, injected so tests don't touch
/// the real clipboard, UI or network discovery.
pub struct SyncHooks {
    pub apply_clip: ApplyClip,
    pub apply_image: ApplyImage,
    pub on_pairing: OnPairing,
    pub resolve_addr: ResolveAddr,
}

/// An image from a paired device that decrypted, matched its header and
/// decoded safely.
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedImage {
    pub png: Vec<u8>,
    pub meta: ImageMeta,
    /// See `image::pixel_hash`; the echo guard compares it.
    pub pixel_hash: String,
    /// The sender's name as stored at pairing, for "Synced from …".
    pub from_name: String,
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
    accepts_pairing: AtomicBool,
    /// Text of the last received clip. Writing it to the clipboard makes the
    /// monitor see it as a new copy; this stops that one copy being sent back.
    last_received: Mutex<Option<String>>,
    /// Pixel hash of the last received image and when it arrived. See
    /// `take_image_echo`.
    last_received_image: Mutex<Option<(String, Instant)>>,
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
            accepts_pairing: AtomicBool::new(false),
            last_received: Mutex::new(None),
            last_received_image: Mutex::new(None),
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

    /// Send a local copy to every paired device. Tries each device's last
    /// address first and falls back to looking it up on the network, so mDNS
    /// only does real work when a device's IP changed. Returns how many
    /// devices it reached.
    pub async fn broadcast(&self, clip: ClipPayload) -> usize {
        // take(): the guard suppresses only the echo itself, so copying the
        // same text again later still syncs.
        let is_echo = lock(&self.last_received).take().as_deref() == Some(clip.text.as_str());
        if is_echo {
            return 0;
        }
        self.send_to_peers(|device_id, key| {
            Ok(Outgoing {
                frame: Frame::seal_clip(device_id, key, &clip)?,
                body: None,
            })
        })
        .await
    }

    /// Send a copied image to every paired device, like `broadcast`. The
    /// monitor checks `take_image_echo` first, before even storing it.
    pub async fn broadcast_image(&self, png: &[u8], details: ImageDetails) -> usize {
        if png.len() > MAX_IMAGE_BYTES {
            eprintln!(
                "Not syncing a {} MB image: the limit is {} MB",
                png.len() / (1024 * 1024),
                MAX_IMAGE_BYTES / (1024 * 1024)
            );
            return 0;
        }
        self.send_to_peers(|device_id, key| {
            let (frame, body) = seal_image(device_id, key, details, png)?;
            Ok(Outgoing {
                frame,
                body: Some(body),
            })
        })
        .await
    }

    /// Whether an image the monitor just saw on the clipboard is one this
    /// device wrote there after receiving it. Consumed on a match, and only
    /// counts for a few seconds: when the monitor's skip works, it never sees
    /// the write, and a later copy of the same picture by the user must sync.
    pub fn take_image_echo(&self, pixel_hash: &str) -> bool {
        self.take_image_echo_at(pixel_hash, Instant::now())
    }

    /// `take_image_echo` as of `now`, so the expiry can be tested exactly.
    fn take_image_echo_at(&self, pixel_hash: &str, now: Instant) -> bool {
        let mut last = lock(&self.last_received_image);
        let is_echo = last.as_ref().is_some_and(|(hash, received_at)| {
            hash == pixel_hash && now.duration_since(*received_at) < IMAGE_ECHO_WINDOW
        });
        if is_echo {
            *last = None;
        }
        is_echo
    }

    /// Seal once per paired device (fresh nonce each) and deliver to all of
    /// them at once. Returns how many devices it reached.
    async fn send_to_peers(
        &self,
        seal: impl Fn(&str, &PeerKey) -> Result<Outgoing, SyncError>,
    ) -> usize {
        let mut sends = JoinSet::new();
        {
            let store = lock(&self.store);
            for peer in &store.peers {
                let outgoing = match seal(&store.device_id, &peer.key) {
                    Ok(outgoing) => outgoing,
                    Err(e) => {
                        eprintln!("Could not seal for {}: {e}", peer.id);
                        continue;
                    }
                };
                let peer_id = peer.id.clone();
                let last_addr = peer.last_addr.clone();
                let resolve_addr = self.hooks.resolve_addr.clone();
                sends.spawn(async move {
                    let delivery = deliver(&peer_id, last_addr, &outgoing, &resolve_addr).await;
                    (peer_id, delivery)
                });
            }
        }

        let mut reached = 0;
        for (peer_id, delivery) in sends.join_all().await {
            match delivery {
                Delivery::Reached => reached += 1,
                Delivery::ReachedAt(addr) => {
                    reached += 1;
                    self.remember_addr(&peer_id, &addr);
                }
                Delivery::Missed => {}
            }
        }
        reached
    }

    /// A paired device reached us from `ip`. Keep its listening port from
    /// pairing: the connection's source port is ephemeral.
    fn learn_ip(&self, peer_id: &str, ip: IpAddr) {
        let port = lock(&self.store)
            .peer(peer_id)
            .and_then(|peer| peer.last_addr.as_deref()?.parse::<SocketAddr>().ok())
            .map(|addr| addr.port());
        if let Some(port) = port {
            self.remember_addr(peer_id, &SocketAddr::new(ip, port).to_string());
        }
    }

    /// Record where a paired device can be reached now, and persist it.
    fn remember_addr(&self, peer_id: &str, addr: &str) {
        let mut store = lock(&self.store);
        if store.peer(peer_id).and_then(|p| p.last_addr.as_deref()) == Some(addr) {
            return;
        }
        let mut updated = store.clone();
        updated.set_last_addr(peer_id, addr);
        match updated.save(&self.store_path) {
            Ok(()) => {
                *store = updated;
                println!("Sync now reaches {peer_id} at {addr}");
            }
            // Still usable this session via the lookup; retried next time.
            Err(e) => eprintln!("Could not save new address for {peer_id}: {e}"),
        }
    }

    /// Pair with the device listening at `addr`. That device shows a code;
    /// `code` delivers what the user typed here.
    pub async fn pair_with(
        &self,
        addr: &str,
        code: oneshot::Receiver<String>,
    ) -> Result<Peer, PairError> {
        let Some(_pairing) = self.begin_pairing() else {
            return self.finish_pairing(Err(PairError::Busy));
        };
        let result = async {
            let mut stream = transport::connect(addr).await?;
            let peer_ip = stream.peer_addr()?.ip();
            let (id, name) = self.identity();
            let me = self.local_device(&id, &name);
            pairing::initiate(&mut stream, &me, peer_ip, code).await
        }
        .await;
        self.finish_pairing(result)
    }

    /// Only answer pairing requests while the user has the Devices screen
    /// open, so nobody on the LAN can pop up a code prompt at other times.
    pub fn set_accepting_pairing(&self, accepting: bool) {
        self.accepts_pairing.store(accepting, Ordering::SeqCst);
    }

    pub fn peers(&self) -> Vec<Peer> {
        lock(&self.store).peers.clone()
    }

    /// Forget a paired device. Returns whether it was paired.
    pub fn unpair(&self, peer_id: &str) -> Result<bool, StoreError> {
        let mut store = lock(&self.store);
        let mut updated = store.clone();
        if !updated.remove_peer(peer_id) {
            return Ok(false);
        }
        // Save before going live, like pairing, so disk and memory agree.
        updated.save(&self.store_path)?;
        *store = updated;
        Ok(true)
    }

    /// This device's (id, name).
    pub fn identity(&self) -> (String, String) {
        let store = lock(&self.store);
        (store.device_id.clone(), store.device_name.clone())
    }

    async fn handle_connection(&self, frame: Frame, mut stream: TcpStream, addr: SocketAddr) {
        match frame {
            Frame::Clip { .. } => self.handle_clip(frame, addr),
            Frame::Image { .. } => self.handle_image(frame, &mut stream, addr).await,
            Frame::PairRequest { from } => {
                if !self.accepts_pairing.load(Ordering::SeqCst) {
                    eprintln!("Ignored pairing request from {addr}: Devices screen not open");
                    return;
                }
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

    async fn handle_image(&self, frame: Frame, stream: &mut TcpStream, from_addr: SocketAddr) {
        let Frame::Image { from, .. } = &frame else {
            return;
        };
        // Check the sender before reading the body, so an unpaired device
        // can't make us download 20 MB just to throw it away.
        let peer = lock(&self.store).peer(from).map(|p| (p.key, p.name.clone()));
        let Some((key, from_name)) = peer else {
            eprintln!("Dropped image from unpaired device at {from_addr}");
            return;
        };

        let body = match read_body(stream, IMAGE_BODY_LIMITS).await {
            Ok(body) => body,
            Err(e) => {
                eprintln!("Dropped image from {from} at {from_addr}: {e}");
                return;
            }
        };
        let (meta, png) = match open_image(&frame, &body, &key) {
            Ok(opened) => opened,
            Err(e) => {
                eprintln!("Dropped image from {from} at {from_addr}: {e}");
                return;
            }
        };
        drop(body);

        // Decoding a large image takes real CPU; keep it off the async workers.
        let decoded = tokio::task::spawn_blocking(move || {
            decode_png(&png, &meta).map(|decoded| (png, meta, decoded))
        })
        .await;
        let (png, meta, decoded) = match decoded {
            Ok(Ok(decoded)) => decoded,
            Ok(Err(e)) => {
                eprintln!("Dropped image from {from} at {from_addr}: {e}");
                return;
            }
            Err(e) => {
                eprintln!("Image decoding for {from} failed to run: {e}");
                return;
            }
        };

        self.learn_ip(from, from_addr.ip());
        *lock(&self.last_received_image) = Some((decoded.pixel_hash.clone(), Instant::now()));
        (self.hooks.apply_image)(ReceivedImage {
            png,
            meta,
            pixel_hash: decoded.pixel_hash,
            from_name,
        });
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
                // Decrypting proved the sender, so its current IP is trustworthy.
                self.learn_ip(from, from_addr.ip());
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

    fn local_device<'a>(&self, id: &'a str, name: &'a str) -> LocalDevice<'a> {
        LocalDevice {
            id,
            name,
            listen_port: self.listen_port,
        }
    }
}

enum Delivery {
    /// Reached at the address we already had.
    Reached,
    /// Reached at a newly looked-up address, which should be remembered.
    ReachedAt(String),
    Missed,
}

/// A sealed frame, and for images the body that follows it.
struct Outgoing {
    frame: Frame,
    body: Option<Vec<u8>>,
}

async fn send(addr: &str, outgoing: &Outgoing) -> std::io::Result<()> {
    let Some(body) = &outgoing.body else {
        return transport::send_frame(addr, &outgoing.frame).await;
    };
    let mut stream = transport::connect(addr).await?;
    tokio::time::timeout(Duration::from_secs(5), transport::write_frame(&mut stream, &outgoing.frame))
        .await
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "send timed out"))??;
    write_body(&mut stream, body, IMAGE_BODY_LIMITS).await
}

async fn deliver(
    peer_id: &str,
    last_addr: Option<String>,
    outgoing: &Outgoing,
    resolve_addr: &ResolveAddr,
) -> Delivery {
    if let Some(addr) = &last_addr {
        match send(addr, outgoing).await {
            Ok(()) => return Delivery::Reached,
            Err(e) => eprintln!("Could not reach {peer_id} at {addr}: {e}"),
        }
    }

    let Some(found) = resolve_addr(peer_id).map(|addr| addr.to_string()) else {
        return Delivery::Missed;
    };
    if last_addr.as_deref() == Some(found.as_str()) {
        // Discovery agrees with the address that just failed; it's offline.
        return Delivery::Missed;
    }
    match send(&found, outgoing).await {
        Ok(()) => Delivery::ReachedAt(found),
        Err(e) => {
            eprintln!("Could not reach {peer_id} at {found} either: {e}");
            Delivery::Missed
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use tokio::sync::mpsc;

    use super::*;
    use crate::sync::{store::STORE_FILE_NAME, PeerKey};

    const KEY: PeerKey = [1; 32];
    const A_ID: &str = "11111111-1111-4111-8111-111111111111";
    const B_ID: &str = "22222222-2222-4222-8222-222222222222";

    struct Device {
        service: Arc<SyncService>,
        received: mpsc::UnboundedReceiver<ClipPayload>,
        images: mpsc::UnboundedReceiver<ReceivedImage>,
        events: mpsc::UnboundedReceiver<PairingEvent>,
        addr: SocketAddr,
        store_path: PathBuf,
        /// Stands in for mDNS: what this device's lookup finds, by device id.
        network: Arc<Mutex<HashMap<String, SocketAddr>>>,
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
        let (image_tx, images) = mpsc::unbounded_channel();
        let store = SyncStore {
            device_id: id.into(),
            device_name: format!("{id} name"),
            peers,
        };
        let network = Arc::new(Mutex::new(HashMap::new()));
        let lookup = Arc::clone(&network);
        let hooks = SyncHooks {
            apply_clip: Arc::new(move |clip| {
                let _ = clip_tx.send(clip);
            }),
            apply_image: Arc::new(move |image| {
                let _ = image_tx.send(image);
            }),
            on_pairing: Arc::new(move |event| {
                let _ = event_tx.send(event);
            }),
            resolve_addr: Arc::new(move |id| lock(&lookup).get(id).copied()),
        };
        let service = SyncService::new(store, store_path.clone(), addr.port(), hooks);
        // As if the Devices screen were open; tests for the closed case turn it off.
        service.set_accepting_pairing(true);
        tokio::spawn(service.clone().listen(listener));
        Device {
            service,
            received,
            images,
            events,
            addr,
            store_path,
            network,
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

    /// An address nothing listens on, like a device that moved away.
    async fn dead_addr() -> SocketAddr {
        TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap()
    }

    /// "a" knows "b" only at `b_last_addr`; "b" knows "a" correctly.
    async fn a_with_stale_address_for_b(b_last_addr: Option<SocketAddr>) -> (Device, Device) {
        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_a = listener_a.local_addr().unwrap();
        let stale_b = Peer {
            last_addr: b_last_addr.map(|addr| addr.to_string()),
            ..peer("b", addr_a)
        };
        let a = device("a", listener_a, vec![stale_b]).await;
        let b = device("b", listener_b, vec![peer("a", addr_a)]).await;
        (a, b)
    }

    #[tokio::test]
    async fn device_that_moved_is_reached_through_lookup() {
        let (a, mut b) = a_with_stale_address_for_b(Some(dead_addr().await)).await;
        lock(&a.network).insert("b".into(), b.addr);
        a.service.broadcast(clip("found you")).await;

        assert_eq!(next_clip(&mut b).await, Some(clip("found you")));
    }

    #[tokio::test]
    async fn device_without_an_address_is_reached_through_lookup() {
        let (a, mut b) = a_with_stale_address_for_b(None).await;
        lock(&a.network).insert("b".into(), b.addr);
        a.service.broadcast(clip("hello")).await;

        assert_eq!(next_clip(&mut b).await, Some(clip("hello")));
    }

    #[tokio::test]
    async fn looked_up_address_is_saved_for_next_time() {
        let (a, b) = a_with_stale_address_for_b(Some(dead_addr().await)).await;
        lock(&a.network).insert("b".into(), b.addr);
        a.service.broadcast(clip("found you")).await;
        let saved = SyncStore::load_or_create(&a.store_path).unwrap();

        assert_eq!(saved.peer("b").unwrap().last_addr, Some(b.addr.to_string()));
    }

    #[tokio::test]
    async fn device_missing_from_lookup_counts_as_unreached() {
        let (a, _b) = a_with_stale_address_for_b(Some(dead_addr().await)).await;
        assert_eq!(a.service.broadcast(clip("anyone?")).await, 0);
    }

    #[tokio::test]
    async fn clip_from_a_new_ip_updates_the_senders_address() {
        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (addr_a, addr_b) = (listener_a.local_addr().unwrap(), listener_b.local_addr().unwrap());
        // b remembers a at an old IP; a's port stays the same.
        let old_a = SocketAddr::new("10.9.9.9".parse().unwrap(), addr_a.port());
        let a = device("a", listener_a, vec![peer("b", addr_b)]).await;
        let mut b = device("b", listener_b, vec![peer("a", old_a)]).await;
        a.service.broadcast(clip("new place")).await;
        next_clip(&mut b).await;

        assert_eq!(
            lock(&b.service.store).peer("a").unwrap().last_addr,
            Some(addr_a.to_string())
        );
    }

    fn png_of(width: u32, height: u32) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(width, height, image::Rgba([10, 200, 60, 255]));
        let mut bytes = Vec::new();
        image::DynamicImage::from(image)
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    /// Send an image the way a paired device would: header frame, then body.
    async fn send_image(to: SocketAddr, from: &str, key: &PeerKey, width: u32, height: u32, png: &[u8]) {
        let details = crate::sync::image::ImageDetails {
            width,
            height,
            is_screenshot: false,
        };
        let (frame, body) = crate::sync::image::seal_image(from, key, details, png).unwrap();
        let mut stream = transport::connect(&to.to_string()).await.unwrap();
        transport::write_frame(&mut stream, &frame).await.unwrap();
        crate::sync::body::write_body(&mut stream, &body, IMAGE_BODY_LIMITS)
            .await
            .unwrap();
    }

    async fn next_image(device: &mut Device) -> Option<ReceivedImage> {
        tokio::time::timeout(Duration::from_millis(500), device.images.recv())
            .await
            .ok()
            .flatten()
    }

    #[tokio::test]
    async fn image_from_a_paired_device_is_applied() {
        let (_a, mut b) = paired_devices().await;
        send_image(b.addr, "a", &KEY, 4, 3, &png_of(4, 3)).await;

        assert_eq!(next_image(&mut b).await.map(|image| image.png), Some(png_of(4, 3)));
    }

    #[tokio::test]
    async fn applied_image_names_its_sender() {
        let (_a, mut b) = paired_devices().await;
        send_image(b.addr, "a", &KEY, 4, 3, &png_of(4, 3)).await;

        assert_eq!(next_image(&mut b).await.unwrap().from_name, "a");
    }

    #[tokio::test]
    async fn applied_image_carries_its_pixel_hash() {
        let (_a, mut b) = paired_devices().await;
        send_image(b.addr, "a", &KEY, 4, 3, &png_of(4, 3)).await;
        let pixels = image::RgbaImage::from_pixel(4, 3, image::Rgba([10, 200, 60, 255]));

        assert_eq!(
            next_image(&mut b).await.unwrap().pixel_hash,
            crate::sync::image::pixel_hash(4, 3, pixels.as_raw())
        );
    }

    #[tokio::test]
    async fn image_from_an_unpaired_device_is_dropped() {
        let (_a, mut b) = paired_devices().await;
        send_image(b.addr, "stranger", &KEY, 4, 3, &png_of(4, 3)).await;

        assert_eq!(next_image(&mut b).await, None);
    }

    #[tokio::test]
    async fn image_that_is_not_a_png_is_dropped() {
        let (_a, mut b) = paired_devices().await;
        send_image(b.addr, "a", &KEY, 4, 3, b"not a png at all").await;

        assert_eq!(next_image(&mut b).await, None);
    }

    #[tokio::test]
    async fn image_whose_size_differs_from_its_header_is_dropped() {
        let (_a, mut b) = paired_devices().await;
        send_image(b.addr, "a", &KEY, 40, 30, &png_of(4, 3)).await;

        assert_eq!(next_image(&mut b).await, None);
    }

    #[tokio::test]
    async fn received_image_teaches_the_senders_new_ip() {
        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_a = listener_a.local_addr().unwrap();
        let old_a = SocketAddr::new("10.9.9.9".parse().unwrap(), addr_a.port());
        let _a = device("a", listener_a, vec![]).await;
        let mut b = device("b", listener_b, vec![peer("a", old_a)]).await;
        send_image(b.addr, "a", &KEY, 4, 3, &png_of(4, 3)).await;
        next_image(&mut b).await;

        assert_eq!(
            lock(&b.service.store).peer("a").unwrap().last_addr,
            Some(addr_a.to_string())
        );
    }

    fn details(width: u32, height: u32) -> ImageDetails {
        ImageDetails {
            width,
            height,
            is_screenshot: false,
        }
    }

    #[tokio::test]
    async fn broadcast_image_arrives_at_the_other_device() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;

        assert_eq!(next_image(&mut b).await.map(|image| image.png), Some(png_of(4, 3)));
    }

    #[tokio::test]
    async fn broadcast_image_reports_the_devices_it_reached() {
        let (a, _b) = paired_devices().await;
        assert_eq!(a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await, 1);
    }

    #[tokio::test]
    async fn oversized_image_is_not_sent() {
        let (a, _b) = paired_devices().await;
        let too_big = vec![0u8; MAX_IMAGE_BYTES + 1];

        assert_eq!(a.service.broadcast_image(&too_big, details(4, 3)).await, 0);
    }

    #[tokio::test]
    async fn image_reaches_a_device_that_moved_through_lookup() {
        let (a, mut b) = a_with_stale_address_for_b(Some(dead_addr().await)).await;
        lock(&a.network).insert("b".into(), b.addr);
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;

        assert_eq!(next_image(&mut b).await.map(|image| image.png), Some(png_of(4, 3)));
    }

    #[tokio::test]
    async fn received_image_is_recognized_as_an_echo() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;
        let received = next_image(&mut b).await.unwrap();

        assert!(b.service.take_image_echo(&received.pixel_hash));
    }

    #[tokio::test]
    async fn image_echo_is_only_suppressed_once() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;
        let received = next_image(&mut b).await.unwrap();
        b.service.take_image_echo(&received.pixel_hash);

        assert!(!b.service.take_image_echo(&received.pixel_hash));
    }

    #[tokio::test]
    async fn a_different_image_is_not_an_echo() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;
        next_image(&mut b).await.unwrap();
        let other = image::RgbaImage::from_pixel(5, 5, image::Rgba([1, 2, 3, 255]));

        assert!(!b
            .service
            .take_image_echo(&crate::sync::image::pixel_hash(5, 5, other.as_raw())));
    }

    #[tokio::test]
    async fn image_echo_expires() {
        // When the monitor's skip works it never sees our write, so a copy of
        // the same picture later on must not be mistaken for an echo.
        let (a, mut b) = paired_devices().await;
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;
        let received = next_image(&mut b).await.unwrap();
        let later = Instant::now() + IMAGE_ECHO_WINDOW;

        assert!(!b.service.take_image_echo_at(&received.pixel_hash, later));
    }

    #[tokio::test]
    async fn text_and_image_echo_guards_are_independent() {
        let (a, mut b) = paired_devices().await;
        a.service.broadcast(clip("ping")).await;
        next_clip(&mut b).await;
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;
        next_image(&mut b).await;

        assert_eq!(b.service.broadcast(clip("ping")).await, 0);
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
    async fn pairing_request_is_ignored_while_devices_screen_is_closed() {
        let a = unpaired_device(A_ID).await;
        let b = unpaired_device(B_ID).await;
        b.service.set_accepting_pairing(false);
        let (_code_tx, code_rx) = oneshot::channel();
        let result = a.service.pair_with(&b.addr.to_string(), code_rx).await;

        assert!(matches!(result, Err(PairError::Io(_))));
    }

    #[tokio::test]
    async fn unpairing_forgets_the_device_on_disk() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        pair(&a, &mut b, str::to_string).await.unwrap();
        b.service.unpair(A_ID).unwrap();

        assert!(SyncStore::load_or_create(&b.store_path).unwrap().peers.is_empty());
    }

    #[tokio::test]
    async fn unpaired_device_can_no_longer_send_clips() {
        let a = unpaired_device(A_ID).await;
        let mut b = unpaired_device(B_ID).await;
        pair(&a, &mut b, str::to_string).await.unwrap();
        b.service.unpair(A_ID).unwrap();
        a.service.broadcast(clip("still there?")).await;

        assert_eq!(next_clip(&mut b).await, None);
    }

    #[test]
    fn unpairing_an_unknown_device_reports_false() {
        let dir = tempfile::tempdir().unwrap();
        let hooks = SyncHooks {
            apply_clip: Arc::new(|_| {}),
            apply_image: Arc::new(|_| {}),
            on_pairing: Arc::new(|_| {}),
            resolve_addr: Arc::new(|_| None),
        };
        let service = SyncService::new(
            SyncStore::new("desk".into()),
            dir.path().join(STORE_FILE_NAME),
            0,
            hooks,
        );

        assert!(!service.unpair(A_ID).unwrap());
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
}
