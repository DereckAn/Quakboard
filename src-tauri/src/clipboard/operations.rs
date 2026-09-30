use arboard::{Clipboard, ImageData};
use std::io::Read;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;

/// Maximum image size we'll pull off the clipboard (50 MB).
/// Bigger than any real screenshot, small enough to never OOM the app.
use crate::clipboard::{detect_backend, state, ClipboardBackend};

lazy_static::lazy_static! {
    static ref CLIPBOARD: Mutex<Clipboard> =Mutex::new(Clipboard::new().unwrap());
}
const MAX_CLIPBOARD_IMAGE_BYTES: u64 = 50 * 1024 * 1024;

// Enum para representar el contenido del clipboard
#[derive(Debug, Clone)]
pub enum ClipboardContent {
    Text(String),
    Image(ImageData<'static>, bool),
    ImageFile(PathBuf), // File path to image from Finder
    File(PathBuf),      // File path to any file from Finder
    Empty,
}

/// public entry point: Route to the conrrect backend at rentime.
pub fn read_clipboard_content() -> Result<ClipboardContent, String> {
    match detect_backend() {
        ClipboardBackend::Wayland => read_clipboard_content_wayland(),
        ClipboardBackend::Native => read_clipboard_content_native(),
    }
}

/// Read the clipboard on Wayland using the `wl-paste` command-line tool.
/// Dispatches by available type: file list (pointer) → image (bytes) → text.
fn read_clipboard_content_wayland() -> Result<ClipboardContent, String> {
    let types = wayland_list_types();

    // 1. File list first, so copied files/images stay POINTERS (not byte copies).
    if types.iter().any(|t| t == "text/uri-list") {
        if let Some(content) = read_files_wayland() {
            return Ok(content);
        }
    }

    // 2. Raw image data (e.g. a screenshot) — there's no file to point to, so save bytes.
    if types.iter().any(|t| t == "image/png") {
        if let Some(image) = read_image_wayland() {
            return Ok(ClipboardContent::Image(image, looks_like_screenshot_types(&types)));
        }
    }

    // 3. Plain text.
    if let Some(text) = read_text_wayland()? {
        return Ok(ClipboardContent::Text(text));
    }

    Ok(ClipboardContent::Empty)
}

/// Run 'wl-paste' and return the clipboard text, or 'None' if empty.
fn read_text_wayland() -> Result<Option<String>, String> {
    let output = Command::new("wl-paste")
        .arg("--no-newline")
        .output()
        .map_err(|e| format!("Failed to execute wl-paste: {}", e))?;

    // wl-paste exist non-zero when the clipboard is empty or holds no text.
    // That's normal, not an error - report "no text".
    if !output.status.success() {
        return Ok(None);
    }

    let text = String::from_utf8_lossy(&output.stdout).to_string();
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

// Read from OS clipboard (Text or Image)
fn read_clipboard_content_native() -> Result<ClipboardContent, String> {
    let mut clipboard = CLIPBOARD.lock().map_err(|e| e.to_string())?;

    // First check for file list (Finder copy)
    if let Ok(paths_vec) = clipboard.get().file_list() {
        for path in &paths_vec {
            if is_image_file_path(path) {
                return Ok(ClipboardContent::ImageFile(path.clone()));
            }
        }
        for path in &paths_vec {
            if path.is_file() && !is_image_file_path(path) {
                println!("📁 Detected file from clipboard: {}", path.display());
                return Ok(ClipboardContent::File(path.clone()));
            }
        }
    }

    let mut cached_text: Option<String> = None;
    let mut screenshot_hint = false;

    if let Ok(text) = clipboard.get_text() {
        if !text.is_empty() {
            let trimmed = text.trim();
            let potential_path = PathBuf::from(trimmed);

            if trimmed.starts_with("file://") {
                if let Some(resolved) = path_from_file_url(trimmed) {
                    if resolved.is_file() && !is_image_file_path(&resolved) {
                        println!("📁 Detected file URL: {}", resolved.display());
                        return Ok(ClipboardContent::File(resolved));
                    }
                }
            }
            if is_image_file_path(&potential_path) {
                println!(
                    "📁 Detected image file from text: {}",
                    potential_path.display()
                );
                return Ok(ClipboardContent::ImageFile(potential_path));
            }
            if is_image_file_url(trimmed) {
                if let Some(resolved) = path_from_file_url(trimmed) {
                    if is_image_file_path(&resolved) {
                        println!("📁 Detected image file URL: {}", resolved.display());
                        return Ok(ClipboardContent::ImageFile(resolved));
                    }
                }
            }
        }

        screenshot_hint = looks_like_screenshot_text(&text);
        if !text.is_empty() {
            cached_text = Some(text);
        }
    }

    if let Ok(image) = clipboard.get_image() {
        //convert ImageData to 'static lifetime
        let owned_image = ImageData {
            width: image.width,
            height: image.height,
            bytes: image.bytes.into_owned().into(),
        };

        return Ok(ClipboardContent::Image(owned_image, screenshot_hint));
    }

    if let Some(text) = cached_text {
        return Ok(ClipboardContent::Text(text));
    }

    // If no cached text, fallback to plain get_text again
    if let Ok(text) = clipboard.get_text() {
        if !text.is_empty() {
            return Ok(ClipboardContent::Text(text));
        }
    }

    Ok(ClipboardContent::Empty)
}

// Helper to detect if text is an image file path
fn is_image_file_path<P: AsRef<Path>>(path: P) -> bool {
    let path = path.as_ref();
    if !path.exists() {
        return false;
    }
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
    {
        Some(ext) => matches!(
            ext.as_str(),
            "png"
                | "jpg"
                | "jpeg"
                | "gif"
                | "webp"
                | "bmp"
                | "tiff"
                | "svg"
                | "ico"
                | "heic"
                | "heif"
        ),
        None => false,
    }
}

/// Read text from OS clipboard (legacy function)
pub fn read_clipboard() -> Result<String, String> {
    let mut clipboard = CLIPBOARD.lock().map_err(|e| e.to_string())?;
    clipboard.get_text().map_err(|e| e.to_string())
}

/// Write text to OS clipboard
fn write_text_native(text: &str) -> Result<(), String> {
    let mut clipboard = CLIPBOARD.lock().map_err(|e| e.to_string())?;
    clipboard.set_text(text).map_err(|e| e.to_string())
}

/// Write text to the Wayland clipboard via 'wl-copy'.
/// Reads the text from stdin
fn write_text_wayland(text: &str) -> Result<(), String> {
    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run wl-copy: {e}"))?;

    // Write the text into wl-copy's stdin, then close it so wl-copy sees EOF.
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or("wl-copy stdin was not available")?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("Failed to write to wl-copy stdin: {e}"))?;
    } // <- stdin is dropped here, sending EOF to wl-copy

    // wl-copy reads stdin, then forks a tiny daemon to serve the selection and
    // the parent exits. Waiting reaps the parent and confirms it succeeded.
    let status = child
        .wait()
        .map_err(|e| format!("Failed to wait for wl-copy: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy exited with status: {status}"))
    }
}

/// Public entry point: route the write to the correct backend at runtime.
pub fn write_clipboard(text: &str) -> Result<(), String> {
    match detect_backend() {
        ClipboardBackend::Wayland => write_text_wayland(text),
        ClipboardBackend::Native => write_text_native(text),
    }
}

// Write image to OS clipboard
pub fn write_clipboard_image(image_path: &str) -> Result<(), String> {
    use image::GenericImageView;

    // Load image from file
    let img = image::open(image_path).map_err(|e| format!("Failed to open image: {}", e))?;

    // COnvert to RGBA
    let rgba = img.to_rgba8();
    let (width, height) = img.dimensions();

    //Create ImageData
    let image_data = ImageData {
        width: width as usize,
        height: height as usize,
        bytes: rgba.into_raw().into(),
    };

    // Write to clipboard
    let mut clipboard = CLIPBOARD.lock().map_err(|e| e.to_string())?;
    clipboard
        .set_image(image_data)
        .map_err(|e| e.to_string())
        // Wayland reports one write as two clipboard events; elsewhere it's one.
        .map(|_| state::request_skip_events(image_write_events()))
}

fn image_write_events() -> usize {
    match detect_backend() {
        ClipboardBackend::Wayland => 2,
        ClipboardBackend::Native => 1,
    }
}

fn is_image_file_url(text: &str) -> bool {
    text.trim_start().starts_with("file://")
}

fn path_from_file_url(url: &str) -> Option<PathBuf> {
    let decoded = percent_encoding::percent_decode_str(url.trim_start_matches("file://"))
        .decode_utf8()
        .ok()?;
    Some(PathBuf::from(decoded.as_ref()))
}

/// Screenshot tools (hyprshot, grim | wl-copy) offer only image data. An app
/// copying an image (a browser's "Copy image") also offers its HTML or source
/// URL, so any of those means "not a screenshot".
fn looks_like_screenshot_types(types: &[String]) -> bool {
    !types.iter().any(|t| {
        t == "text/html" || t == "text/x-moz-url" || t == "chromium/x-source-url"
    })
}

fn looks_like_screenshot_text(text: &str) -> bool {
    let lowered = text.trim().to_lowercase();
    if lowered.is_empty() {
        return false;
    }

    let screenshot_markers = [
        "screenshot",
        "screen shot",
        "captura de pantalla",
        "スクリーンショット",
        "截圖",
    ];

    if screenshot_markers
        .iter()
        .any(|marker| lowered.contains(marker))
    {
        return true;
    }

    lowered.ends_with(".png") || lowered.ends_with(".tiff") || lowered.ends_with(".heic")
}

/// List the MIME types currently available on the Wayland clipboard.
/// Runs 'wl-paste'  and return one type per line
/// Returns an empty Vex if wl past fails of the clipboard is empty1
fn wayland_list_types() -> Vec<String> {
    let output = match Command::new("wl-paste").arg("--list-types").output() {
        Ok(out) => out,
        Err(_) => return Vec::new(),
    };

    if !output.status.success() {
        return Vec::new();
    }

    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// Read an image from the Wayland clipboard as raw RGBA pixels.
/// Streams at most MAX_CLIPBOARD_IMAGE_BYTES so a giant image can't OOM us.
fn read_image_wayland() -> Option<ImageData<'static>> {
    use image::GenericImageView;

    let mut child = Command::new("wl-paste")
        .arg("--type")
        .arg("image/png")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let mut buf = Vec::new();

    // Read one byte past the cap so we can tell "exactly at cap" from "too big".
    stdout
        .by_ref()
        .take(MAX_CLIPBOARD_IMAGE_BYTES + 1)
        .read_to_end(&mut buf)
        .ok()?;

    drop(stdout); // close the pipe; wl-paste stops writing
    let _ = child.wait();

    if buf.is_empty() || buf.len() as u64 > MAX_CLIPBOARD_IMAGE_BYTES {
        return None; // empty, or too large to handle safely
    }

    let img = image::load_from_memory(&buf).ok()?;
    let rgba = img.to_rgba8();
    let (width, height) = img.dimensions();

    Some(ImageData {
        width: width as usize,
        height: height as usize,
        bytes: rgba.into_raw().into(),
    })
}

/// Read a file list from the wayland clipboard via wl-paste --type text/uri--list
/// Returns the first image file as imagefile otherwise the first regular file
/// as file . returns none if the clipboard holds no usable file paths
fn read_files_wayland() -> Option<ClipboardContent> {
    let output = Command::new("wl-paste")
        .arg("--type")
        .arg("text/uri-list")
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);

    // A uri-list is one URI per line (CRLF); lines strating with # are comments
    let paths: Vec<PathBuf> = text
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty() && !line.starts_with("#"))
        .filter_map(path_from_file_url)
        .collect();

    // Prefer an image file (mirrors the native backend's behavior)
    for path in &paths {
        if is_image_file_path(path) {
            return Some(ClipboardContent::ImageFile(path.clone()));
        }
    }

    // Otherwise the first regular file (skip directories and mission paths).
    for path in &paths {
        if path.is_file() && !is_image_file_path(path) {
            return Some(ClipboardContent::File(path.clone()));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn types(list: &[&str]) -> Vec<String> {
        list.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn image_only_copy_looks_like_a_screenshot() {
        // What hyprshot's `wl-copy --type image/png` offers.
        assert!(looks_like_screenshot_types(&types(&["image/png"])));
    }

    #[test]
    fn browser_image_copy_is_not_a_screenshot() {
        // What Brave's "Copy image" offers.
        assert!(!looks_like_screenshot_types(&types(&[
            "text/x-moz-url",
            "text/html",
            "image/png",
            "chromium/x-internal-source-rfh-token",
            "chromium/x-source-url",
        ])));
    }

    #[test]
    fn image_with_html_is_not_a_screenshot() {
        assert!(!looks_like_screenshot_types(&types(&["image/png", "text/html"])));
    }
}
