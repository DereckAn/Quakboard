//! TCP transport: one frame per connection, sent as a 4-byte big-endian length
//! followed by the frame's JSON, stamped with the sender's `Protocol`.
//!
//! The listener faces the LAN, so everything it reads is untrusted: frames are
//! size-capped, reads time out, and concurrent connections are bounded.

use std::{future::Future, io, net::SocketAddr, pin::Pin, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    time::timeout,
};

use super::{
    protocol::{Incompatible, Protocol},
    Frame,
};

/// Fixed so users can allow it through their firewall.
pub const SYNC_PORT: u16 = 47823;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_CONNECTIONS: usize = 16;
const PROTOCOL_KEY: &str = "protocol";

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Frame) -> io::Result<()> {
    write_stamped(writer, frame, Protocol::CURRENT).await
}

/// `write_frame` claiming `protocol`, for tests that play another version.
pub async fn write_stamped<W: AsyncWrite + Unpin>(
    writer: &mut W,
    frame: &Frame,
    protocol: Protocol,
) -> io::Result<()> {
    let mut value = serde_json::to_value(frame)?;
    // Older devices ignore keys they don't know, so they still read it.
    if let Some(fields) = value.as_object_mut() {
        fields.insert(PROTOCOL_KEY.into(), serde_json::to_value(protocol)?);
    }
    let body = serde_json::to_vec(&value)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sync frame too large",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}

/// Read a frame from a device this one can work with. Any other frame fails
/// with an `Incompatible` inside the error (see `Incompatible::of`), judged
/// before parsing: a newer device's frames may not parse here at all.
pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Frame> {
    let (body, protocol) = read_body(reader).await?;
    Protocol::CURRENT.check(protocol).map_err(refused)?;
    parse(body)
}

/// The first frame on a connection, as the listener hands it on.
#[derive(Debug, PartialEq)]
pub enum Incoming {
    /// A frame and the protocol its sender claims, unjudged, so pairing can
    /// still answer and explain a refusal.
    Frame(Frame, Protocol),
    /// A frame this build can't parse, from a device it can't work with.
    Incompatible {
        /// The `from` it claims, if any. Untrusted.
        sender: Option<String>,
        reason: Incompatible,
    },
}

/// Read a connection's first frame. Its sender id and protocol are taken
/// from the JSON before parsing, so a newer device's frame that doesn't
/// parse here still says who sent it and why it was refused.
pub async fn read_stamped<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Incoming> {
    let (body, protocol) = read_body(reader).await?;
    let sender = body
        .get("from")
        .and_then(|from| from.as_str())
        .map(str::to_string);
    match parse(body) {
        Ok(frame) => Ok(Incoming::Frame(frame, protocol)),
        Err(e) => match Protocol::CURRENT.check(protocol) {
            Ok(()) => Err(e),
            Err(reason) => Ok(Incoming::Incompatible { sender, reason }),
        },
    }
}

/// The frame's JSON, with its protocol stamp taken out.
async fn read_body<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> io::Result<(serde_json::Value, Protocol)> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await?;
    let len = u32::from_be_bytes(len) as usize;
    // Checked before allocating, so a bogus length can't reserve memory.
    if len > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "sync frame too large",
        ));
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body).await?;
    let mut value: serde_json::Value = serde_json::from_slice(&body).map_err(invalid)?;
    // No stamp: a v1.5.x device.
    let protocol = match value.as_object_mut().and_then(|v| v.remove(PROTOCOL_KEY)) {
        Some(stamp) => serde_json::from_value(stamp).map_err(invalid)?,
        None => Protocol::default(),
    };
    Ok((value, protocol))
}

fn parse(body: serde_json::Value) -> io::Result<Frame> {
    serde_json::from_value(body).map_err(invalid)
}

fn invalid(e: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

fn refused(e: Incompatible) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

pub async fn connect(addr: &str) -> io::Result<TcpStream> {
    timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))?
}

pub async fn send_frame(addr: &str, frame: &Frame) -> io::Result<()> {
    let mut stream = connect(addr).await?;
    timeout(IO_TIMEOUT, write_frame(&mut stream, frame))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "send timed out"))?
}

/// Handles a connection after its first frame. Gets the stream too, because
/// pairing continues the conversation on the same connection.
pub type ConnectionHandler = Arc<
    dyn Fn(Incoming, TcpStream, SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send>>
        + Send
        + Sync,
>;

/// Accept connections until the task is dropped, reading the first frame
/// from each and handing the connection on.
pub async fn serve(listener: TcpListener, on_connection: ConnectionHandler) {
    let slots = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));

    loop {
        let (mut stream, peer_addr) = match listener.accept().await {
            Ok(connection) => connection,
            Err(e) => {
                eprintln!("Sync listener accept failed: {e}");
                // Back off so a persistent error (e.g. out of fds) can't spin.
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Ok(slot) = slots.clone().acquire_owned().await else {
            return;
        };
        let on_connection = on_connection.clone();

        tokio::spawn(async move {
            let _slot = slot;
            match timeout(IO_TIMEOUT, read_stamped(&mut stream)).await {
                Ok(Ok(incoming)) => on_connection(incoming, stream, peer_addr).await,
                Ok(Err(e)) => eprintln!("Dropped sync frame from {peer_addr}: {e}"),
                Err(_) => eprintln!("Sync connection from {peer_addr} timed out"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClipPayload, PeerKey};

    const KEY: PeerKey = [5; 32];

    fn frame() -> Frame {
        let clip = ClipPayload {
            text: "over the wire".into(),
            content_type: "text".into(),
        };
        Frame::seal_clip("device-a", &KEY, &clip).unwrap()
    }

    #[tokio::test]
    async fn frame_survives_write_then_read() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let sent = frame();
        write_frame(&mut client, &sent).await.unwrap();

        assert_eq!(read_frame(&mut server).await.unwrap(), sent);
    }

    #[tokio::test]
    async fn oversized_length_prefix_is_rejected() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let too_big = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        client.write_all(&too_big).await.unwrap();

        let err = read_frame(&mut server).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn non_frame_json_is_rejected() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let body = br#"{"type":"Nope"}"#;
        client.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
        client.write_all(body).await.unwrap();

        let err = read_frame(&mut server).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    /// A future device that no longer works with this build.
    const FUTURE: Protocol = Protocol {
        version: 2,
        min_peer: 2,
    };

    async fn body_of(frame: &Frame) -> Vec<u8> {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        write_frame(&mut client, frame).await.unwrap();
        let mut len = [0u8; 4];
        server.read_exact(&mut len).await.unwrap();
        let mut body = vec![0; u32::from_be_bytes(len) as usize];
        server.read_exact(&mut body).await.unwrap();
        body
    }

    #[tokio::test]
    async fn frames_carry_this_devices_protocol() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        write_frame(&mut client, &frame()).await.unwrap();

        let Incoming::Frame(_, protocol) = read_stamped(&mut server).await.unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!(protocol, Protocol::CURRENT);
    }

    #[tokio::test]
    async fn frame_without_a_stamp_reads_as_version_1() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let body = br#"{"type":"PairDone"}"#;
        client.write_all(&(body.len() as u32).to_be_bytes()).await.unwrap();
        client.write_all(body).await.unwrap();

        assert_eq!(
            read_stamped(&mut server).await.unwrap(),
            Incoming::Frame(Frame::PairDone, Protocol::default())
        );
    }

    #[tokio::test]
    async fn older_devices_can_still_read_a_stamped_frame() {
        // v1.5.x devices parse the body straight into a Frame, which has no
        // protocol field and ignores the stamp.
        let sent = frame();
        let body = body_of(&sent).await;
        assert_eq!(serde_json::from_slice::<Frame>(&body).unwrap(), sent);
    }

    #[tokio::test]
    async fn frame_from_a_device_we_cannot_work_with_is_refused() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        write_stamped(&mut client, &frame(), FUTURE).await.unwrap();

        let err = read_frame(&mut server).await.unwrap_err();
        assert_eq!(Incompatible::of(&err), Some(Incompatible::ThisTooOld));
    }

    /// A frame type this build doesn't know, from a device it can't work with.
    const UNKNOWN_FUTURE_FRAME: &[u8] =
        br#"{"type":"ClipV2","protocol":{"version":2,"min_peer":2}}"#;

    async fn server_reading(body: &[u8]) -> tokio::io::DuplexStream {
        let (mut client, server) = tokio::io::duplex(1024);
        client
            .write_all(&(body.len() as u32).to_be_bytes())
            .await
            .unwrap();
        client.write_all(body).await.unwrap();
        server
    }

    #[tokio::test]
    async fn unknown_frame_from_a_device_we_cannot_work_with_is_refused_as_incompatible() {
        let mut server = server_reading(UNKNOWN_FUTURE_FRAME).await;
        let err = read_frame(&mut server).await.unwrap_err();

        assert_eq!(Incompatible::of(&err), Some(Incompatible::ThisTooOld));
    }

    #[tokio::test]
    async fn listener_keeps_the_sender_of_an_unknown_future_frame() {
        let body = br#"{"type":"ClipV2","from":"laptop","protocol":{"version":2,"min_peer":2}}"#;
        let mut server = server_reading(body).await;

        assert_eq!(
            read_stamped(&mut server).await.unwrap(),
            Incoming::Incompatible {
                sender: Some("laptop".into()),
                reason: Incompatible::ThisTooOld,
            }
        );
    }

    #[tokio::test]
    async fn listener_reads_an_unknown_future_frame_without_a_sender() {
        let mut server = server_reading(UNKNOWN_FUTURE_FRAME).await;

        assert_eq!(
            read_stamped(&mut server).await.unwrap(),
            Incoming::Incompatible {
                sender: None,
                reason: Incompatible::ThisTooOld,
            }
        );
    }

    #[tokio::test]
    async fn unknown_frame_from_a_compatible_device_is_still_malformed() {
        let mut server = server_reading(br#"{"type":"Nope","from":"laptop"}"#).await;
        let err = read_stamped(&mut server).await.unwrap_err();

        assert_eq!(
            (err.kind(), Incompatible::of(&err)),
            (io::ErrorKind::InvalidData, None)
        );
    }

    #[tokio::test]
    async fn sending_to_a_closed_port_fails() {
        // Bind then drop to get a port nothing is listening on.
        let addr = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        assert!(send_frame(&addr.to_string(), &frame()).await.is_err());
    }
}
