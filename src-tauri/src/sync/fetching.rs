//! Fetching a file another device offered: download into a `.partial` file,
//! verify size and SHA-256, and only then give it its final name, without
//! ever replacing a file that's already there.

use std::{
    fmt, io,
    path::{Path, PathBuf},
    time::Duration,
};

use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use super::{
    fetch::{
        decode_salt, open_fetch_reply, seal_fetch_request, stream_context, FetchReply,
        RefusalReason, FILE_IDLE_TIMEOUT,
    },
    remote_files::clean_file_name,
    stream::receive_stream,
    transport::{read_frame, write_frame},
    PeerKey,
};
use crate::db::models::ClipboardItem;

/// How long the owner may take to answer: it may re-hash a big file first.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
const PARTIAL_DIR: &str = ".partial";
const MAX_NAME_ATTEMPTS: usize = 1000;

/// A remote item's offer, as `remote_files` stored it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteFile {
    pub offer_id: String,
    pub origin_id: String,
    pub origin_name: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
}

impl RemoteFile {
    pub fn of(item: &ClipboardItem) -> Option<RemoteFile> {
        let metadata: Value = serde_json::from_str(&item.content_metadata).ok()?;
        let remote = metadata.get("remote")?;
        let text = |key: &str| remote.get(key)?.as_str().map(str::to_string);
        Some(RemoteFile {
            offer_id: text("offer_id")?,
            origin_id: text("origin_device_id")?,
            origin_name: text("origin_name")?,
            name: item.file_name.clone()?,
            size: remote.get("size")?.as_u64()?,
            sha256: text("sha256")?,
        })
    }
}

#[derive(Debug)]
pub enum FetchError {
    /// The device that has the file couldn't be reached.
    Offline(String),
    Refused(RefusalReason),
    /// What arrived isn't the file that was offered.
    Integrity(String),
    Io(io::Error),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Offline(name) => write!(f, "{name} is offline or unreachable"),
            FetchError::Refused(RefusalReason::NotShared) => {
                write!(f, "the file is no longer shared with this device")
            }
            FetchError::Refused(RefusalReason::Missing) => {
                write!(f, "the file was moved or deleted on the other device")
            }
            FetchError::Refused(RefusalReason::Changed) => {
                write!(f, "the file changed on the other device; send it again")
            }
            FetchError::Integrity(reason) => write!(f, "the download was damaged: {reason}"),
            FetchError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for FetchError {
    fn from(e: io::Error) -> Self {
        FetchError::Io(e)
    }
}

/// Fetch `remote` over `stream` (already connected to its owner) as device
/// `me`, into `dest_dir`. Returns where the file ended up.
pub async fn fetch_into<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    me: &str,
    key: &PeerKey,
    remote: &RemoteFile,
    dest_dir: &Path,
    on_progress: impl FnMut(u64),
) -> Result<PathBuf, FetchError> {
    let request = seal_fetch_request(me, key, &remote.offer_id)
        .map_err(|e| FetchError::Io(io::Error::other(e.to_string())))?;
    write_frame(stream, &request).await?;

    let reply = tokio::time::timeout(REPLY_TIMEOUT, read_frame(stream))
        .await
        .map_err(|_| FetchError::Offline(remote.origin_name.clone()))??;
    let (salt, size) = match open_fetch_reply(&reply, key, &remote.offer_id)
        .map_err(|e| FetchError::Integrity(e.to_string()))?
    {
        FetchReply::Start { salt, size } => (salt, size),
        FetchReply::Refused { reason } => return Err(FetchError::Refused(reason)),
    };
    if size != remote.size {
        return Err(FetchError::Integrity(format!(
            "offered {} bytes, sending {size}",
            remote.size
        )));
    }
    let salt = decode_salt(&salt).map_err(|e| FetchError::Integrity(e.to_string()))?;

    let partial = PartialFile::create(dest_dir, &remote.offer_id).await?;
    let mut file = tokio::fs::File::create(&partial.path).await?;
    let context = stream_context(&remote.offer_id, &remote.origin_id);
    let received = receive_stream(
        stream,
        &mut file,
        key,
        &salt,
        &context,
        size,
        FILE_IDLE_TIMEOUT,
        on_progress,
    )
    .await?;
    file.sync_all().await?;
    drop(file);

    if received.byte_len != remote.size {
        return Err(FetchError::Integrity(format!(
            "got {} of {} bytes",
            received.byte_len, remote.size
        )));
    }
    if received.sha256 != remote.sha256 {
        return Err(FetchError::Integrity("its hash doesn't match the offer".into()));
    }

    let placed = place_without_overwrite(&partial.path, dest_dir, &remote.name)?;
    partial.keep();
    Ok(placed)
}

/// Move `from` into `dir` as `name`, or `name (1)`, `name (2)`… if that's
/// taken. Never replaces an existing file, and never lands outside `dir`.
pub fn place_without_overwrite(from: &Path, dir: &Path, name: &str) -> io::Result<PathBuf> {
    let name = clean_file_name(name);
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 => name.split_at(dot),
        _ => (name.as_str(), ""),
    };

    for attempt in 0..MAX_NAME_ATTEMPTS {
        let candidate = if attempt == 0 {
            name.clone()
        } else {
            format!("{stem} ({attempt}){extension}")
        };
        let target = dir.join(&candidate);
        // Belt and braces: the cleaned name has no separators, so this holds.
        if target.parent() != Some(dir) {
            return Err(io::Error::other("refusing a name outside the download folder"));
        }
        // A hard link fails if the name exists, so taking a free name is
        // atomic: nothing can appear there between checking and placing.
        match std::fs::hard_link(from, &target) {
            Ok(()) => {
                std::fs::remove_file(from)?;
                return Ok(target);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            // Filesystems without hard links: check, then rename.
            Err(_) if !target.exists() => {
                std::fs::rename(from, &target)?;
                return Ok(target);
            }
            Err(_) => continue,
        }
    }
    Err(io::Error::other("no free name for the downloaded file"))
}

/// A download in progress. Deleted when dropped unless kept: a failed,
/// cancelled or aborted fetch leaves nothing behind.
struct PartialFile {
    path: PathBuf,
    is_kept: bool,
}

impl PartialFile {
    async fn create(dest_dir: &Path, offer_id: &str) -> io::Result<PartialFile> {
        let dir = dest_dir.join(PARTIAL_DIR);
        tokio::fs::create_dir_all(&dir).await?;
        Ok(PartialFile {
            // The offer id is a checked UUID, so it's a safe file name.
            path: dir.join(format!("{offer_id}.part")),
            is_kept: false,
        })
    }

    fn keep(mut self) {
        self.is_kept = true;
    }
}

impl Drop for PartialFile {
    fn drop(&mut self) {
        if !self.is_kept {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn free_name_is_used_as_is() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("download.part");
        fs::write(&from, b"new").unwrap();

        assert_eq!(
            place_without_overwrite(&from, dir.path(), "report.pdf").unwrap(),
            dir.path().join("report.pdf")
        );
    }

    #[test]
    fn taken_name_gets_a_number() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("report.pdf"), b"mine").unwrap();
        let from = dir.path().join("download.part");
        fs::write(&from, b"new").unwrap();

        assert_eq!(
            place_without_overwrite(&from, dir.path(), "report.pdf").unwrap(),
            dir.path().join("report (1).pdf")
        );
    }

    #[test]
    fn existing_file_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("report.pdf"), b"mine").unwrap();
        let from = dir.path().join("download.part");
        fs::write(&from, b"new").unwrap();
        place_without_overwrite(&from, dir.path(), "report.pdf").unwrap();

        assert_eq!(fs::read(dir.path().join("report.pdf")).unwrap(), b"mine");
    }

    #[test]
    fn several_taken_names_count_up() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.txt"), b"").unwrap();
        fs::write(dir.path().join("a (1).txt"), b"").unwrap();
        let from = dir.path().join("download.part");
        fs::write(&from, b"new").unwrap();

        assert_eq!(
            place_without_overwrite(&from, dir.path(), "a.txt").unwrap(),
            dir.path().join("a (2).txt")
        );
    }

    #[test]
    fn name_that_climbs_out_stays_in_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("download.part");
        fs::write(&from, b"new").unwrap();
        let placed = place_without_overwrite(&from, dir.path(), "../../.bashrc").unwrap();

        assert_eq!(placed.parent(), Some(dir.path()));
    }

    #[test]
    fn placing_moves_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let from = dir.path().join("download.part");
        fs::write(&from, b"new").unwrap();
        place_without_overwrite(&from, dir.path(), "report.pdf").unwrap();

        assert!(!from.exists());
    }

    #[test]
    fn dotfile_without_extension_keeps_its_name() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".bashrc"), b"mine").unwrap();
        let from = dir.path().join("download.part");
        fs::write(&from, b"new").unwrap();

        assert_eq!(
            place_without_overwrite(&from, dir.path(), ".bashrc").unwrap(),
            dir.path().join(".bashrc (1)")
        );
    }

    fn remote_item(metadata: &str) -> ClipboardItem {
        ClipboardItem {
            id: "item".into(),
            content_type: "file".into(),
            content_text: Some("report.pdf".into()),
            content_metadata: metadata.into(),
            source_app: None,
            code_language: None,
            file_url: None,
            file_name: Some("report.pdf".into()),
            file_size_bytes: Some(17),
            file_mime_type: None,
            file_hash: None,
            is_favorite: false,
            is_snippet: false,
            snippet_name: None,
            created_at: String::new(),
            updated_at: String::new(),
            synced: false,
            server_id: None,
        }
    }

    #[test]
    fn remote_item_describes_its_offer() {
        let item = remote_item(
            r#"{"remote":{"offer_id":"o","origin_device_id":"d","origin_name":"Laptop","sha256":"h","size":17}}"#,
        );
        assert_eq!(
            RemoteFile::of(&item),
            Some(RemoteFile {
                offer_id: "o".into(),
                origin_id: "d".into(),
                origin_name: "Laptop".into(),
                name: "report.pdf".into(),
                size: 17,
                sha256: "h".into(),
            })
        );
    }

    #[test]
    fn local_item_has_no_offer() {
        assert_eq!(RemoteFile::of(&remote_item(r#"{"source":"file"}"#)), None);
    }
}
