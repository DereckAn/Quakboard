//! This device's sync identity and the devices it is paired with, persisted in
//! `<app_data_dir>/sync.json`. The file holds pairing keys, so it is written
//! owner-only and atomically, and a file that fails to parse is never replaced.

use std::{fmt, fs, io, path::Path};

use serde::{Deserialize, Serialize};

use super::PeerKey;

pub const STORE_FILE_NAME: &str = "sync.json";

const FALLBACK_DEVICE_NAME: &str = "Quakboard device";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    #[serde(with = "key_base64")]
    pub key: PeerKey,
    /// Last `ip:port` this peer was reached at; tried before an mDNS lookup.
    pub last_addr: Option<String>,
}

// Hand-written so a peer's key never ends up in logs.
impl fmt::Debug for Peer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Peer")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("key", &"<redacted>")
            .field("last_addr", &self.last_addr)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SyncStore {
    pub device_id: String,
    pub device_name: String,
    pub peers: Vec<Peer>,
}

#[derive(Debug)]
pub enum StoreError {
    Io(io::Error),
    /// The file exists but isn't valid; left untouched so pairings aren't lost.
    Corrupt(serde_json::Error),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StoreError::Io(e) => write!(f, "sync store I/O failed: {e}"),
            StoreError::Corrupt(e) => write!(f, "sync store is corrupt: {e}"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<io::Error> for StoreError {
    fn from(e: io::Error) -> Self {
        StoreError::Io(e)
    }
}

impl SyncStore {
    pub fn new(device_name: String) -> Self {
        SyncStore {
            device_id: uuid::Uuid::new_v4().to_string(),
            device_name,
            peers: Vec::new(),
        }
    }

    /// Load the store, or create and save a fresh identity on first run.
    pub fn load_or_create(path: &Path) -> Result<Self, StoreError> {
        match fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(StoreError::Corrupt),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let store = SyncStore::new(default_device_name());
                store.save(path)?;
                Ok(store)
            }
            Err(e) => Err(StoreError::Io(e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        let json = serde_json::to_vec_pretty(self).map_err(StoreError::Corrupt)?;
        let tmp = path.with_extension("json.tmp");
        // A stale tmp file would keep its old permissions; start clean.
        let _ = fs::remove_file(&tmp);
        write_owner_only(&tmp, &json)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn peer(&self, id: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.id == id)
    }

    /// Add a peer, or replace it when re-pairing with the same device.
    pub fn upsert_peer(&mut self, peer: Peer) {
        match self.peers.iter_mut().find(|p| p.id == peer.id) {
            Some(existing) => *existing = peer,
            None => self.peers.push(peer),
        }
    }

    /// Returns whether a peer was removed.
    pub fn remove_peer(&mut self, id: &str) -> bool {
        let before = self.peers.len();
        self.peers.retain(|p| p.id != id);
        self.peers.len() != before
    }

    /// Returns whether the peer exists.
    pub fn set_last_addr(&mut self, id: &str, addr: &str) -> bool {
        match self.peers.iter_mut().find(|p| p.id == id) {
            Some(peer) => {
                peer.last_addr = Some(addr.to_string());
                true
            }
            None => false,
        }
    }
}

fn default_device_name() -> String {
    hostname()
        .map(|name| name.trim_end_matches(".local").to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| FALLBACK_DEVICE_NAME.to_string())
}

#[cfg(unix)]
fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the pointer and length describe `buf`, which outlives the call.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec()).ok()
}

#[cfg(windows)]
fn hostname() -> Option<String> {
    std::env::var("COMPUTERNAME").ok()
}

#[cfg(unix)]
fn write_owner_only(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};

    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

// NOTE: files under the Windows per-user AppData dir are already private to
// that user by default ACLs.
#[cfg(windows)]
fn write_owner_only(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

mod key_base64 {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
    use serde::{de, Deserialize, Deserializer, Serializer};

    use crate::sync::PeerKey;

    pub fn serialize<S: Serializer>(key: &PeerKey, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&BASE64.encode(key))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<PeerKey, D::Error> {
        let text = String::deserialize(deserializer)?;
        let bytes = BASE64.decode(text).map_err(de::Error::custom)?;
        bytes
            .try_into()
            .map_err(|_| de::Error::custom("peer key must be 32 bytes"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer(id: &str) -> Peer {
        Peer {
            id: id.into(),
            name: format!("{id} laptop"),
            key: [3; 32],
            last_addr: None,
        }
    }

    fn store_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
        dir.path().join(STORE_FILE_NAME)
    }

    #[test]
    fn first_load_creates_the_file() {
        let dir = tempfile::tempdir().unwrap();
        SyncStore::load_or_create(&store_path(&dir)).unwrap();
        assert!(store_path(&dir).exists());
    }

    #[test]
    fn first_load_generates_a_uuid_device_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = SyncStore::load_or_create(&store_path(&dir)).unwrap();
        assert!(uuid::Uuid::parse_str(&store.device_id).is_ok());
    }

    #[test]
    fn first_load_names_the_device() {
        let dir = tempfile::tempdir().unwrap();
        let store = SyncStore::load_or_create(&store_path(&dir)).unwrap();
        assert!(!store.device_name.is_empty());
    }

    #[test]
    fn device_id_is_stable_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let first = SyncStore::load_or_create(&store_path(&dir)).unwrap();
        let second = SyncStore::load_or_create(&store_path(&dir)).unwrap();
        assert_eq!(first.device_id, second.device_id);
    }

    #[test]
    fn saved_peers_are_reloaded_with_their_keys() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = SyncStore::new("desk".into());
        store.upsert_peer(peer("a"));
        store.save(&store_path(&dir)).unwrap();

        assert_eq!(SyncStore::load_or_create(&store_path(&dir)).unwrap(), store);
    }

    #[test]
    fn upsert_replaces_a_peer_with_the_same_id() {
        let mut store = SyncStore::new("desk".into());
        store.upsert_peer(peer("a"));
        store.upsert_peer(Peer {
            key: [9; 32],
            ..peer("a")
        });

        assert_eq!(
            store.peers,
            vec![Peer {
                key: [9; 32],
                ..peer("a")
            }]
        );
    }

    #[test]
    fn remove_peer_drops_only_that_peer() {
        let mut store = SyncStore::new("desk".into());
        store.upsert_peer(peer("a"));
        store.upsert_peer(peer("b"));
        store.remove_peer("a");

        assert_eq!(store.peers, vec![peer("b")]);
    }

    #[test]
    fn removing_an_unknown_peer_reports_false() {
        let mut store = SyncStore::new("desk".into());
        assert!(!store.remove_peer("nobody"));
    }

    #[test]
    fn set_last_addr_updates_the_peer() {
        let mut store = SyncStore::new("desk".into());
        store.upsert_peer(peer("a"));
        store.set_last_addr("a", "192.168.1.20:47823");

        assert_eq!(
            store.peer("a").unwrap().last_addr.as_deref(),
            Some("192.168.1.20:47823")
        );
    }

    #[test]
    fn corrupt_file_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(store_path(&dir), b"{ not json").unwrap();

        assert!(matches!(
            SyncStore::load_or_create(&store_path(&dir)),
            Err(StoreError::Corrupt(_))
        ));
    }

    #[test]
    fn corrupt_file_is_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(store_path(&dir), b"{ not json").unwrap();
        let _ = SyncStore::load_or_create(&store_path(&dir));

        assert_eq!(fs::read(store_path(&dir)).unwrap(), b"{ not json");
    }

    #[test]
    fn key_of_the_wrong_length_is_rejected() {
        let json = r#"{"id":"a","name":"a","key":"AAAA","last_addr":null}"#;
        assert!(serde_json::from_str::<Peer>(json).is_err());
    }

    #[test]
    fn debug_output_hides_the_key() {
        // A derived Debug would print the key bytes as `[3, 3, 3, ...]`.
        assert!(!format!("{:?}", peer("a")).contains("[3, 3"));
    }

    #[cfg(unix)]
    #[test]
    fn saved_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        SyncStore::new("desk".into())
            .save(&store_path(&dir))
            .unwrap();
        let mode = fs::metadata(store_path(&dir)).unwrap().permissions().mode();

        assert_eq!(mode & 0o777, 0o600);
    }
}
