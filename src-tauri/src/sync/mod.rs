//! LAN clipboard sync between paired devices. Design: docs/LAN_SYNC_PLAN.md.
//!
//! The wire format, pairing, discovery and transfer live in the
//! `quakboard-sync` crate; this module wires them into the app.

pub use quakboard_sync::{
    body, discovery, fetch, fetching, image, pairing, protocol, store, stream, transport,
    ClipPayload, Frame, PeerKey, SyncError,
};

pub mod offers;
pub mod received;
pub mod remote_files;
pub mod runtime;
pub mod service;
pub mod sharing;
