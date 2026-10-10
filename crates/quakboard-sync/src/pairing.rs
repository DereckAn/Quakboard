//! Pairing two devices with a 6-digit code, over one connection.
//!
//! ```text
//! A (initiator)                              B (responder, shows the code)
//!   PairRequest   { from: A }        ->
//!                                    <-  PairChallenge { from: B }
//!   ... user reads B's code and types it on A ...
//!   PairSpake     { A's SPAKE2 msg } ->
//!                                    <-  PairAccept { B's msg, sealed B info }
//!   (A opening B's info proves both sides used the same code)
//!   PairFinish    { sealed A info }  ->
//!                                    <-  PairDone
//! ```
//!
//! Both opening frames carry their sender's `Protocol` (as every frame does),
//! so devices that can't work together say which one needs updating before
//! anyone types a code.
//!
//! SPAKE2 turns the short code into a strong shared key without revealing it:
//! an eavesdropper learns nothing, and an active attacker gets one guess per
//! attempt, with a fresh code every attempt. SPAKE2 itself doesn't report a
//! wrong code (both sides just derive different keys), so each side proves it
//! holds the key by sealing its info, which the other must open.

use std::{
    fmt, io,
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spake2::{Ed25519Group, Identity, Password, Spake2};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::oneshot,
    time::timeout,
};

use super::{
    protocol::{Incompatible, Protocol},
    store::Peer,
    transport::{read_frame, write_frame},
    Frame, PeerKey, Sealed,
};

/// How long the user has to read the code on one device and type it on the other.
pub const CODE_ENTRY_TIMEOUT: Duration = Duration::from_secs(120);
/// How long either side waits for the next protocol message otherwise.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_NAME_CHARS: usize = 64;
const UNNAMED_DEVICE: &str = "Unnamed device";

/// This device, as seen by the one it pairs with.
pub struct LocalDevice<'a> {
    pub id: &'a str,
    pub name: &'a str,
    /// Port this device's sync listener is on, so the peer can reach it later.
    pub listen_port: u16,
}

/// Sent sealed, so names only travel once the code is proven.
#[derive(Serialize, Deserialize)]
struct PairInfo {
    name: String,
    port: u16,
}

#[derive(Debug)]
pub enum PairError {
    Io(io::Error),
    /// The other side sent something out of order or invalid.
    Protocol(&'static str),
    /// The codes typed and shown didn't match.
    WrongCode,
    /// Nobody entered the code in time, or the other side went quiet.
    Timeout,
    /// The code entry was abandoned (e.g. the pairing dialog was closed).
    Cancelled,
    /// This device is already pairing with another.
    Busy,
    /// One of the two devices needs a newer Quakboard.
    Incompatible(Incompatible),
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairError::Io(e) => write!(f, "pairing connection failed: {e}"),
            PairError::Protocol(reason) => write!(f, "pairing protocol error: {reason}"),
            PairError::WrongCode => write!(f, "the pairing code didn't match"),
            PairError::Timeout => write!(f, "pairing timed out"),
            PairError::Cancelled => write!(f, "pairing was cancelled"),
            PairError::Busy => write!(f, "already pairing with another device"),
            PairError::Incompatible(e) => write!(f, "{e}, then pair again"),
        }
    }
}

impl std::error::Error for PairError {}

impl From<io::Error> for PairError {
    fn from(e: io::Error) -> Self {
        PairError::Io(e)
    }
}

/// Run the initiator side (A). `code` delivers what the user typed.
/// `peer_ip` is the address `stream` is connected to.
pub async fn initiate<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    me: &LocalDevice<'_>,
    peer_ip: IpAddr,
    code: oneshot::Receiver<String>,
) -> Result<Peer, PairError> {
    write_frame(stream, &Frame::PairRequest { from: me.id.into() }).await?;
    // A responder we can't work with is refused here, by `read_frame`.
    let their_id = match next_frame(stream, STEP_TIMEOUT).await? {
        Frame::PairChallenge { from } => validated_id(from)?,
        _ => return Err(PairError::Protocol("expected PairChallenge")),
    };

    let code = timeout(CODE_ENTRY_TIMEOUT, code)
        .await
        .map_err(|_| PairError::Timeout)?
        .map_err(|_| PairError::Cancelled)?;
    let (spake, my_msg) = Spake2::<Ed25519Group>::start_a(
        &Password::new(code.trim().as_bytes()),
        &Identity::new(me.id.as_bytes()),
        &Identity::new(their_id.as_bytes()),
    );
    write_frame(
        stream,
        &Frame::PairSpake {
            spake_msg: BASE64.encode(my_msg),
        },
    )
    .await?;

    let (their_msg, their_sealed) = match next_frame(stream, STEP_TIMEOUT).await? {
        Frame::PairAccept { spake_msg, sealed } => (spake_msg, sealed),
        _ => return Err(PairError::Protocol("expected PairAccept")),
    };
    let keys = finish_spake(spake, &their_msg)?;
    let their_info: PairInfo = their_sealed
        .open(&keys.confirm, &info_aad(&their_id))
        .map_err(|_| PairError::WrongCode)?;

    write_frame(
        stream,
        &Frame::PairFinish {
            sealed: seal_info(&keys, me)?,
        },
    )
    .await?;
    match next_frame(stream, STEP_TIMEOUT).await? {
        Frame::PairDone => {}
        _ => return Err(PairError::Protocol("expected PairDone")),
    }

    Ok(peer_from(their_id, their_info, peer_ip, keys.peer))
}

/// Run the responder side (B), after its listener read `PairRequest` and the
/// protocol it was stamped with.
/// `show_code` is called once with the code the user must type on the other device.
pub async fn respond<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    me: &LocalDevice<'_>,
    their_id: String,
    their_protocol: Protocol,
    peer_ip: IpAddr,
    show_code: impl FnOnce(String),
) -> Result<Peer, PairError> {
    let their_id = validated_id(their_id)?;
    // Sent even to a device we'll refuse: our stamp tells it why.
    write_frame(stream, &Frame::PairChallenge { from: me.id.into() }).await?;
    Protocol::CURRENT
        .check(their_protocol)
        .map_err(PairError::Incompatible)?;

    let code = generate_code()?;
    show_code(code.clone());

    let their_msg = match next_frame(stream, CODE_ENTRY_TIMEOUT).await? {
        Frame::PairSpake { spake_msg } => spake_msg,
        _ => return Err(PairError::Protocol("expected PairSpake")),
    };
    let (spake, my_msg) = Spake2::<Ed25519Group>::start_b(
        &Password::new(code.as_bytes()),
        &Identity::new(their_id.as_bytes()),
        &Identity::new(me.id.as_bytes()),
    );
    let keys = finish_spake(spake, &their_msg)?;

    write_frame(
        stream,
        &Frame::PairAccept {
            spake_msg: BASE64.encode(my_msg),
            sealed: seal_info(&keys, me)?,
        },
    )
    .await?;

    // A wrong code makes the initiator hang up here instead of answering.
    let their_sealed = match next_frame(stream, STEP_TIMEOUT).await {
        Ok(Frame::PairFinish { sealed }) => sealed,
        Ok(_) => return Err(PairError::Protocol("expected PairFinish")),
        Err(PairError::Io(e)) if e.kind() == io::ErrorKind::UnexpectedEof => {
            return Err(PairError::WrongCode)
        }
        Err(e) => return Err(e),
    };
    let their_info: PairInfo = their_sealed
        .open(&keys.confirm, &info_aad(&their_id))
        .map_err(|_| PairError::WrongCode)?;
    write_frame(stream, &Frame::PairDone).await?;

    Ok(peer_from(their_id, their_info, peer_ip, keys.peer))
}

/// A uniformly random 6-digit code, zero-padded.
pub fn generate_code() -> Result<String, PairError> {
    // Rejection sampling: 4_294_000_000 is the largest multiple of 1_000_000
    // below 2^32, so accepting only values under it avoids modulo bias.
    const LIMIT: u32 = 4_294_000_000;
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes)
            .map_err(|e| PairError::Io(io::Error::other(e.to_string())))?;
        let value = u32::from_be_bytes(bytes);
        if value < LIMIT {
            return Ok(format!("{:06}", value % 1_000_000));
        }
    }
}

struct PairKeys {
    /// Only for sealing the handshake's info messages.
    confirm: PeerKey,
    /// Stored with the peer and used for every clip afterwards.
    peer: PeerKey,
}

fn finish_spake(spake: Spake2<Ed25519Group>, their_msg: &str) -> Result<PairKeys, PairError> {
    let their_msg = BASE64
        .decode(their_msg)
        .map_err(|_| PairError::Protocol("SPAKE2 message isn't base64"))?;
    let shared = spake
        .finish(&their_msg)
        .map_err(|_| PairError::Protocol("invalid SPAKE2 message"))?;

    // Domain-separated hashes, so the handshake key and the long-term key
    // are independent even though both come from the same secret.
    let derive = |label: &[u8]| -> PeerKey {
        let mut hasher = Sha256::new();
        hasher.update(label);
        hasher.update(&shared);
        hasher.finalize().into()
    };
    Ok(PairKeys {
        confirm: derive(b"quakboard-pair-confirm-v1"),
        peer: derive(b"quakboard-peer-key-v1"),
    })
}

fn seal_info(keys: &PairKeys, me: &LocalDevice<'_>) -> Result<Sealed, PairError> {
    let info = PairInfo {
        name: me.name.into(),
        port: me.listen_port,
    };
    Sealed::seal(&keys.confirm, &info_aad(me.id), &info)
        .map_err(|_| PairError::Protocol("could not seal device info"))
}

fn info_aad(device_id: &str) -> String {
    format!("pair-info:{device_id}")
}

fn peer_from(id: String, info: PairInfo, ip: IpAddr, key: PeerKey) -> Peer {
    Peer {
        id,
        name: clean_name(&info.name),
        key,
        last_addr: Some(SocketAddr::new(ip, info.port).to_string()),
    }
}

/// The other device picks its own name, so keep it short and printable.
fn clean_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    if cleaned.is_empty() {
        UNNAMED_DEVICE.into()
    } else {
        cleaned.into()
    }
}

fn validated_id(id: String) -> Result<String, PairError> {
    uuid::Uuid::parse_str(&id)
        .map(|_| id)
        .map_err(|_| PairError::Protocol("device id isn't a UUID"))
}

async fn next_frame<S: AsyncRead + Unpin>(stream: &mut S, wait: Duration) -> Result<Frame, PairError> {
    timeout(wait, read_frame(stream))
        .await
        .map_err(|_| PairError::Timeout)?
        .map_err(|e| match Incompatible::of(&e) {
            Some(refusal) => PairError::Incompatible(refusal),
            None => PairError::Io(e),
        })
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;

    use super::*;
    use crate::transport::{read_stamped, write_stamped, Incoming};

    const A_ID: &str = "11111111-1111-4111-8111-111111111111";
    const B_ID: &str = "22222222-2222-4222-8222-222222222222";
    const LOCALHOST: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

    fn device_a() -> LocalDevice<'static> {
        LocalDevice {
            id: A_ID,
            name: "Laptop",
            listen_port: 4001,
        }
    }

    fn device_b() -> LocalDevice<'static> {
        LocalDevice {
            id: B_ID,
            name: "Desktop",
            listen_port: 4002,
        }
    }

    /// Run both sides at once. `typed` turns the code B shows into what the
    /// user types on A. Returns (A's result, B's result).
    async fn pair(
        typed: impl FnOnce(String) -> String + Send + 'static,
    ) -> (Result<Peer, PairError>, Result<Peer, PairError>) {
        let (mut a_stream, mut b_stream) = tokio::io::duplex(64 * 1024);
        let (code_tx, code_rx) = oneshot::channel();

        let a = tokio::spawn(async move {
            initiate(&mut a_stream, &device_a(), LOCALHOST, code_rx).await
        });
        let b = tokio::spawn(async move {
            // The listener reads the opening PairRequest before handing over.
            let opening = read_stamped(&mut b_stream).await.unwrap();
            let Incoming::Frame(Frame::PairRequest { from }, protocol) = opening else {
                panic!("expected PairRequest first");
            };
            let mut code_tx = Some(code_tx);
            let result = respond(
                &mut b_stream,
                &device_b(),
                from,
                protocol,
                LOCALHOST,
                |code| {
                    let _ = code_tx.take().unwrap().send(typed(code));
                },
            )
            .await;
            // Hang up like a real connection would when this side is done.
            drop(b_stream);
            result
        });

        (a.await.unwrap(), b.await.unwrap())
    }

    #[tokio::test]
    async fn right_code_gives_both_sides_the_same_key() {
        let (a, b) = pair(|code| code).await;
        assert_eq!(a.unwrap().key, b.unwrap().key);
    }

    #[tokio::test]
    async fn initiator_learns_the_responders_identity() {
        let (a, _) = pair(|code| code).await;
        let peer = a.unwrap();
        assert_eq!(
            (peer.id.as_str(), peer.name.as_str(), peer.last_addr.as_deref()),
            (B_ID, "Desktop", Some("127.0.0.1:4002"))
        );
    }

    #[tokio::test]
    async fn responder_learns_the_initiators_identity() {
        let (_, b) = pair(|code| code).await;
        let peer = b.unwrap();
        assert_eq!(
            (peer.id.as_str(), peer.name.as_str(), peer.last_addr.as_deref()),
            (A_ID, "Laptop", Some("127.0.0.1:4001"))
        );
    }

    #[tokio::test]
    async fn surrounding_whitespace_in_the_typed_code_is_ignored() {
        let (a, _) = pair(|code| format!("  {code}\n")).await;
        assert!(a.is_ok());
    }

    #[tokio::test]
    async fn wrong_code_fails_on_the_initiator() {
        let (a, _) = pair(|code| wrong(&code)).await;
        assert!(matches!(a, Err(PairError::WrongCode)));
    }

    #[tokio::test]
    async fn wrong_code_fails_on_the_responder() {
        let (_, b) = pair(|code| wrong(&code)).await;
        assert!(matches!(b, Err(PairError::WrongCode)));
    }

    #[tokio::test]
    async fn abandoned_code_entry_cancels_pairing() {
        let (mut a_stream, mut b_stream) = tokio::io::duplex(64 * 1024);
        let (code_tx, code_rx) = oneshot::channel::<String>();
        tokio::spawn(async move {
            let _ = respond(
                &mut b_stream,
                &device_b(),
                A_ID.into(),
                Protocol::CURRENT,
                LOCALHOST,
                |_| {},
            )
            .await;
        });
        drop(code_tx);

        let result = initiate(&mut a_stream, &device_a(), LOCALHOST, code_rx).await;
        assert!(matches!(result, Err(PairError::Cancelled)));
    }

    #[tokio::test]
    async fn responder_rejects_a_non_uuid_device_id() {
        let (_a_stream, mut b_stream) = tokio::io::duplex(64 * 1024);
        let result = respond(
            &mut b_stream,
            &device_b(),
            "not-a-uuid".into(),
            Protocol::CURRENT,
            LOCALHOST,
            |_| {},
        )
        .await;
        assert!(matches!(result, Err(PairError::Protocol(_))));
    }

    // ---- protocol versions ----

    /// A future device that no longer pairs with this build.
    const FUTURE: Protocol = Protocol {
        version: 2,
        min_peer: 2,
    };

    #[tokio::test]
    async fn responder_refuses_a_device_it_cannot_work_with() {
        let (_a_stream, mut b_stream) = tokio::io::duplex(64 * 1024);
        let mut shown = false;
        let result = respond(
            &mut b_stream,
            &device_b(),
            A_ID.into(),
            FUTURE,
            LOCALHOST,
            |_| shown = true,
        )
        .await;

        let refused = matches!(
            result,
            Err(PairError::Incompatible(Incompatible::ThisTooOld))
        );
        assert_eq!((refused, shown), (true, false));
    }

    #[tokio::test]
    async fn initiator_refuses_before_the_code_is_typed() {
        let (mut a_stream, mut b_stream) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            read_frame(&mut b_stream).await.unwrap();
            let challenge = Frame::PairChallenge { from: B_ID.into() };
            write_stamped(&mut b_stream, &challenge, FUTURE)
                .await
                .unwrap();
            // Keep the connection open: the initiator must not wait for a code.
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        // Never sent: a refusal must not depend on the user typing anything.
        let (_code_tx, code_rx) = oneshot::channel();

        let result = initiate(&mut a_stream, &device_a(), LOCALHOST, code_rx).await;
        assert!(matches!(
            result,
            Err(PairError::Incompatible(Incompatible::ThisTooOld))
        ));
    }

    #[test]
    fn codes_are_six_digits() {
        let code = generate_code().unwrap();
        assert!(code.len() == 6 && code.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn control_characters_are_stripped_from_names() {
        assert_eq!(clean_name("Evil\u{1b}[31m Laptop\n"), "Evil[31m Laptop");
    }

    #[test]
    fn long_names_are_truncated() {
        assert_eq!(clean_name(&"x".repeat(200)).chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn blank_names_get_a_placeholder() {
        assert_eq!(clean_name(" \t "), UNNAMED_DEVICE);
    }

    /// A different 6-digit code than `code`.
    fn wrong(code: &str) -> String {
        let n: u32 = code.parse().unwrap();
        format!("{:06}", (n + 1) % 1_000_000)
    }
}
