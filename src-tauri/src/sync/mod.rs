//! LAN clipboard sync between paired devices. Design: docs/LAN_SYNC_PLAN.md.
//!
//! The wire format, pairing, discovery and transfer live in the
//! `quakboard-sync` crate; this module wires them into the app.

pub use quakboard_sync::{
    body, discovery, fetching, image, protocol, service, store, transport, ClipPayload,
};

pub mod offers;
pub mod received;
pub mod remote_files;
pub mod runtime;
pub mod sharing;
