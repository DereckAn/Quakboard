//! The running sync service: receives clips from paired devices, sends local
//! copies to them, and pairs new devices.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use serde::Serialize;
use tokio::{
    io::AsyncReadExt,
    net::{TcpListener, TcpStream},
    sync::oneshot,
    task::JoinSet,
};

use super::{
    body::{read_body, write_body, IMAGE_BODY_LIMITS, MAX_IMAGE_BYTES},
    image::{decode_png, open_image, seal_image, ImageDetails, ImageMeta},
    fetch::{
        decide, encode_salt, open_fetch_request, seal_fetch_reply, stream_context, FetchReply,
        RefusalReason, FILE_IDLE_TIMEOUT,
    },
    fetching::{fetch_into, FetchError, RemoteFile},
    offers::{hash_file, open_file_offer, seal_file_offer, FileOfferInfo, Offer},
    stream::{new_salt, send_stream},
    remote_files::validate_offer,
    pairing::{self, LocalDevice, PairError},
    protocol::{Incompatible, Protocol},
    store::{Peer, StoreError, SyncStore},
    transport::{self, Incoming},
    ClipPayload, Frame, PeerKey, SyncError,
};

/// How long after receiving an image the monitor may still see our own write
/// of it. Duplicate clipboard events arrive within a second; this leaves room
/// for slow machines without blocking a deliberate re-copy for long.
const IMAGE_ECHO_WINDOW: Duration = Duration::from_secs(5);

/// Puts a received clip on the system clipboard.
pub type ApplyClip = Arc<dyn Fn(ClipPayload) + Send + Sync>;
/// Stores a received, verified image and puts it on the system clipboard.
pub type ApplyImage = Arc<dyn Fn(ReceivedImage) + Send + Sync>;
/// Records a file another device offered, as a remote item in history.
pub type ApplyOffer = Arc<dyn Fn(ReceivedOffer) + Send + Sync>;
/// Finds a file this device offered, by offer id. Blocking (database).
pub type LookupOffer = Arc<dyn Fn(&str) -> Option<Offer> + Send + Sync>;
/// Tells the UI what pairing is doing.
pub type OnPairing = Arc<dyn Fn(PairingEvent) + Send + Sync>;
/// Looks a device up on the network by id (mDNS), for when its last address fails.
pub type ResolveAddr = Arc<dyn Fn(&str) -> Option<SocketAddr> + Send + Sync>;

/// The service's effects on the outside world, injected so tests don't touch
/// the real clipboard, UI or network discovery.
pub struct SyncHooks {
    pub apply_clip: ApplyClip,
    pub apply_image: ApplyImage,
    pub apply_offer: ApplyOffer,
    pub lookup_offer: LookupOffer,
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

/// A file offer from a paired device, already checked (`validate_offer`).
#[derive(Debug, Clone, PartialEq)]
pub struct ReceivedOffer {
    pub info: FileOfferInfo,
    pub from_id: String,
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
    Failed {
        reason: String,
        /// One of the devices needs a newer Quakboard. Worth showing even
        /// when no pairing dialog is open, unlike a cancel.
        needs_update: bool,
    },
}

pub struct SyncService {
    store: Mutex<SyncStore>,
    store_path: PathBuf,
    /// Sent to devices during pairing so they know where to reach us.
    listen_port: u16,
    /// One pairing at a time, so a device on the LAN can't stack up prompts.
    is_pairing: AtomicBool,
    accepts_pairing: AtomicBool,
    /// The user's "Also sync images" setting; covers sending and receiving.
    syncs_images: AtomicBool,
    /// Text of the last received clip. Writing it to the clipboard makes the
    /// monitor see it as a new copy; this stops that one copy being sent back.
    last_received: Mutex<Option<String>>,
    /// Pixel hash of the last received image and when it arrived. See
    /// `take_image_echo`.
    last_received_image: Mutex<Option<(String, Instant)>>,
    /// Paired devices whose last frame was refused as incompatible. Only
    /// shown to the user, never saved: the protocol stamp can be forged.
    incompatible: Mutex<HashMap<String, Incompatible>>,
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
            syncs_images: AtomicBool::new(true),
            last_received: Mutex::new(None),
            last_received_image: Mutex::new(None),
            incompatible: Mutex::new(HashMap::new()),
            hooks,
        })
    }

    pub async fn listen(self: Arc<Self>, listener: TcpListener) {
        transport::serve(
            listener,
            Arc::new(move |incoming, stream, addr| {
                let service = self.clone();
                Box::pin(async move { service.handle_connection(incoming, stream, addr).await })
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
        self.send_to_peers("text", None, |device_id, key| {
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
        if !self.is_syncing_images() {
            return 0;
        }
        if png.len() > MAX_IMAGE_BYTES {
            eprintln!(
                "Not syncing a {} MB image: the limit is {} MB",
                png.len() / (1024 * 1024),
                MAX_IMAGE_BYTES / (1024 * 1024)
            );
            return 0;
        }
        self.send_to_peers("image", None, |device_id, key| {
            let (frame, body) = seal_image(device_id, key, details, png)?;
            Ok(Outgoing {
                frame,
                body: Some(body),
            })
        })
        .await
    }

    /// Offer a file to the paired devices chosen in the Send picker.
    /// Returns how many of them were reached.
    pub async fn send_offer(&self, info: &FileOfferInfo, to: &[String]) -> usize {
        self.send_to_peers("a file offer", Some(to), |device_id, key| {
            Ok(Outgoing {
                frame: seal_file_offer(device_id, key, info)?,
                body: None,
            })
        })
        .await
    }

    /// Fetch a file another device offered, into `dest_dir`.
    pub async fn fetch_file(
        &self,
        remote: &RemoteFile,
        dest_dir: &Path,
        on_progress: impl FnMut(u64),
    ) -> Result<PathBuf, FetchError> {
        let peer = lock(&self.store)
            .peer(&remote.origin_id)
            .map(|p| (p.key, p.last_addr.clone()));
        let Some((key, last_addr)) = peer else {
            return Err(FetchError::Refused(RefusalReason::NotShared));
        };

        // Same fallback as clips: the last address, then a network lookup.
        let looked_up = (self.hooks.resolve_addr)(&remote.origin_id).map(|a| a.to_string());
        let mut stream = None;
        for addr in last_addr.iter().chain(looked_up.iter()) {
            if let Ok(connected) = transport::connect(addr).await {
                stream = Some(connected);
                break;
            }
        }
        let mut stream = stream.ok_or_else(|| FetchError::Offline(remote.origin_name.clone()))?;

        let (me, _) = self.identity();
        fetch_into(&mut stream, &me, &key, remote, dest_dir, on_progress).await
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
        kind: &str,
        only: Option<&[String]>,
        seal: impl Fn(&str, &PeerKey) -> Result<Outgoing, SyncError>,
    ) -> usize {
        let mut sends = JoinSet::new();
        let mut chosen = 0;
        {
            let store = lock(&self.store);
            let targets = store
                .peers
                .iter()
                .filter(|peer| only.is_none_or(|ids| ids.contains(&peer.id)));
            for peer in targets {
                chosen += 1;
                let outgoing = match seal(&store.device_id, &peer.key) {
                    Ok(outgoing) => outgoing,
                    Err(e) => {
                        eprintln!("Could not seal for {}: {e}", peer.name);
                        continue;
                    }
                };
                let peer_id = peer.id.clone();
                let peer_name = peer.name.clone();
                let last_addr = peer.last_addr.clone();
                let resolve_addr = self.hooks.resolve_addr.clone();
                sends.spawn(async move {
                    let delivery =
                        deliver(&peer_id, &peer_name, last_addr, &outgoing, &resolve_addr).await;
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
        // Counts only: clip contents can be passwords and never get logged.
        if chosen > 0 {
            println!("Sync sent {kind} to {reached} of {chosen} paired device(s)");
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
        let name = store.peer(peer_id).map_or_else(|| peer_id.to_string(), |p| p.name.clone());
        let mut updated = store.clone();
        updated.set_last_addr(peer_id, addr);
        match updated.save(&self.store_path) {
            Ok(()) => {
                *store = updated;
                println!("Sync now reaches {name} at {addr}");
            }
            // Still usable this session via the lookup; retried next time.
            Err(e) => eprintln!("Could not save new address for {name}: {e}"),
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

    pub fn set_syncing_images(&self, enabled: bool) {
        self.syncs_images.store(enabled, Ordering::SeqCst);
    }

    pub fn is_syncing_images(&self) -> bool {
        self.syncs_images.load(Ordering::SeqCst)
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

    /// Why the last frame from this paired device was refused, if it was.
    pub fn incompatibility(&self, peer_id: &str) -> Option<Incompatible> {
        lock(&self.incompatible).get(peer_id).copied()
    }

    async fn handle_connection(&self, incoming: Incoming, mut stream: TcpStream, addr: SocketAddr) {
        let (frame, protocol) = match incoming {
            Incoming::Frame(frame, protocol) => (frame, protocol),
            Incoming::Incompatible { sender, reason } => {
                self.note_compatibility(sender.as_deref(), Err(reason));
                eprintln!("Refused a sync frame from {addr}: {reason}");
                return;
            }
        };
        // Pairing answers before refusing, so the other device learns why.
        if !matches!(frame, Frame::PairRequest { .. }) {
            let verdict = Protocol::CURRENT.check(protocol);
            self.note_compatibility(frame.sender(), verdict);
            if let Err(refusal) = verdict {
                eprintln!("Refused a sync frame from {addr}: {refusal}");
                return;
            }
        }
        match frame {
            Frame::Clip { .. } => self.handle_clip(frame, addr),
            Frame::Image { .. } => self.handle_image(frame, &mut stream, addr).await,
            Frame::FileOffer { .. } => self.handle_offer(frame, addr),
            Frame::FileFetch { .. } => self.handle_fetch(frame, &mut stream, addr).await,
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
                let result =
                    pairing::respond(&mut stream, &me, from, protocol, addr.ip(), |code| {
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
        // Hang up before reading the body: no point downloading what the user
        // chose not to receive.
        if !self.is_syncing_images() {
            eprintln!("Ignored image from {from_addr}: image sync is off");
            return;
        }
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
                eprintln!("Dropped image from {from_name} at {from_addr}: {e}");
                return;
            }
        };
        let (meta, png) = match open_image(&frame, &body, &key) {
            Ok(opened) => opened,
            Err(e) => {
                eprintln!("Dropped image from {from_name} at {from_addr}: {e}");
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
                eprintln!("Dropped image from {from_name} at {from_addr}: {e}");
                return;
            }
            Err(e) => {
                eprintln!("Image decoding for {from_name} failed to run: {e}");
                return;
            }
        };

        self.learn_ip(from, from_addr.ip());
        *lock(&self.last_received_image) = Some((decoded.pixel_hash.clone(), Instant::now()));
        println!(
            "Sync received image {}x{} ({} KB) from {from_name}",
            meta.width,
            meta.height,
            meta.byte_len / 1024
        );
        (self.hooks.apply_image)(ReceivedImage {
            png,
            meta,
            pixel_hash: decoded.pixel_hash,
            from_name,
        });
    }

    /// Serve an offered file, after every check in `fetch::decide`.
    async fn handle_fetch(&self, frame: Frame, stream: &mut TcpStream, from_addr: SocketAddr) {
        let Frame::FileFetch { from, .. } = &frame else {
            return;
        };
        let peer = lock(&self.store).peer(from).map(|p| (p.key, p.name.clone()));
        let Some((key, from_name)) = peer else {
            eprintln!("Ignored file request from unpaired device at {from_addr}");
            return;
        };
        let request = match open_fetch_request(&frame, &key) {
            Ok(request) => request,
            Err(e) => {
                eprintln!("Dropped file request from {from_name} at {from_addr}: {e}");
                return;
            }
        };

        // Database lookup and a possible re-hash: keep both off the workers.
        let lookup_offer = self.hooks.lookup_offer.clone();
        let (offer_id, requester) = (request.offer_id.clone(), from.clone());
        let decision = tokio::task::spawn_blocking(move || {
            decide(lookup_offer(&offer_id), &requester, |path| hash_file(path, |_, _| {}))
        })
        .await;
        let decision = match decision {
            Ok(decision) => decision,
            Err(e) => {
                eprintln!("Checking a file request from {from_name} failed to run: {e}");
                return;
            }
        };

        let serving = match decision {
            Ok(offer) => match new_salt() {
                Ok(salt) => Some((offer, salt)),
                Err(e) => {
                    eprintln!("Could not start serving a file to {from_name}: {e}");
                    return;
                }
            },
            Err(reason) => {
                println!("Refused a file request from {from_name}: {reason:?}");
                let _ = self
                    .reply_to_fetch(stream, &key, &request.offer_id, &FetchReply::Refused { reason })
                    .await;
                None
            }
        };
        let Some((offer, salt)) = serving else {
            return;
        };

        let start = FetchReply::Start {
            salt: encode_salt(&salt),
            size: offer.stamp.size,
        };
        if let Err(e) = self.reply_to_fetch(stream, &key, &offer.offer_id, &start).await {
            eprintln!("Could not answer {from_name}'s file request: {e}");
            return;
        }

        let file = match tokio::fs::File::open(&offer.path).await {
            Ok(file) => file,
            Err(e) => {
                // The requester sees the stream end early and discards it.
                eprintln!("Could not open a file {from_name} asked for: {e}");
                return;
            }
        };
        // Never past the offered size: a file still being written would
        // otherwise stream forever (see `send_stream`).
        let mut source = file.take(offer.stamp.size);
        let (my_id, _) = self.identity();
        let context = stream_context(&offer.offer_id, &my_id);
        match send_stream(&mut source, stream, &key, &salt, &context, FILE_IDLE_TIMEOUT).await {
            Ok(sent) if sent.sha256 == offer.sha256 => {
                println!("Sync served a file ({} KB) to {from_name}", sent.byte_len / 1024);
            }
            Ok(_) => eprintln!("A file changed while serving it to {from_name}; they will reject it"),
            Err(e) => eprintln!("Serving a file to {from_name} failed: {e}"),
        }
    }

    async fn reply_to_fetch(
        &self,
        stream: &mut TcpStream,
        key: &PeerKey,
        offer_id: &str,
        reply: &FetchReply,
    ) -> std::io::Result<()> {
        let frame = seal_fetch_reply(key, offer_id, reply)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        tokio::time::timeout(Duration::from_secs(5), transport::write_frame(stream, &frame))
            .await
            .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "reply timed out"))?
    }

    fn handle_offer(&self, frame: Frame, from_addr: SocketAddr) {
        let Frame::FileOffer { from, .. } = &frame else {
            return;
        };
        let peer = lock(&self.store).peer(from).map(|p| (p.key, p.name.clone()));
        let Some((key, from_name)) = peer else {
            eprintln!("Dropped file offer from unpaired device at {from_addr}");
            return;
        };

        let info = open_file_offer(&frame, &key)
            .map_err(|e| e.to_string())
            .and_then(validate_offer);
        let info = match info {
            Ok(info) => info,
            Err(e) => {
                eprintln!("Dropped file offer from {from_name} at {from_addr}: {e}");
                return;
            }
        };

        self.learn_ip(from, from_addr.ip());
        // The size only: file names can be private.
        println!("Sync received a file offer ({} KB) from {from_name}", info.size / 1024);
        (self.hooks.apply_offer)(ReceivedOffer {
            info,
            from_id: from.clone(),
            from_name,
        });
    }

    fn handle_clip(&self, frame: Frame, from_addr: SocketAddr) {
        let Frame::Clip { from, .. } = &frame else {
            return;
        };

        let peer = lock(&self.store).peer(from).map(|p| (p.key, p.name.clone()));
        let Some((key, from_name)) = peer else {
            eprintln!("Dropped clip from unpaired device at {from_addr}");
            return;
        };

        match frame.open_clip(&key) {
            Ok(clip) => {
                *lock(&self.last_received) = Some(clip.text.clone());
                // Decrypting proved the sender, so its current IP is trustworthy.
                self.learn_ip(from, from_addr.ip());
                println!("Sync received text from {from_name}");
                (self.hooks.apply_clip)(clip);
            }
            Err(e) => eprintln!("Dropped clip from {from_name} at {from_addr}: {e}"),
        }
    }

    /// Persist a successful pairing and report the outcome to the UI.
    /// Remember whether a paired device's frames are being refused. Ids that
    /// aren't paired are ignored, so strangers can't grow the map.
    fn note_compatibility(&self, sender: Option<&str>, verdict: Result<(), Incompatible>) {
        let Some(id) = sender else {
            return;
        };
        if lock(&self.store).peer(id).is_none() {
            return;
        }
        let mut incompatible = lock(&self.incompatible);
        match verdict {
            Ok(()) => incompatible.remove(id),
            Err(refusal) => incompatible.insert(id.to_string(), refusal),
        };
    }

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
                needs_update: matches!(e, PairError::Incompatible(_)),
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
    peer_name: &str,
    last_addr: Option<String>,
    outgoing: &Outgoing,
    resolve_addr: &ResolveAddr,
) -> Delivery {
    if let Some(addr) = &last_addr {
        match send(addr, outgoing).await {
            Ok(()) => return Delivery::Reached,
            Err(e) => eprintln!("Could not reach {peer_name} at {addr}: {e}"),
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
            eprintln!("Could not reach {peer_name} at {found} either: {e}");
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

    use tokio::{io::AsyncWriteExt, sync::mpsc};

    use super::*;
    use crate::sync::{store::STORE_FILE_NAME, PeerKey};

    const KEY: PeerKey = [1; 32];
    const A_ID: &str = "11111111-1111-4111-8111-111111111111";
    const B_ID: &str = "22222222-2222-4222-8222-222222222222";

    struct Device {
        service: Arc<SyncService>,
        received: mpsc::UnboundedReceiver<ClipPayload>,
        images: mpsc::UnboundedReceiver<ReceivedImage>,
        offers: mpsc::UnboundedReceiver<ReceivedOffer>,
        /// Stands in for the offers table: what this device has shared.
        shared_files: Arc<Mutex<HashMap<String, Offer>>>,
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
        let (offer_tx, offers) = mpsc::unbounded_channel();
        let shared_files = Arc::new(Mutex::new(HashMap::new()));
        let shared_lookup = Arc::clone(&shared_files);
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
            apply_offer: Arc::new(move |offer| {
                let _ = offer_tx.send(offer);
            }),
            lookup_offer: Arc::new(move |id| lock(&shared_lookup).get(id).cloned()),
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
            offers,
            shared_files,
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

    /// A future device that no longer works with this build.
    const FUTURE: Protocol = Protocol {
        version: 2,
        min_peer: 2,
    };

    /// Send `frame` to `to` stamped as `FUTURE`. Returns the open connection.
    async fn send_from_the_future(to: SocketAddr, frame: &Frame) -> TcpStream {
        let mut stream = transport::connect(&to.to_string()).await.unwrap();
        transport::write_stamped(&mut stream, frame, FUTURE)
            .await
            .unwrap();
        stream
    }

    fn future_clip() -> Frame {
        Frame::seal_clip("a", &KEY, &clip("from the future")).unwrap()
    }

    #[tokio::test]
    async fn clip_from_a_device_we_cannot_work_with_is_dropped() {
        let (_a, mut b) = paired_devices().await;
        send_from_the_future(b.addr, &future_clip()).await;

        assert_eq!(next_clip(&mut b).await, None);
    }

    #[tokio::test]
    async fn paired_device_we_cannot_work_with_is_flagged() {
        let (_a, mut b) = paired_devices().await;
        send_from_the_future(b.addr, &future_clip()).await;
        next_clip(&mut b).await;

        assert_eq!(
            b.service.incompatibility("a"),
            Some(Incompatible::ThisTooOld)
        );
    }

    #[tokio::test]
    async fn compatible_frame_clears_the_flag() {
        let (a, mut b) = paired_devices().await;
        send_from_the_future(b.addr, &future_clip()).await;
        next_clip(&mut b).await;
        a.service.broadcast(clip("updated")).await;
        next_clip(&mut b).await;

        assert_eq!(b.service.incompatibility("a"), None);
    }

    /// A frame type this build can't parse, from a device it can't work with.
    async fn send_unknown_future_frame(to: SocketAddr, from: &str) {
        let body = format!(
            r#"{{"type":"ClipV2","from":"{from}","protocol":{{"version":2,"min_peer":2}}}}"#
        );
        let mut stream = transport::connect(&to.to_string()).await.unwrap();
        stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        stream.write_all(body.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn unknown_future_frame_applies_nothing() {
        let (_a, mut b) = paired_devices().await;
        send_unknown_future_frame(b.addr, "a").await;

        assert_eq!(next_clip(&mut b).await, None);
    }

    #[tokio::test]
    async fn unknown_future_frame_flags_its_paired_sender() {
        let (_a, mut b) = paired_devices().await;
        send_unknown_future_frame(b.addr, "a").await;
        next_clip(&mut b).await;

        assert_eq!(
            b.service.incompatibility("a"),
            Some(Incompatible::ThisTooOld)
        );
    }

    #[tokio::test]
    async fn unknown_future_frame_from_a_stranger_is_not_tracked() {
        let (_a, mut b) = paired_devices().await;
        send_unknown_future_frame(b.addr, "stranger").await;
        next_clip(&mut b).await;

        assert_eq!(b.service.incompatibility("stranger"), None);
    }

    #[tokio::test]
    async fn pairing_request_we_cannot_work_with_reports_an_update() {
        let mut b = unpaired_device("b").await;
        let request = Frame::PairRequest { from: A_ID.into() };
        let _connection = send_from_the_future(b.addr, &request).await;

        assert!(matches!(
            next_event(&mut b).await,
            PairingEvent::Failed {
                needs_update: true,
                ..
            }
        ));
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
    async fn no_image_is_sent_while_image_sync_is_off() {
        let (a, _b) = paired_devices().await;
        a.service.set_syncing_images(false);

        assert_eq!(a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await, 0);
    }

    #[tokio::test]
    async fn incoming_image_is_ignored_while_image_sync_is_off() {
        let (a, mut b) = paired_devices().await;
        b.service.set_syncing_images(false);
        a.service.broadcast_image(&png_of(4, 3), details(4, 3)).await;

        assert_eq!(next_image(&mut b).await, None);
    }

    #[tokio::test]
    async fn text_still_syncs_while_image_sync_is_off() {
        let (a, mut b) = paired_devices().await;
        a.service.set_syncing_images(false);
        b.service.set_syncing_images(false);
        a.service.broadcast(clip("text only")).await;

        assert_eq!(next_clip(&mut b).await, Some(clip("text only")));
    }

    fn offer_info() -> FileOfferInfo {
        FileOfferInfo {
            offer_id: "33333333-3333-4333-8333-333333333333".into(),
            name: "report.pdf".into(),
            size: 2048,
            mime: "application/pdf".into(),
            sha256: "a".repeat(64),
        }
    }

    async fn send_offer(to: SocketAddr, from: &str, info: &FileOfferInfo) {
        let frame = crate::sync::offers::seal_file_offer(from, &KEY, info).unwrap();
        transport::send_frame(&to.to_string(), &frame).await.unwrap();
    }

    async fn next_offer(device: &mut Device) -> Option<ReceivedOffer> {
        tokio::time::timeout(Duration::from_millis(500), device.offers.recv())
            .await
            .ok()
            .flatten()
    }

    #[tokio::test]
    async fn offer_from_a_paired_device_is_applied() {
        let (_a, mut b) = paired_devices().await;
        send_offer(b.addr, "a", &offer_info()).await;

        assert_eq!(
            next_offer(&mut b).await,
            Some(ReceivedOffer {
                info: offer_info(),
                from_id: "a".into(),
                from_name: "a".into(),
            })
        );
    }

    #[tokio::test]
    async fn offered_file_name_is_cleaned_on_arrival() {
        let (_a, mut b) = paired_devices().await;
        let sneaky = FileOfferInfo {
            name: "../../.bashrc".into(),
            ..offer_info()
        };
        send_offer(b.addr, "a", &sneaky).await;

        assert_eq!(next_offer(&mut b).await.unwrap().info.name, ".bashrc");
    }

    #[tokio::test]
    async fn offer_from_an_unpaired_device_is_dropped() {
        let (_a, mut b) = paired_devices().await;
        send_offer(b.addr, "stranger", &offer_info()).await;

        assert_eq!(next_offer(&mut b).await, None);
    }

    #[tokio::test]
    async fn invalid_offer_is_dropped() {
        let (_a, mut b) = paired_devices().await;
        let bad = FileOfferInfo {
            sha256: "not-a-hash".into(),
            ..offer_info()
        };
        send_offer(b.addr, "a", &bad).await;

        assert_eq!(next_offer(&mut b).await, None);
    }

    const SHARED_OFFER: &str = "55555555-5555-4555-8555-555555555555";

    /// Share `content` from `owner` with `offered_to`, the way Send will.
    fn share(owner: &Device, content: &[u8], offered_to: &[&str]) -> PathBuf {
        let path = owner._dir.path().join("shared.bin");
        std::fs::write(&path, content).unwrap();
        let hashed = hash_file(&path, |_, _| {}).unwrap();
        let offer = Offer {
            offer_id: SHARED_OFFER.into(),
            item_id: "item".into(),
            path: path.clone(),
            stamp: hashed.stamp,
            sha256: hashed.sha256,
            offered_to: offered_to.iter().map(|id| id.to_string()).collect(),
            created_at: String::new(),
        };
        lock(&owner.shared_files).insert(SHARED_OFFER.into(), offer);
        path
    }

    /// Fetch `offer_id` from `owner` as device `as_id`: the file, or why not.
    async fn fetch(
        owner: &Device,
        as_id: &str,
        offer_id: &str,
    ) -> Result<Vec<u8>, Option<crate::sync::fetch::RefusalReason>> {
        use crate::sync::fetch::{decode_salt, open_fetch_reply, seal_fetch_request};

        let mut stream = transport::connect(&owner.addr.to_string()).await.unwrap();
        let request = seal_fetch_request(as_id, &KEY, offer_id).unwrap();
        transport::write_frame(&mut stream, &request).await.unwrap();
        // No answer at all (connection closed) is `Err(None)`.
        let reply = transport::read_frame(&mut stream).await.map_err(|_| None)?;
        let (salt, size) = match open_fetch_reply(&reply, &KEY, offer_id).unwrap() {
            FetchReply::Start { salt, size } => (decode_salt(&salt).unwrap(), size),
            FetchReply::Refused { reason } => return Err(Some(reason)),
        };

        let mut file = Vec::new();
        let context = stream_context(offer_id, &owner.service.identity().0);
        crate::sync::stream::receive_stream(
            &mut stream,
            &mut file,
            &KEY,
            &salt,
            &context,
            size,
            FILE_IDLE_TIMEOUT,
            |_| {},
        )
        .await
        .unwrap();
        Ok(file)
    }

    #[tokio::test]
    async fn offered_file_is_served_to_its_device() {
        let (a, _b) = paired_devices().await;
        share(&a, b"the quarterly report", &["b"]);

        assert_eq!(fetch(&a, "b", SHARED_OFFER).await, Ok(b"the quarterly report".to_vec()));
    }

    #[tokio::test]
    async fn multi_chunk_file_is_served_intact() {
        let (a, _b) = paired_devices().await;
        let content: Vec<u8> = (0..700 * 1024).map(|i| (i % 251) as u8).collect();
        share(&a, &content, &["b"]);

        assert_eq!(fetch(&a, "b", SHARED_OFFER).await, Ok(content));
    }

    #[tokio::test]
    async fn file_not_offered_to_the_requester_is_refused() {
        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr_b = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let addr_c = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        let a = device("a", listener_a, vec![peer("b", addr_b), peer("c", addr_c)]).await;
        share(&a, b"only for b", &["b"]);

        assert_eq!(
            fetch(&a, "c", SHARED_OFFER).await,
            Err(Some(crate::sync::fetch::RefusalReason::NotShared))
        );
    }

    #[tokio::test]
    async fn unknown_offer_is_refused() {
        let (a, _b) = paired_devices().await;
        assert_eq!(
            fetch(&a, "b", SHARED_OFFER).await,
            Err(Some(crate::sync::fetch::RefusalReason::NotShared))
        );
    }

    #[tokio::test]
    async fn deleted_file_is_refused_as_missing() {
        let (a, _b) = paired_devices().await;
        let path = share(&a, b"soon gone", &["b"]);
        std::fs::remove_file(path).unwrap();

        assert_eq!(
            fetch(&a, "b", SHARED_OFFER).await,
            Err(Some(crate::sync::fetch::RefusalReason::Missing))
        );
    }

    #[tokio::test]
    async fn edited_file_is_refused_as_changed() {
        let (a, _b) = paired_devices().await;
        let path = share(&a, b"first draft", &["b"]);
        std::fs::write(path, b"second draft, longer").unwrap();

        assert_eq!(
            fetch(&a, "b", SHARED_OFFER).await,
            Err(Some(crate::sync::fetch::RefusalReason::Changed))
        );
    }

    #[tokio::test]
    async fn unpaired_device_gets_no_answer() {
        let (a, _b) = paired_devices().await;
        share(&a, b"private", &["stranger"]);

        assert_eq!(fetch(&a, "stranger", SHARED_OFFER).await, Err(None));
    }

    #[tokio::test]
    async fn serving_a_file_never_modifies_it() {
        let (a, _b) = paired_devices().await;
        let path = share(&a, b"leave me alone", &["b"]);
        fetch(&a, "b", SHARED_OFFER).await.unwrap();

        assert_eq!(std::fs::read(path).unwrap(), b"leave me alone");
    }

    fn remote_for(owner: &Device, content: &[u8]) -> RemoteFile {
        RemoteFile {
            offer_id: SHARED_OFFER.into(),
            origin_id: owner.service.identity().0,
            origin_name: "a".into(),
            name: "report.pdf".into(),
            size: content.len() as u64,
            sha256: hash_file(&owner._dir.path().join("shared.bin"), |_, _| {}).unwrap().sha256,
        }
    }

    #[tokio::test]
    async fn fetched_file_lands_in_the_download_folder() {
        let (a, b) = paired_devices().await;
        share(&a, b"the quarterly report", &["b"]);
        let downloads = tempfile::tempdir().unwrap();
        let placed = b
            .service
            .fetch_file(&remote_for(&a, b"the quarterly report"), downloads.path(), |_| {})
            .await
            .unwrap();

        assert_eq!(
            (placed.clone(), std::fs::read(&placed).unwrap()),
            (downloads.path().join("report.pdf"), b"the quarterly report".to_vec())
        );
    }

    #[tokio::test]
    async fn fetching_leaves_no_partial_file() {
        let (a, b) = paired_devices().await;
        share(&a, b"the quarterly report", &["b"]);
        let downloads = tempfile::tempdir().unwrap();
        b.service
            .fetch_file(&remote_for(&a, b"the quarterly report"), downloads.path(), |_| {})
            .await
            .unwrap();
        let leftovers = std::fs::read_dir(downloads.path().join(".partial")).unwrap().count();

        assert_eq!(leftovers, 0);
    }

    #[tokio::test]
    async fn file_that_doesnt_match_its_offer_is_discarded() {
        let (a, b) = paired_devices().await;
        share(&a, b"the quarterly report", &["b"]);
        let downloads = tempfile::tempdir().unwrap();
        let lie = RemoteFile {
            sha256: "f".repeat(64),
            ..remote_for(&a, b"the quarterly report")
        };
        let result = b.service.fetch_file(&lie, downloads.path(), |_| {}).await;
        let in_folder = std::fs::read_dir(downloads.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name() != ".partial")
            .count();
        let partials = std::fs::read_dir(downloads.path().join(".partial")).unwrap().count();

        assert_eq!((result.is_err(), in_folder, partials), (true, 0, 0));
    }

    #[tokio::test]
    async fn fetching_a_refused_file_reports_why() {
        let (a, b) = paired_devices().await;
        let path = share(&a, b"soon gone", &["b"]);
        let remote = remote_for(&a, b"soon gone");
        std::fs::remove_file(path).unwrap();
        let downloads = tempfile::tempdir().unwrap();

        assert!(matches!(
            b.service.fetch_file(&remote, downloads.path(), |_| {}).await,
            Err(FetchError::Refused(RefusalReason::Missing))
        ));
    }

    #[tokio::test]
    async fn fetching_from_an_offline_device_says_so() {
        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let gone = dead_addr().await;
        let b = device("b", listener_b, vec![peer("a", gone)]).await;
        let remote = RemoteFile {
            offer_id: SHARED_OFFER.into(),
            origin_id: "a".into(),
            origin_name: "a".into(),
            name: "x".into(),
            size: 1,
            sha256: "a".repeat(64),
        };
        let downloads = tempfile::tempdir().unwrap();

        assert!(matches!(
            b.service.fetch_file(&remote, downloads.path(), |_| {}).await,
            Err(FetchError::Offline(_))
        ));
    }

    #[tokio::test]
    async fn offer_goes_only_to_the_chosen_devices() {
        let listener_a = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_b = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let listener_c = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (addr_a, addr_b, addr_c) = (
            listener_a.local_addr().unwrap(),
            listener_b.local_addr().unwrap(),
            listener_c.local_addr().unwrap(),
        );
        let a = device("a", listener_a, vec![peer("b", addr_b), peer("c", addr_c)]).await;
        let mut b = device("b", listener_b, vec![peer("a", addr_a)]).await;
        let mut c = device("c", listener_c, vec![peer("a", addr_a)]).await;
        a.service.send_offer(&offer_info(), &["b".to_string()]).await;

        assert_eq!(
            (next_offer(&mut b).await.is_some(), next_offer(&mut c).await.is_some()),
            (true, false)
        );
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
            apply_offer: Arc::new(|_| {}),
            lookup_offer: Arc::new(|_| None),
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
