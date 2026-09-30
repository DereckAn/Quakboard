//! Sealing an image for a paired device: a `Frame::Image` header with the
//! image's details, and the PNG itself as a raw encrypted body (`body.rs`).
//!
//! The body's associated data binds it to its sender and to the SHA-256 in
//! its header, so a body can't be moved under another image's header or
//! another device's id. After decrypting, the receiver re-checks length and
//! hash, so a header can't describe different bytes than it carries.
// NOTE: image sync is built in steps (docs/IMAGE_SYNC_PLAN.md); nothing seals
// or opens images until step 5. Drop this allow then.
#![allow(dead_code)]

use chacha20poly1305::{
    aead::{Aead, Generate, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{body::MAX_IMAGE_BYTES, Frame, PeerKey, Sealed, SyncError};

const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// What the sender knows about an image besides its bytes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageDetails {
    pub width: u32,
    pub height: u32,
    pub is_screenshot: bool,
}

/// The sealed header of an image. `sha256` is lowercase hex, the same format
/// `image_handler::calculate_file_hash` stores, so the receiver can look the
/// image up in its history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageMeta {
    pub width: u32,
    pub height: u32,
    pub byte_len: u64,
    pub sha256: String,
    pub is_screenshot: bool,
}

/// Seal `png` for the peer sharing `key`. Hash and length are computed here,
/// never taken from the caller, so the header always matches the body.
/// Returns the frame to send, then the body to send right after it.
pub fn seal_image(
    from: &str,
    key: &PeerKey,
    details: ImageDetails,
    png: &[u8],
) -> Result<(Frame, Vec<u8>), SyncError> {
    if png.len() > MAX_IMAGE_BYTES {
        return Err(SyncError::Malformed(format!(
            "image is {} bytes, over the {MAX_IMAGE_BYTES}-byte limit",
            png.len()
        )));
    }

    let meta = ImageMeta {
        width: details.width,
        height: details.height,
        byte_len: png.len() as u64,
        sha256: sha256_hex(png),
        is_screenshot: details.is_screenshot,
    };
    let body = seal_bytes(key, &body_aad(from, &meta.sha256), png)?;
    let frame = Frame::Image {
        from: from.to_string(),
        meta: Sealed::seal(key, &meta_aad(from), &meta)?,
    };
    Ok((frame, body))
}

/// Open an image frame and its body. Everything here came off the network.
pub fn open_image(
    frame: &Frame,
    body: &[u8],
    key: &PeerKey,
) -> Result<(ImageMeta, Vec<u8>), SyncError> {
    let Frame::Image { from, meta } = frame else {
        return Err(SyncError::Malformed("not an image frame".into()));
    };

    let meta: ImageMeta = meta.open(key, &meta_aad(from))?;
    if meta.byte_len > MAX_IMAGE_BYTES as u64 {
        return Err(SyncError::Malformed(
            "image header claims an oversized image".into(),
        ));
    }

    let png = open_bytes(key, &body_aad(from, &meta.sha256), body)?;
    if png.len() as u64 != meta.byte_len {
        return Err(SyncError::Integrity(format!(
            "image is {} bytes, header says {}",
            png.len(),
            meta.byte_len
        )));
    }
    if sha256_hex(&png) != meta.sha256 {
        return Err(SyncError::Integrity(
            "image hash doesn't match its header".into(),
        ));
    }
    Ok((meta, png))
}

fn meta_aad(from: &str) -> String {
    format!("image-meta:{from}")
}

fn body_aad(from: &str, sha256: &str) -> String {
    format!("image-body:{from}:{sha256}")
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Raw-bytes counterpart of `Sealed`: nonce followed by ciphertext, no base64.
fn seal_bytes(key: &PeerKey, aad: &str, plaintext: &[u8]) -> Result<Vec<u8>, SyncError> {
    let nonce = Nonce::generate();
    let ciphertext = cipher(key)?
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| SyncError::Decrypt)?;

    let mut sealed = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
    sealed.extend_from_slice(&nonce);
    sealed.extend_from_slice(&ciphertext);
    Ok(sealed)
}

fn open_bytes(key: &PeerKey, aad: &str, sealed: &[u8]) -> Result<Vec<u8>, SyncError> {
    if sealed.len() < NONCE_BYTES + TAG_BYTES {
        return Err(SyncError::Malformed("sealed body too short".into()));
    }
    let (nonce, ciphertext) = sealed.split_at(NONCE_BYTES);
    let nonce = Nonce::try_from(nonce).map_err(|_| SyncError::Malformed("bad nonce".into()))?;

    cipher(key)?
        .decrypt(
            &nonce,
            Payload {
                msg: ciphertext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| SyncError::Decrypt)
}

fn cipher(key: &PeerKey) -> Result<ChaCha20Poly1305, SyncError> {
    ChaCha20Poly1305::new_from_slice(key).map_err(|_| SyncError::Malformed("bad key".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: PeerKey = [9; 32];
    const DETAILS: ImageDetails = ImageDetails {
        width: 640,
        height: 480,
        is_screenshot: true,
    };

    /// Stand-in PNG bytes; sealing never parses them (decoding is step 3).
    fn png(seed: u8) -> Vec<u8> {
        (0..2048).map(|i| (i as u8).wrapping_mul(seed)).collect()
    }

    fn sealed(seed: u8) -> (Frame, Vec<u8>) {
        seal_image("device-a", &KEY, DETAILS, &png(seed)).unwrap()
    }

    fn from_and_meta(frame: Frame) -> (String, Sealed) {
        match frame {
            Frame::Image { from, meta } => (from, meta),
            other => panic!("expected an image frame, got {other:?}"),
        }
    }

    #[test]
    fn sealed_image_opens_to_the_same_bytes() {
        let (frame, body) = sealed(3);
        let (_, opened) = open_image(&frame, &body, &KEY).unwrap();
        assert_eq!(opened, png(3));
    }

    #[test]
    fn header_describes_the_image() {
        let (frame, body) = sealed(3);
        let (meta, _) = open_image(&frame, &body, &KEY).unwrap();
        assert_eq!(
            meta,
            ImageMeta {
                width: 640,
                height: 480,
                byte_len: 2048,
                sha256: sha256_hex(&png(3)),
                is_screenshot: true,
            }
        );
    }

    #[test]
    fn body_does_not_contain_the_plaintext() {
        let (_, body) = sealed(3);
        assert!(!body.windows(64).any(|window| window == &png(3)[..64]));
    }

    #[test]
    fn wrong_key_is_rejected() {
        let (frame, body) = sealed(3);
        assert_eq!(open_image(&frame, &body, &[1; 32]), Err(SyncError::Decrypt));
    }

    #[test]
    fn tampered_body_is_rejected() {
        let (frame, mut body) = sealed(3);
        let last = body.len() - 1;
        body[last] ^= 1;

        assert_eq!(open_image(&frame, &body, &KEY), Err(SyncError::Decrypt));
    }

    #[test]
    fn body_swapped_from_another_image_is_rejected() {
        let (frame, _) = sealed(3);
        let (_, other_body) = sealed(5);

        assert_eq!(
            open_image(&frame, &other_body, &KEY),
            Err(SyncError::Decrypt)
        );
    }

    #[test]
    fn frame_relabelled_with_another_sender_is_rejected() {
        let (frame, body) = sealed(3);
        let (_, meta) = from_and_meta(frame);
        let spoofed = Frame::Image {
            from: "device-b".into(),
            meta,
        };

        assert_eq!(open_image(&spoofed, &body, &KEY), Err(SyncError::Decrypt));
    }

    /// The honest header for `png(seed)`.
    fn meta_of(seed: u8) -> ImageMeta {
        let (frame, body) = sealed(seed);
        open_image(&frame, &body, &KEY).unwrap().0
    }

    /// What a paired device that seals honestly but lies in its header would
    /// send: `meta` sealed correctly, and `png` bound to `meta`'s hash.
    fn forged(meta: &ImageMeta, png: &[u8]) -> (Frame, Vec<u8>) {
        let frame = Frame::Image {
            from: "device-a".into(),
            meta: Sealed::seal(&KEY, &meta_aad("device-a"), meta).unwrap(),
        };
        let body = seal_bytes(&KEY, &body_aad("device-a", &meta.sha256), png).unwrap();
        (frame, body)
    }

    #[test]
    fn header_with_the_wrong_hash_fails_the_integrity_check() {
        let lie = ImageMeta {
            sha256: sha256_hex(b"something else"),
            ..meta_of(3)
        };
        let (frame, body) = forged(&lie, &png(3));

        assert!(matches!(
            open_image(&frame, &body, &KEY),
            Err(SyncError::Integrity(_))
        ));
    }

    #[test]
    fn header_with_the_wrong_length_fails_the_integrity_check() {
        let lie = ImageMeta {
            byte_len: 1,
            ..meta_of(3)
        };
        let (frame, body) = forged(&lie, &png(3));

        assert!(matches!(
            open_image(&frame, &body, &KEY),
            Err(SyncError::Integrity(_))
        ));
    }

    #[test]
    fn header_claiming_an_oversized_image_is_rejected() {
        let lie = ImageMeta {
            byte_len: MAX_IMAGE_BYTES as u64 + 1,
            ..meta_of(3)
        };
        let (frame, body) = forged(&lie, &png(3));

        assert!(matches!(
            open_image(&frame, &body, &KEY),
            Err(SyncError::Malformed(_))
        ));
    }

    #[test]
    fn oversized_image_is_refused_when_sealing() {
        let too_big = vec![0u8; MAX_IMAGE_BYTES + 1];
        assert!(matches!(
            seal_image("device-a", &KEY, DETAILS, &too_big),
            Err(SyncError::Malformed(_))
        ));
    }

    #[test]
    fn body_too_short_to_hold_a_nonce_and_tag_is_rejected() {
        let (frame, _) = sealed(3);
        assert!(matches!(
            open_image(&frame, &[0; 20], &KEY),
            Err(SyncError::Malformed(_))
        ));
    }

    #[test]
    fn non_image_frame_is_rejected() {
        assert!(matches!(
            open_image(&Frame::PairDone, &[], &KEY),
            Err(SyncError::Malformed(_))
        ));
    }

    #[test]
    fn hash_matches_the_format_stored_in_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        std::fs::write(&path, png(3)).unwrap();
        let stored = crate::clipboard::image_handler::calculate_file_hash(&path).unwrap();

        assert_eq!(sha256_hex(&png(3)), stored);
    }
}
