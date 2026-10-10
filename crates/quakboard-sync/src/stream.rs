//! Streaming a file of any size between paired devices, in encrypted chunks.
//!
//! The STREAM construction: every transfer gets a random salt and its own key
//! derived from the peer key, so chunk numbers can serve as nonces without
//! ever repeating under one key. Each chunk's associated data carries the
//! transfer's context, its number and a "last" flag, so chunks can't be
//! reordered, dropped, replayed, spliced from another transfer, or cut short
//! (a stream only ends on an authenticated last chunk).
//!
//! On the wire each chunk is a `body.rs` body: a flag byte (1 = last), then
//! the ciphertext. Nothing is held in memory beyond one chunk.

use std::{io, time::Duration};

use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::{
    body::{read_body, write_body, BodyLimits},
    PeerKey,
};

/// Plaintext bytes per chunk.
pub const CHUNK_BYTES: usize = 256 * 1024;
pub const SALT_BYTES: usize = 16;
const TAG_BYTES: usize = 16;

/// A fresh random salt for one transfer. Never reuse one.
pub fn new_salt() -> io::Result<[u8; SALT_BYTES]> {
    let mut salt = [0u8; SALT_BYTES];
    getrandom::fill(&mut salt).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(salt)
}

/// What a finished stream carried.
#[derive(Debug, Clone, PartialEq)]
pub struct StreamSummary {
    pub byte_len: u64,
    /// Lowercase hex SHA-256 of the plaintext.
    pub sha256: String,
}

/// Encrypt everything `source` yields and send it on `sink`. `context`
/// names the transfer (offer and sender); the receiver must pass the same.
///
/// Reads `source` to its end: callers pass a file limited to the offered
/// size (`File::take`), or a file still being written streams forever.
pub async fn send_stream<R, W>(
    source: &mut R,
    sink: &mut W,
    peer_key: &PeerKey,
    salt: &[u8; SALT_BYTES],
    context: &str,
    idle: Duration,
) -> io::Result<StreamSummary>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let cipher = stream_cipher(peer_key, salt);
    let limits = chunk_limits(idle);
    let mut hasher = Sha256::new();
    let mut byte_len = 0u64;

    // Read one chunk ahead: a chunk is the last one when nothing follows it.
    let mut current = read_chunk(source).await?;
    for index in 0u64.. {
        let next = read_chunk(source).await?;
        let is_last = next.is_empty();

        hasher.update(&current);
        byte_len += current.len() as u64;
        let ciphertext = cipher
            .encrypt(
                &chunk_nonce(index),
                Payload {
                    msg: &current,
                    aad: &chunk_aad(context, index, is_last),
                },
            )
            .map_err(|_| io::Error::other("could not encrypt chunk"))?;

        let mut body = Vec::with_capacity(1 + ciphertext.len());
        body.push(u8::from(is_last));
        body.extend_from_slice(&ciphertext);
        write_body(sink, &body, limits).await?;

        if is_last {
            break;
        }
        current = next;
    }
    sink.flush().await?;

    Ok(StreamSummary {
        byte_len,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

/// Receive and decrypt a stream into `sink`. Fails without writing past
/// `max_len` (the size the sender promised), and fails on any chunk that is
/// tampered, out of order, missing, or from another transfer.
/// `on_progress` gets the bytes received so far after each chunk.
#[allow(clippy::too_many_arguments)]
pub async fn receive_stream<R, W>(
    source: &mut R,
    sink: &mut W,
    peer_key: &PeerKey,
    salt: &[u8; SALT_BYTES],
    context: &str,
    max_len: u64,
    idle: Duration,
    mut on_progress: impl FnMut(u64),
) -> io::Result<StreamSummary>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let cipher = stream_cipher(peer_key, salt);
    let limits = chunk_limits(idle);
    let mut hasher = Sha256::new();
    let mut byte_len = 0u64;

    for index in 0u64.. {
        // A connection that closes before the last chunk is a cut-short file.
        let body = read_body(source, limits).await?;
        let Some((&flag, ciphertext)) = body.split_first() else {
            return Err(invalid("empty chunk"));
        };
        let is_last = match flag {
            0 => false,
            1 => true,
            _ => return Err(invalid("bad chunk flag")),
        };

        let plaintext = cipher
            .decrypt(
                &chunk_nonce(index),
                Payload {
                    msg: ciphertext,
                    aad: &chunk_aad(context, index, is_last),
                },
            )
            .map_err(|_| invalid("chunk failed to decrypt"))?;

        byte_len += plaintext.len() as u64;
        if byte_len > max_len {
            return Err(invalid("stream is longer than promised"));
        }
        hasher.update(&plaintext);
        sink.write_all(&plaintext).await?;
        on_progress(byte_len);

        if is_last {
            break;
        }
    }
    sink.flush().await?;

    Ok(StreamSummary {
        byte_len,
        sha256: format!("{:x}", hasher.finalize()),
    })
}

/// Fill up to one chunk from `source`; shorter only at the end of the file.
async fn read_chunk<R: AsyncRead + Unpin>(source: &mut R) -> io::Result<Vec<u8>> {
    let mut chunk = vec![0u8; CHUNK_BYTES];
    let mut filled = 0;
    while filled < CHUNK_BYTES {
        let read = source.read(&mut chunk[filled..]).await?;
        if read == 0 {
            break;
        }
        filled += read;
    }
    chunk.truncate(filled);
    Ok(chunk)
}

/// A key for this transfer only, so chunk-number nonces never repeat under
/// the long-term peer key.
fn stream_cipher(peer_key: &PeerKey, salt: &[u8; SALT_BYTES]) -> ChaCha20Poly1305 {
    let mut hasher = Sha256::new();
    hasher.update(b"quakboard-file-stream-v1");
    hasher.update(peer_key);
    hasher.update(salt);
    let key: [u8; 32] = hasher.finalize().into();
    ChaCha20Poly1305::new(&key.into())
}

fn chunk_nonce(index: u64) -> Nonce {
    let mut nonce = [0u8; 12];
    nonce[4..].copy_from_slice(&index.to_be_bytes());
    nonce.into()
}

fn chunk_aad(context: &str, index: u64, is_last: bool) -> Vec<u8> {
    format!("{context}:{index}:{}", u8::from(is_last)).into_bytes()
}

fn chunk_limits(idle: Duration) -> BodyLimits {
    BodyLimits {
        max_len: 1 + CHUNK_BYTES + TAG_BYTES,
        idle,
        // The user chose to fetch this file, however long it takes; only a
        // stall ends it.
        total: None,
    }
}

fn invalid(reason: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, reason.to_string())
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;

    const KEY: PeerKey = [4; 32];
    const SALT: [u8; SALT_BYTES] = [8; SALT_BYTES];
    const CONTEXT: &str = "offer-1:device-a";
    const IDLE: Duration = Duration::from_millis(200);

    /// Bytes that differ per position, so a reordered chunk can't decrypt to
    /// the same plaintext by luck.
    fn file_of(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    /// What `send_stream` puts on the wire for `file`.
    async fn wire_for(file: &[u8]) -> Vec<u8> {
        let mut wire = Vec::new();
        send_stream(&mut Cursor::new(file), &mut wire, &KEY, &SALT, CONTEXT, IDLE)
            .await
            .unwrap();
        wire
    }

    async fn receive(wire: &[u8], max_len: u64) -> io::Result<(Vec<u8>, StreamSummary)> {
        let mut out = Vec::new();
        let summary =
            receive_stream(&mut Cursor::new(wire), &mut out, &KEY, &SALT, CONTEXT, max_len, IDLE, |_| {})
                .await?;
        Ok((out, summary))
    }

    /// Split wire bytes into chunk bodies, length prefix included.
    fn chunks(wire: &[u8]) -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut rest = wire;
        while !rest.is_empty() {
            let len = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
            out.push(rest[..4 + len].to_vec());
            rest = &rest[4 + len..];
        }
        out
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    const MULTI_CHUNK: usize = 2 * CHUNK_BYTES + 1000;

    #[tokio::test]
    async fn multi_chunk_file_survives_the_trip() {
        let file = file_of(MULTI_CHUNK);
        let (received, _) = receive(&wire_for(&file).await, u64::MAX).await.unwrap();
        assert_eq!(received, file);
    }

    #[tokio::test]
    async fn file_of_exactly_whole_chunks_survives_the_trip() {
        let file = file_of(2 * CHUNK_BYTES);
        let (received, _) = receive(&wire_for(&file).await, u64::MAX).await.unwrap();
        assert_eq!(received, file);
    }

    #[tokio::test]
    async fn empty_file_survives_the_trip() {
        let (received, _) = receive(&wire_for(&[]).await, u64::MAX).await.unwrap();
        assert!(received.is_empty());
    }

    #[tokio::test]
    async fn file_is_sent_in_chunks() {
        assert_eq!(chunks(&wire_for(&file_of(MULTI_CHUNK)).await).len(), 3);
    }

    #[tokio::test]
    async fn sender_reports_the_files_hash() {
        let file = file_of(MULTI_CHUNK);
        let mut wire = Vec::new();
        let summary = send_stream(&mut Cursor::new(&file), &mut wire, &KEY, &SALT, CONTEXT, IDLE)
            .await
            .unwrap();

        assert_eq!(
            summary,
            StreamSummary {
                byte_len: MULTI_CHUNK as u64,
                sha256: sha256_hex(&file),
            }
        );
    }

    #[tokio::test]
    async fn receiver_reports_the_files_hash() {
        let file = file_of(MULTI_CHUNK);
        let (_, summary) = receive(&wire_for(&file).await, u64::MAX).await.unwrap();
        assert_eq!(summary.sha256, sha256_hex(&file));
    }

    #[tokio::test]
    async fn plaintext_is_not_on_the_wire() {
        let file = file_of(MULTI_CHUNK);
        let wire = wire_for(&file).await;
        assert!(!wire.windows(64).any(|window| window == &file[1000..1064]));
    }

    #[tokio::test]
    async fn tampered_chunk_is_rejected() {
        let mut wire = wire_for(&file_of(MULTI_CHUNK)).await;
        let middle = wire.len() / 2;
        wire[middle] ^= 1;

        let err = receive(&wire, u64::MAX).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn reordered_chunks_are_rejected() {
        let mut parts = chunks(&wire_for(&file_of(MULTI_CHUNK)).await);
        parts.swap(0, 1);

        let err = receive(&parts.concat(), u64::MAX).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn dropped_chunk_is_rejected() {
        let mut parts = chunks(&wire_for(&file_of(MULTI_CHUNK)).await);
        parts.remove(1);

        let err = receive(&parts.concat(), u64::MAX).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn replayed_chunk_is_rejected() {
        let mut parts = chunks(&wire_for(&file_of(MULTI_CHUNK)).await);
        let first = parts[0].clone();
        parts.insert(1, first);

        let err = receive(&parts.concat(), u64::MAX).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn stream_cut_short_is_rejected() {
        let mut parts = chunks(&wire_for(&file_of(MULTI_CHUNK)).await);
        parts.pop();

        let err = receive(&parts.concat(), u64::MAX).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn chunk_relabelled_as_last_is_rejected() {
        // Flipping the flag can't end a file early: the flag is authenticated.
        let mut parts = chunks(&wire_for(&file_of(MULTI_CHUNK)).await);
        parts[0][4] = 1;

        let err = receive(&parts[0], u64::MAX).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn stream_from_another_transfer_is_rejected() {
        let wire = wire_for(&file_of(1000)).await;
        let mut out = Vec::new();
        let err = receive_stream(
            &mut Cursor::new(&wire),
            &mut out,
            &KEY,
            &SALT,
            "offer-2:device-a",
            u64::MAX,
            IDLE, |_| {})
        .await
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn stream_with_another_salt_is_rejected() {
        let wire = wire_for(&file_of(1000)).await;
        let mut out = Vec::new();
        let err = receive_stream(
            &mut Cursor::new(&wire),
            &mut out,
            &KEY,
            &[9; SALT_BYTES],
            CONTEXT,
            u64::MAX,
            IDLE, |_| {})
        .await
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn stream_longer_than_promised_is_rejected() {
        let wire = wire_for(&file_of(MULTI_CHUNK)).await;
        let err = receive(&wire, CHUNK_BYTES as u64).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn nothing_past_the_promised_size_is_written() {
        let wire = wire_for(&file_of(MULTI_CHUNK)).await;
        let mut out = Vec::new();
        let _ = receive_stream(
            &mut Cursor::new(&wire),
            &mut out,
            &KEY,
            &SALT,
            CONTEXT,
            CHUNK_BYTES as u64,
            IDLE, |_| {})
        .await;

        assert!(out.len() as u64 <= CHUNK_BYTES as u64);
    }

    #[tokio::test]
    async fn stalled_sender_times_out() {
        let wire = wire_for(&file_of(MULTI_CHUNK)).await;
        let first_chunk = chunks(&wire).remove(0);
        let (mut client, mut server) = tokio::io::duplex(2 * CHUNK_BYTES);
        client.write_all(&first_chunk).await.unwrap();
        // `client` stays open but sends nothing more.

        let mut out = Vec::new();
        let err = receive_stream(&mut server, &mut out, &KEY, &SALT, CONTEXT, u64::MAX, IDLE, |_| {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn progress_is_reported_up_to_the_whole_file() {
        let wire = wire_for(&file_of(MULTI_CHUNK)).await;
        let mut seen = Vec::new();
        let mut out = Vec::new();
        receive_stream(&mut Cursor::new(&wire), &mut out, &KEY, &SALT, CONTEXT, u64::MAX, IDLE, |done| {
            seen.push(done)
        })
        .await
        .unwrap();

        assert_eq!(
            seen,
            vec![CHUNK_BYTES as u64, 2 * CHUNK_BYTES as u64, MULTI_CHUNK as u64]
        );
    }

    #[tokio::test]
    async fn every_salt_is_different() {
        assert_ne!(new_salt().unwrap(), new_salt().unwrap());
    }
}
