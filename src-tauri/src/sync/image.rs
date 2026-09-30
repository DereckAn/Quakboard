//! Sealing an image for a paired device: a `Frame::Image` header with the
//! image's details, and the PNG itself as a raw encrypted body (`body.rs`).
//!
//! The body's associated data binds it to its sender and to the SHA-256 in
//! its header, so a body can't be moved under another image's header or
//! another device's id. After decrypting, the receiver re-checks length and
//! hash, so a header can't describe different bytes than it carries.

use std::io::Cursor;

use chacha20poly1305::{
    aead::{Aead, Generate, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use image::{error::ImageError, ImageFormat, ImageReader, Limits};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{body::MAX_IMAGE_BYTES, Frame, PeerKey, Sealed, SyncError};

const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;

/// Widest or tallest image accepted, well past any real screen.
const MAX_DIMENSION: u32 = 16_384;
/// Memory a decoded image may take as RGBA. A small PNG can declare a huge
/// image (a "decompression bomb"), so this is checked before allocating.
const MAX_RGBA_BYTES: u64 = 256 * 1024 * 1024;
pub const TOO_LARGE_TO_DECODE: &str = "image is too large to decode";

/// A received PNG that decoded safely. The pixels themselves are dropped;
/// storing uses the original PNG bytes, so its hash matches the sender's.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    /// See `pixel_hash`.
    pub pixel_hash: String,
}

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

/// Check that `png` really is a PNG of the size its header claims, without
/// letting it exhaust memory. CPU-heavy for big images: callers run it off
/// the async workers.
pub fn decode_png(png: &[u8], meta: &ImageMeta) -> Result<DecodedImage, SyncError> {
    // Forcing the PNG decoder rejects every other format, even valid images.
    let mut reader = ImageReader::with_format(Cursor::new(png), ImageFormat::Png);
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_RGBA_BYTES);
    reader.limits(limits);

    let image = reader.decode().map_err(|e| match e {
        ImageError::Limits(_) => SyncError::Malformed(TOO_LARGE_TO_DECODE.into()),
        other => SyncError::Malformed(format!("not a usable PNG: {other}")),
    })?;

    let (width, height) = (image.width(), image.height());
    if (width, height) != (meta.width, meta.height) {
        return Err(SyncError::Integrity(format!(
            "image is {width}x{height}, header says {}x{}",
            meta.width, meta.height
        )));
    }
    // The decode limit doesn't cover converting to RGBA, which can quadruple
    // the size of a grayscale image; check before converting.
    if u64::from(width) * u64::from(height) * 4 > MAX_RGBA_BYTES {
        return Err(SyncError::Malformed(TOO_LARGE_TO_DECODE.into()));
    }

    let rgba = image.into_rgba8();
    Ok(DecodedImage {
        width,
        height,
        pixel_hash: pixel_hash(width, height, rgba.as_raw()),
    })
}

/// Encode clipboard pixels (8-bit RGBA, row by row) as a PNG in memory, for
/// sending an image that isn't saved locally. CPU-heavy for big images:
/// callers run it off the async workers.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, SyncError> {
    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| SyncError::Malformed("pixel buffer doesn't match its size".into()))?;
    let mut png = Vec::new();
    image::DynamicImage::from(image)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .map_err(|e| SyncError::Malformed(format!("could not encode PNG: {e}")))?;
    Ok(png)
}

/// Identifies an image by its pixels rather than its file bytes: the same
/// picture re-encoded on its way through a clipboard keeps its pixel hash.
/// `rgba` is 8-bit RGBA, row by row, as `arboard::ImageData` holds it.
pub fn pixel_hash(width: u32, height: u32, rgba: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(width.to_le_bytes());
    hasher.update(height.to_le_bytes());
    hasher.update(rgba);
    format!("{:x}", hasher.finalize())
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

    fn encode(image: image::DynamicImage, format: ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        image.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
        bytes
    }

    /// A 3x2 RGBA image with every pixel different.
    fn rgba_pixels() -> Vec<u8> {
        (0..3 * 2 * 4).map(|i| (i * 11) as u8).collect()
    }

    fn rgba_png() -> Vec<u8> {
        let image = image::RgbaImage::from_raw(3, 2, rgba_pixels()).unwrap();
        encode(image.into(), ImageFormat::Png)
    }

    fn meta_for(width: u32, height: u32) -> ImageMeta {
        ImageMeta {
            width,
            height,
            byte_len: 0,
            sha256: String::new(),
            is_screenshot: false,
        }
    }

    /// A PNG of a few dozen bytes claiming `width` x `height`: tiny on the
    /// wire, enormous if a decoder believed it and allocated.
    fn png_claiming(width: u32, height: u32) -> Vec<u8> {
        fn crc32(bytes: &[u8]) -> u32 {
            let mut crc = 0xFFFF_FFFFu32;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
                }
            }
            !crc
        }
        fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
            let mut out = (data.len() as u32).to_be_bytes().to_vec();
            let mut body = kind.to_vec();
            body.extend_from_slice(data);
            out.extend_from_slice(&body);
            out.extend_from_slice(&crc32(&body).to_be_bytes());
            out
        }

        let mut header = Vec::new();
        header.extend_from_slice(&width.to_be_bytes());
        header.extend_from_slice(&height.to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA, no interlace

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        png.extend(chunk(b"IHDR", &header));
        // A few bytes of "image data": decoders read up to the first IDAT
        // before sizing the image, and a real bomb always has one.
        png.extend(chunk(b"IDAT", &[0x78, 0x9c, 0x03, 0x00]));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn decoded_png_has_the_pixel_hash_of_its_raw_pixels() {
        let decoded = decode_png(&rgba_png(), &meta_for(3, 2)).unwrap();
        assert_eq!(decoded.pixel_hash, pixel_hash(3, 2, &rgba_pixels()));
    }

    #[test]
    fn png_without_alpha_hashes_like_its_opaque_rgba_pixels() {
        let rgb: Vec<u8> = (0..3 * 2 * 3).map(|i| (i * 7) as u8).collect();
        let opaque_rgba: Vec<u8> = rgb.chunks(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let png = encode(
            image::RgbImage::from_raw(3, 2, rgb).unwrap().into(),
            ImageFormat::Png,
        );

        let decoded = decode_png(&png, &meta_for(3, 2)).unwrap();
        assert_eq!(decoded.pixel_hash, pixel_hash(3, 2, &opaque_rgba));
    }

    #[test]
    fn valid_jpeg_is_rejected() {
        let rgb = image::RgbImage::from_pixel(3, 2, image::Rgb([10, 20, 30]));
        let jpeg = encode(rgb.into(), ImageFormat::Jpeg);

        assert!(matches!(
            decode_png(&jpeg, &meta_for(3, 2)),
            Err(SyncError::Malformed(_))
        ));
    }

    #[test]
    fn garbage_is_rejected() {
        assert!(matches!(
            decode_png(b"definitely not a picture", &meta_for(3, 2)),
            Err(SyncError::Malformed(_))
        ));
    }

    #[test]
    fn truncated_png_is_rejected() {
        let png = rgba_png();
        assert!(matches!(
            decode_png(&png[..png.len() / 2], &meta_for(3, 2)),
            Err(SyncError::Malformed(_))
        ));
    }

    #[test]
    fn too_wide_image_is_rejected_before_decoding() {
        assert_eq!(
            decode_png(&png_claiming(MAX_DIMENSION + 1, 1), &meta_for(MAX_DIMENSION + 1, 1)),
            Err(SyncError::Malformed(TOO_LARGE_TO_DECODE.into()))
        );
    }

    #[test]
    fn decompression_bomb_is_rejected_before_allocating() {
        // Within the dimension cap, but 9000x9000 RGBA is ~324 MB.
        assert_eq!(
            decode_png(&png_claiming(9000, 9000), &meta_for(9000, 9000)),
            Err(SyncError::Malformed(TOO_LARGE_TO_DECODE.into()))
        );
    }

    #[test]
    fn grayscale_image_too_large_once_converted_to_rgba_is_rejected() {
        // 9000x9000 grayscale decodes within the limit (81 MB), but its RGBA
        // form (~324 MB) wouldn't; the conversion must be refused.
        let gray = image::GrayImage::new(9000, 9000);
        let png = encode(gray.into(), ImageFormat::Png);

        assert_eq!(
            decode_png(&png, &meta_for(9000, 9000)),
            Err(SyncError::Malformed(TOO_LARGE_TO_DECODE.into()))
        );
    }

    #[test]
    fn size_that_differs_from_the_header_fails_the_integrity_check() {
        assert!(matches!(
            decode_png(&rgba_png(), &meta_for(2, 3)),
            Err(SyncError::Integrity(_))
        ));
    }

    #[test]
    fn one_changed_pixel_changes_the_pixel_hash() {
        let mut changed = rgba_pixels();
        changed[0] ^= 1;
        assert_ne!(pixel_hash(3, 2, &changed), pixel_hash(3, 2, &rgba_pixels()));
    }

    #[test]
    fn same_bytes_at_another_size_have_another_pixel_hash() {
        assert_ne!(pixel_hash(2, 3, &rgba_pixels()), pixel_hash(3, 2, &rgba_pixels()));
    }

    #[test]
    fn encoded_png_decodes_to_the_same_pixels() {
        let png = encode_png(3, 2, &rgba_pixels()).unwrap();
        let decoded = decode_png(&png, &meta_for(3, 2)).unwrap();

        assert_eq!(decoded.pixel_hash, pixel_hash(3, 2, &rgba_pixels()));
    }

    #[test]
    fn pixels_that_do_not_fill_the_size_are_rejected() {
        assert!(matches!(
            encode_png(3, 3, &rgba_pixels()),
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
