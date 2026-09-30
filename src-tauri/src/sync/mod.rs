//! LAN clipboard sync between paired devices. Design: docs/LAN_SYNC_PLAN.md.
//!
//! This module holds the wire format: the frames devices exchange and the
//! encryption that protects their contents.

use std::fmt;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chacha20poly1305::{
    aead::{Aead, Generate, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

pub mod pairing;
pub mod service;
pub mod store;
pub mod transport;

/// Symmetric key shared by two paired devices.
pub type PeerKey = [u8; 32];

/// What a clip carries once decrypted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipPayload {
    pub text: String,
    pub content_type: String,
}

/// An encrypted value. Binary fields are base64 so the JSON stays compact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sealed {
    pub nonce: String,
    pub ciphertext: String,
}

/// A message on the wire. See `pairing` for the order of the `Pair*` frames.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Frame {
    Clip { from: String, sealed: Sealed },
    PairRequest { from: String },
    PairChallenge { from: String },
    PairSpake { spake_msg: String },
    PairAccept { spake_msg: String, sealed: Sealed },
    PairFinish { sealed: Sealed },
    PairDone,
}

#[derive(Debug, PartialEq)]
pub enum SyncError {
    /// Bytes that aren't a well-formed frame or payload.
    Malformed(String),
    /// Wrong key, wrong sender, or tampered ciphertext.
    Decrypt,
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SyncError::Malformed(reason) => write!(f, "malformed sync frame: {reason}"),
            SyncError::Decrypt => write!(f, "sync frame failed to decrypt"),
        }
    }
}

impl std::error::Error for SyncError {}

impl Sealed {
    /// Encrypt `value` under `key` with a fresh nonce. `aad` is authenticated
    /// but not encrypted: opening with a different `aad` fails.
    pub fn seal<T: Serialize>(key: &PeerKey, aad: &str, value: &T) -> Result<Sealed, SyncError> {
        let plaintext =
            serde_json::to_vec(value).map_err(|e| SyncError::Malformed(e.to_string()))?;
        let nonce = Nonce::generate();
        let ciphertext = cipher(key)?
            .encrypt(
                &nonce,
                Payload {
                    msg: &plaintext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SyncError::Decrypt)?;

        Ok(Sealed {
            nonce: BASE64.encode(nonce),
            ciphertext: BASE64.encode(ciphertext),
        })
    }

    pub fn open<T: DeserializeOwned>(&self, key: &PeerKey, aad: &str) -> Result<T, SyncError> {
        let nonce_bytes = decode_base64(&self.nonce)?;
        let nonce = Nonce::try_from(nonce_bytes.as_slice())
            .map_err(|_| SyncError::Malformed("bad nonce length".into()))?;
        let ciphertext = decode_base64(&self.ciphertext)?;

        let plaintext = cipher(key)?
            .decrypt(
                &nonce,
                Payload {
                    msg: &ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| SyncError::Decrypt)?;

        serde_json::from_slice(&plaintext).map_err(|e| SyncError::Malformed(e.to_string()))
    }
}

impl Frame {
    /// Encrypt `clip` for the peer sharing `key`. The sender id is bound as
    /// associated data, so a frame can't be replayed under another device's id.
    pub fn seal_clip(from: &str, key: &PeerKey, clip: &ClipPayload) -> Result<Frame, SyncError> {
        Ok(Frame::Clip {
            from: from.to_string(),
            sealed: Sealed::seal(key, from, clip)?,
        })
    }

    pub fn open_clip(&self, key: &PeerKey) -> Result<ClipPayload, SyncError> {
        match self {
            Frame::Clip { from, sealed } => sealed.open(key, from),
            _ => Err(SyncError::Malformed("not a clip frame".into())),
        }
    }
}

fn cipher(key: &PeerKey) -> Result<ChaCha20Poly1305, SyncError> {
    ChaCha20Poly1305::new_from_slice(key).map_err(|_| SyncError::Malformed("bad key".into()))
}

fn decode_base64(value: &str) -> Result<Vec<u8>, SyncError> {
    BASE64
        .decode(value)
        .map_err(|e| SyncError::Malformed(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: PeerKey = [7; 32];

    fn clip() -> ClipPayload {
        ClipPayload {
            text: "hello from the laptop".into(),
            content_type: "text".into(),
        }
    }

    fn sealed_frame() -> Frame {
        Frame::seal_clip("device-a", &KEY, &clip()).unwrap()
    }

    fn parts(frame: Frame) -> (String, Sealed) {
        match frame {
            Frame::Clip { from, sealed } => (from, sealed),
            other => panic!("expected a clip frame, got {other:?}"),
        }
    }

    #[test]
    fn sealed_clip_opens_with_the_same_key() {
        assert_eq!(sealed_frame().open_clip(&KEY), Ok(clip()));
    }

    #[test]
    fn sealed_clip_does_not_contain_the_plaintext() {
        let json = serde_json::to_string(&sealed_frame()).unwrap();
        assert!(!json.contains("hello from the laptop"));
    }

    #[test]
    fn wrong_key_is_rejected() {
        assert_eq!(sealed_frame().open_clip(&[8; 32]), Err(SyncError::Decrypt));
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let (from, sealed) = parts(sealed_frame());
        let mut bytes = BASE64.decode(sealed.ciphertext).unwrap();
        bytes[0] ^= 1;
        let tampered = Frame::Clip {
            from,
            sealed: Sealed {
                nonce: sealed.nonce,
                ciphertext: BASE64.encode(bytes),
            },
        };

        assert_eq!(tampered.open_clip(&KEY), Err(SyncError::Decrypt));
    }

    #[test]
    fn frame_relabelled_with_another_sender_is_rejected() {
        let (_, sealed) = parts(sealed_frame());
        let spoofed = Frame::Clip {
            from: "device-b".into(),
            sealed,
        };

        assert_eq!(spoofed.open_clip(&KEY), Err(SyncError::Decrypt));
    }

    #[test]
    fn non_clip_frame_does_not_open_as_a_clip() {
        let frame = Frame::PairDone;
        assert!(matches!(frame.open_clip(&KEY), Err(SyncError::Malformed(_))));
    }

    #[test]
    fn each_seal_uses_a_fresh_nonce() {
        assert_ne!(sealed_frame(), sealed_frame());
    }

    #[test]
    fn frame_survives_a_json_round_trip() {
        let frame = sealed_frame();
        let json = serde_json::to_vec(&frame).unwrap();
        assert_eq!(serde_json::from_slice::<Frame>(&json).unwrap(), frame);
    }

    #[test]
    fn frame_json_is_tagged_with_its_type() {
        let json = serde_json::to_value(sealed_frame()).unwrap();
        assert_eq!(json["type"], "Clip");
    }
}
