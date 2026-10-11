use crate::clipboard::document_thumbnail;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct StoredFileInfo {
    pub full_path: PathBuf,
    pub original_path: Option<PathBuf>,
    pub file_name: String,
    pub file_size: u64,
    pub file_mime_type: String,
    pub file_hash: String,
    pub original_extension: Option<String>,
    pub original_name: String,
}
pub struct PreparedFile {
    pub hash: String,
    pub mime_type: String,
    pub size: u64,
    pub extension: Option<String>,
    pub file_name: String,
}

pub fn prepare_file_metadata(source_path: &Path) -> Result<PreparedFile, String> {
    if !source_path.exists() {
        return Err(format!(
            "Source file does not exist: {}",
            source_path.display()
        ));
    }

    let extension = source_path
        .extension()
        .and_then(|s| s.to_str())
        .map(|ext| ext.to_lowercase());

    let hash = fingerprint_external_file(source_path)?;
    let size = fs::metadata(source_path).map(|m| m.len()).unwrap_or(0);
    let mime_type = detect_mime_type(extension.as_deref());
    let base_name = source_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();

    Ok(PreparedFile {
        hash,
        mime_type,
        size,
        extension,
        file_name: base_name,
    })
}

pub fn store_prepared_file(
    prepared: &PreparedFile,
    source_path: &Path,
    files_dir: &Path,
) -> Result<StoredFileInfo, String> {
    // Pointer mode (all platforms): we store the ORIGINAL path, never copy.
    let _ = files_dir; // kept for API stability; unused now that we don't copy

    let file_name = source_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(prepared.file_name.as_str())
        .to_string();

    let original_name = file_name.clone();

    Ok(StoredFileInfo {
        full_path: source_path.to_path_buf(),
        original_path: Some(source_path.to_path_buf()),
        file_name,
        file_size: prepared.size,
        file_mime_type: prepared.mime_type.clone(),
        file_hash: prepared.hash.clone(),
        original_extension: prepared.extension.clone(),
        original_name,
    })
}

pub fn delete_file_thumbnail(thumbnail_path: &str) -> Result<(), String> {
    document_thumbnail::delete_thumbnail(Path::new(thumbnail_path))
}

/// Files are always stored as pointers, so deleting a history item must NEVER
/// remove the user's original file — only the generated thumbnail.
pub fn delete_file_assets(_file_path: &str, metadata_json: &str) -> Result<(), String> {
    if let Some(thumb_path) = extract_thumbnail_path(metadata_json) {
        delete_file_thumbnail(&thumb_path)?;
    }
    Ok(())
}

/// Cheap content fingerprint for large external files (videos, big images).
/// Instead of hashing the whole file, we hash its size plus up to 256 KB from
/// the head and tail — O(1) in file size, so copying a multi-GB video no longer
/// reads gigabytes just to build a dedup key.
pub fn fingerprint_external_file(path: &Path) -> Result<String, String> {
    const SAMPLE: u64 = 256 * 1024; // 256 KB

    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open file for fingerprint: {}", e))?;
    let len = file
        .metadata()
        .map_err(|e| format!("Failed to read file metadata: {}", e))?
        .len();

    let mut hasher = Sha256::new();
    hasher.update(len.to_le_bytes());

    // Sample from the start.
    let head_len = SAMPLE.min(len) as usize;
    let mut head = vec![0u8; head_len];
    file.read_exact(&mut head)
        .map_err(|e| format!("Failed to read file head: {}", e))?;
    hasher.update(&head);

    // Sample from the end (only if the file is bigger than one sample).
    if len > SAMPLE {
        file.seek(SeekFrom::End(-(SAMPLE as i64)))
            .map_err(|e| format!("Failed to seek file tail: {}", e))?;
        let mut tail = vec![0u8; SAMPLE as usize];
        file.read_exact(&mut tail)
            .map_err(|e| format!("Failed to read file tail: {}", e))?;
        hasher.update(&tail);
    }

    Ok(format!("{:x}", hasher.finalize()))
}

fn detect_mime_type(extension: Option<&str>) -> String {
    match extension {
        // Text files
        Some("txt") => "text/plain".to_string(),
        Some("log") => "text/plain".to_string(),
        Some("md") => "text/markdown".to_string(),
        Some("csv") => "text/csv".to_string(),
        Some("xml") => "application/xml".to_string(),
        Some("json") => "application/json".to_string(),
        Some("html") | Some("htm") => "text/html".to_string(),
        Some("css") => "text/css".to_string(),
        Some("js") => "application/javascript".to_string(),

        // Documents
        Some("pdf") => "application/pdf".to_string(),
        Some("doc") => "application/msword".to_string(),
        Some("docx") => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document".to_string()
        }
        Some("xls") => "application/vnd.ms-excel".to_string(),
        Some("xlsx") => {
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()
        }
        Some("ppt") => "application/vnd.ms-powerpoint".to_string(),
        Some("pptx") => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation".to_string()
        }
        Some("odt") => "application/vnd.oasis.opendocument.text".to_string(),
        Some("ods") => "application/vnd.oasis.opendocument.spreadsheet".to_string(),
        Some("odp") => "application/vnd.oasis.opendocument.presentation".to_string(),

        // Archives
        Some("zip") => "application/zip".to_string(),
        Some("rar") => "application/x-rar-compressed".to_string(),
        Some("7z") => "application/x-7z-compressed".to_string(),
        Some("tar") => "application/x-tar".to_string(),
        Some("gz") => "application/gzip".to_string(),

        // Code files
        Some("rs") => "text/x-rust".to_string(),
        Some("py") => "text/x-python".to_string(),
        Some("java") => "text/x-java".to_string(),
        Some("cpp") | Some("cc") | Some("cxx") => "text/x-c++".to_string(),
        Some("c") => "text/x-c".to_string(),
        Some("h") | Some("hpp") => "text/x-c-header".to_string(),
        Some("go") => "text/x-go".to_string(),
        Some("rb") => "text/x-ruby".to_string(),
        Some("php") => "application/x-httpd-php".to_string(),
        Some("swift") => "text/x-swift".to_string(),
        Some("kt") => "text/x-kotlin".to_string(),
        Some("ts") => "application/typescript".to_string(),
        Some("sh") => "application/x-sh".to_string(),
        Some("bat") => "application/x-bat".to_string(),

        // Audio
        Some("mp3") => "audio/mpeg".to_string(),
        Some("wav") => "audio/wav".to_string(),
        Some("ogg") => "audio/ogg".to_string(),
        Some("m4a") => "audio/mp4".to_string(),
        Some("flac") => "audio/flac".to_string(),

        // Video
        Some("mp4") => "video/mp4".to_string(),
        Some("avi") => "video/x-msvideo".to_string(),
        Some("mov") => "video/quicktime".to_string(),
        Some("wmv") => "video/x-ms-wmv".to_string(),
        Some("mkv") => "video/x-matroska".to_string(),
        Some("webm") => "video/webm".to_string(),

        // Fonts
        Some("ttf") => "font/ttf".to_string(),
        Some("otf") => "font/otf".to_string(),
        Some("woff") => "font/woff".to_string(),
        Some("woff2") => "font/woff2".to_string(),

        // Other common files
        Some("apk") => "application/vnd.android.package-archive".to_string(),
        Some("dmg") => "application/x-apple-diskimage".to_string(),
        Some("exe") => "application/x-msdownload".to_string(),
        Some("deb") => "application/x-debian-package".to_string(),
        Some("rpm") => "application/x-rpm".to_string(),

        // Fallback for any other extension (excluding images)
        Some(ext) => mime_guess::from_ext(ext)
            .first_or_octet_stream()
            .essence_str()
            .to_string(),
        None => "application/octet-stream".to_string(),
    }
}

fn extract_thumbnail_path(metadata_json: &str) -> Option<String> {
    if metadata_json.trim().is_empty() {
        return None;
    }

    serde_json::from_str::<Value>(metadata_json)
        .ok()
        .and_then(|value| {
            value
                .get("thumbnail_path")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
        })
}

#[cfg(target_os = "macos")]
pub fn write_file_to_clipboard(file_path: &str) -> Result<(), String> {
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{NSPasteboard, NSPasteboardWriting};
    use objc2_foundation::{NSArray, NSString, NSURL};

    let pasteboard: Option<Retained<NSPasteboard>> = unsafe {
        use objc2::{msg_send, ClassType};
        msg_send![NSPasteboard::class(), generalPasteboard]
    };
    let Some(pasteboard) = pasteboard else {
        return Err("Failed to access pasteboard".into());
    };

    // Remove unnecessary unsafe - clearContents is safe to call
    pasteboard.clearContents();

    let new_path = NSString::from_str(file_path);
    // Remove unnecessary unsafe - fileURLWithPath is safe to call
    let url = NSURL::fileURLWithPath(&new_path);

    // Cast NSURL to NSPasteboardWriting protocol object
    let protocol_obj: Retained<ProtocolObject<dyn NSPasteboardWriting>> =
        ProtocolObject::from_retained(url);
    let objects: Retained<NSArray<ProtocolObject<dyn NSPasteboardWriting>>> =
        NSArray::from_retained_slice(&[protocol_obj]);

    // Remove unnecessary unsafe - writeObjects is safe to call
    let success = pasteboard.writeObjects(&objects);
    if success {
        Ok(())
    } else {
        Err("Failed to write file to clipboard".into())
    }
}
