//! TCP transport: one frame per connection, sent as a 4-byte big-endian length
//! followed by the frame's JSON.
//!
//! The listener faces the LAN, so everything it reads is untrusted: frames are
//! size-capped, reads time out, and concurrent connections are bounded.

use std::{io, net::SocketAddr, sync::Arc, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
    time::timeout,
};

use super::Frame;

/// Fixed so users can allow it through their firewall.
pub const SYNC_PORT: u16 = 47823;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_CONNECTIONS: usize = 16;

pub async fn write_frame<W: AsyncWrite + Unpin>(writer: &mut W, frame: &Frame) -> io::Result<()> {
    let body = serde_json::to_vec(frame)?;
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

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> io::Result<Frame> {
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
    serde_json::from_slice(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

pub async fn send_frame(addr: &str, frame: &Frame) -> io::Result<()> {
    let mut stream = timeout(CONNECT_TIMEOUT, TcpStream::connect(addr))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "connect timed out"))??;
    timeout(IO_TIMEOUT, write_frame(&mut stream, frame))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "send timed out"))?
}

pub type FrameHandler = Arc<dyn Fn(Frame, SocketAddr) + Send + Sync>;

/// Accept connections until the task is dropped, reading one frame from each.
pub async fn serve(listener: TcpListener, on_frame: FrameHandler) {
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
        let on_frame = on_frame.clone();

        tokio::spawn(async move {
            let _slot = slot;
            match timeout(IO_TIMEOUT, read_frame(&mut stream)).await {
                Ok(Ok(frame)) => on_frame(frame, peer_addr),
                Ok(Err(e)) => eprintln!("Dropped sync frame from {peer_addr}: {e}"),
                Err(_) => eprintln!("Sync connection from {peer_addr} timed out"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::{ClipPayload, PeerKey};

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

    #[tokio::test]
    async fn sending_to_a_closed_port_fails() {
        // Bind then drop to get a port nothing is listening on.
        let addr = TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap();
        assert!(send_frame(&addr.to_string(), &frame()).await.is_err());
    }
}
