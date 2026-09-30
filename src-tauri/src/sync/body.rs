//! Raw bytes sent after a frame on the same connection, for payloads too big
//! for JSON frames (images now, files next): a 4-byte big-endian length, then
//! the bytes in chunks.
//!
//! Everything read here is untrusted: the length is capped before reading,
//! memory grows only as bytes arrive, and a stalled or trickling sender times
//! out.
// NOTE: image sync is built in steps (docs/IMAGE_SYNC_PLAN.md); nothing sends
// or receives bodies until step 5. Drop this allow then.
#![allow(dead_code)]

use std::{future::Future, io, time::Duration};

use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    time::timeout,
};

/// Largest image, as its PNG bytes, that sync will send or accept.
pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
/// ChaCha20-Poly1305 adds this many bytes of authentication tag.
const AEAD_TAG_BYTES: usize = 16;
const BODY_CHUNK_BYTES: usize = 64 * 1024;

/// How much a binary body may carry and how long it may take.
#[derive(Debug, Clone, Copy)]
pub struct BodyLimits {
    pub max_len: usize,
    /// Longest wait for the next bytes. A slow but steady sender never trips
    /// it; a stalled one does.
    pub idle: Duration,
    /// Cap on the whole body, so a trickle can't hold a connection forever.
    /// None for transfers whose length the user chose (files).
    pub total: Option<Duration>,
}

/// An encrypted image body: up to 20 MB of PNG, which takes seconds on slow
/// Wi-Fi, so the idle timeout matters more than the total.
pub const IMAGE_BODY_LIMITS: BodyLimits = BodyLimits {
    max_len: MAX_IMAGE_BYTES + AEAD_TAG_BYTES,
    idle: Duration::from_secs(5),
    total: Some(Duration::from_secs(60)),
};

/// Send raw bytes after a frame: a 4-byte big-endian length, then the bytes
/// in chunks, each under the idle timeout.
pub async fn write_body<W: AsyncWrite + Unpin>(
    writer: &mut W,
    body: &[u8],
    limits: BodyLimits,
) -> io::Result<()> {
    if body.len() > limits.max_len {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "sync body too large",
        ));
    }
    with_total(limits, async {
        idle(limits, writer.write_all(&(body.len() as u32).to_be_bytes())).await?;
        for chunk in body.chunks(BODY_CHUNK_BYTES) {
            idle(limits, writer.write_all(chunk)).await?;
        }
        idle(limits, writer.flush()).await
    })
    .await
}

/// Read raw bytes sent by `write_body`. The length is checked before reading,
/// and memory grows only as bytes actually arrive, so a peer that announces
/// a large body and then stalls can't make us reserve it.
pub async fn read_body<R: AsyncRead + Unpin>(
    reader: &mut R,
    limits: BodyLimits,
) -> io::Result<Vec<u8>> {
    with_total(limits, async {
        let mut len = [0u8; 4];
        idle(limits, reader.read_exact(&mut len)).await?;
        let len = u32::from_be_bytes(len) as usize;
        if len > limits.max_len {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sync body too large",
            ));
        }

        let mut body = Vec::new();
        let mut chunk = vec![0u8; BODY_CHUNK_BYTES];
        while body.len() < len {
            let want = BODY_CHUNK_BYTES.min(len - body.len());
            let read = idle(limits, reader.read(&mut chunk[..want])).await?;
            if read == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "sync body ended early",
                ));
            }
            body.extend_from_slice(&chunk[..read]);
        }
        Ok(body)
    })
    .await
}

async fn idle<T>(limits: BodyLimits, step: impl Future<Output = io::Result<T>>) -> io::Result<T> {
    timeout(limits.idle, step)
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "sync transfer stalled"))?
}

async fn with_total<T>(
    limits: BodyLimits,
    transfer: impl Future<Output = io::Result<T>>,
) -> io::Result<T> {
    match limits.total {
        Some(total) => timeout(total, transfer)
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "sync transfer took too long"))?,
        None => transfer.await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tight limits so timeout tests run in milliseconds.
    const TEST_LIMITS: BodyLimits = BodyLimits {
        max_len: 1024 * 1024,
        idle: Duration::from_millis(100),
        total: Some(Duration::from_secs(2)),
    };

    /// Bytes that differ per position, so a reordered or dropped chunk shows.
    fn patterned(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[tokio::test]
    async fn body_survives_write_then_read() {
        // A small pipe forces the body through in many chunks.
        let (mut client, mut server) = tokio::io::duplex(8 * 1024);
        let sent = patterned(300 * 1024);
        let writer = {
            let sent = sent.clone();
            tokio::spawn(async move { write_body(&mut client, &sent, TEST_LIMITS).await })
        };

        let received = read_body(&mut server, TEST_LIMITS).await.unwrap();
        writer.await.unwrap().unwrap();
        assert_eq!(received, sent);
    }

    #[tokio::test]
    async fn empty_body_survives_write_then_read() {
        let (mut client, mut server) = tokio::io::duplex(64);
        write_body(&mut client, &[], TEST_LIMITS).await.unwrap();

        assert_eq!(read_body(&mut server, TEST_LIMITS).await.unwrap(), Vec::<u8>::new());
    }

    #[tokio::test]
    async fn body_exactly_at_the_limit_is_accepted() {
        let limits = BodyLimits { max_len: 4096, ..TEST_LIMITS };
        let (mut client, mut server) = tokio::io::duplex(8 * 1024);
        write_body(&mut client, &patterned(4096), limits).await.unwrap();

        assert_eq!(read_body(&mut server, limits).await.unwrap().len(), 4096);
    }

    #[tokio::test]
    async fn oversized_body_length_is_rejected_before_reading() {
        let limits = BodyLimits { max_len: 4096, ..TEST_LIMITS };
        let (mut client, mut server) = tokio::io::duplex(64);
        client.write_all(&4097u32.to_be_bytes()).await.unwrap();

        let err = read_body(&mut server, limits).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn oversized_body_is_refused_by_the_sender() {
        let limits = BodyLimits { max_len: 4096, ..TEST_LIMITS };
        let (mut client, _server) = tokio::io::duplex(64);

        let err = write_body(&mut client, &patterned(4097), limits).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[tokio::test]
    async fn body_cut_short_is_rejected() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        client.write_all(&1000u32.to_be_bytes()).await.unwrap();
        client.write_all(&patterned(10)).await.unwrap();
        drop(client);

        let err = read_body(&mut server, TEST_LIMITS).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn stalled_sender_times_out() {
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        client.write_all(&1000u32.to_be_bytes()).await.unwrap();
        client.write_all(&patterned(10)).await.unwrap();
        // `client` stays open but sends nothing more.

        let err = read_body(&mut server, TEST_LIMITS).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn slow_but_steady_sender_is_not_cut_off() {
        // Takes ~300ms in total, longer than the 100ms idle timeout, but never
        // pauses for longer than 30ms.
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        let writer = tokio::spawn(async move {
            client.write_all(&100u32.to_be_bytes()).await.unwrap();
            for piece in patterned(100).chunks(10) {
                tokio::time::sleep(Duration::from_millis(30)).await;
                client.write_all(piece).await.unwrap();
            }
            client
        });

        let received = read_body(&mut server, TEST_LIMITS).await;
        let _client = writer.await.unwrap();
        assert_eq!(received.unwrap(), patterned(100));
    }

    #[tokio::test]
    async fn trickling_sender_hits_the_total_cap() {
        // Each byte arrives within the idle timeout, but the whole body can't
        // finish inside the 200ms total.
        let limits = BodyLimits {
            total: Some(Duration::from_millis(200)),
            ..TEST_LIMITS
        };
        let (mut client, mut server) = tokio::io::duplex(64 * 1024);
        tokio::spawn(async move {
            client.write_all(&100u32.to_be_bytes()).await.unwrap();
            for byte in patterned(100) {
                tokio::time::sleep(Duration::from_millis(20)).await;
                if client.write_all(&[byte]).await.is_err() {
                    return;
                }
            }
        });

        let err = read_body(&mut server, limits).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }
}
