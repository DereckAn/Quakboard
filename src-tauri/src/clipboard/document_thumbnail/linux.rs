use cairo::{Context, ImageSurface};
use md5;
use pango::FontDescription;
use pangocairo::functions::{create_layout, show_layout};
use std::env;
use std::fs;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::Command;
use uuid::Uuid;

pub fn generate_thumbnail(path: &Path, target_path: &Path) -> Result<bool, String> {
    if !path.exists() {
        return Ok(false);
    }

    let Some(target_dir) = target_path.parent() else {
        return Err("Invalid thumbnail target path".to_string());
    };

    if target_path.exists() {
        return Ok(true);
    }

    if let Some(existing) = lookup_freedesktop_cache(path) {
        if fs::copy(&existing, target_path).is_ok() {
            return Ok(true);
        }
    }

    if generate_with_gdk_pixbuf(path, target_path)? {
        return Ok(true);
    }

    if is_pdf(path) {
        if generate_pdf_thumbnail(path, target_dir, target_path)? {
            return Ok(true);
        }
    } else if is_text_like(path) {
        if generate_text_thumbnail(path, target_path)? {
            return Ok(true);
        }
    } else if is_video(path) && generate_video_thumbnail(path, target_path)? {
        return Ok(true);
    }

    Ok(false)
}

fn lookup_freedesktop_cache(path: &Path) -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let cache_root = Path::new(&home).join(".cache/thumbnails/normal");
    let hash = md5_path(path);
    let candidate = cache_root.join(format!("{hash}.png"));
    candidate.exists().then_some(candidate)
}

fn md5_path(path: &Path) -> String {
    let digest = md5::compute(path.to_string_lossy().as_bytes());
    format!("{:x}", digest)
}

fn is_pdf(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("pdf"))
        .unwrap_or(false)
}

fn is_video(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("mp4")
            | Some("mkv")
            | Some("mov")
            | Some("avi")
            | Some("webm")
            | Some("wmv")
            | Some("m4v")
            | Some("flv")
            | Some("mpeg")
            | Some("mpg")
    )
}

fn is_text_like(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|s| s.to_ascii_lowercase()),
        Some(ref ext)
            if matches!(
                ext.as_str(),
                "txt"
                    | "md"
                    | "markdown"
                    | "log"
                    | "json"
                    | "rs"
                    | "ts"
                    | "tsx"
                    | "js"
                    | "jsx"
                    | "css"
                    | "html"
                    | "java"
                    | "py"
                    | "go"
                    | "php"
            )
    )
}

fn generate_pdf_thumbnail(
    path: &Path,
    target_dir: &Path,
    target_path: &Path,
) -> Result<bool, String> {
    let temp_prefix = target_dir.join(format!(".pdftmp-{}", Uuid::new_v4()));
    let status = Command::new("pdftoppm")
        .arg("-png")
        .arg("-singlefile")
        .arg(path)
        .arg(&temp_prefix)
        .status();

    let png_path = temp_prefix.with_extension("png");
    let result = match status {
        Ok(s) if s.success() && png_path.exists() => {
            fs::rename(&png_path, target_path)
                .map_err(|e| format!("Failed to move PDF thumbnail: {e}"))?;
            Ok(true)
        }
        _ => Ok(false),
    };

    let _ = fs::remove_file(&png_path);
    let _ = fs::remove_file(&temp_prefix);
    result
}

fn generate_video_thumbnail(path: &Path, target_path: &Path) -> Result<bool, String> {
    // ffmpegthumbnailer extracts a representative frame (same tool Dolphin/Thunar use).
    // -s 256 = longest side 256px. Returns Ok(false) if it can't decode — graceful fallback.
    let status = Command::new("ffmpegthumbnailer")
        .arg("-i")
        .arg(path)
        .arg("-o")
        .arg(target_path)
        .arg("-s")
        .arg("256")
        .status();

    match status {
        Ok(s) if s.success() && target_path.exists() => Ok(true),
        _ => Ok(false),
    }
}

fn generate_text_thumbnail(path: &Path, target_path: &Path) -> Result<bool, String> {
    let content = fs::read_to_string(path).unwrap_or_default();
    let preview: String = content.lines().take(20).collect::<Vec<_>>().join("\n");

    let surface = ImageSurface::create(cairo::Format::ARgb32, 512, 512)
        .map_err(|e| format!("Failed to create surface: {e}"))?;
    let context = Context::new(&surface).map_err(|e| format!("Failed to create context: {e}"))?;

    context.set_source_rgb(0.11, 0.12, 0.15);
    context.rectangle(0.0, 0.0, 512.0, 512.0);
    context
        .fill()
        .map_err(|e| format!("Failed to fill surface: {e}"))?;

    let layout = create_layout(&context);
    let font_desc = FontDescription::from_string("JetBrains Mono 12");
    layout.set_font_description(Some(&font_desc));
    layout.set_width(480 * pango::SCALE);
    layout.set_text(&preview);

    context.set_source_rgb(0.85, 0.87, 0.92);
    context.move_to(16.0, 16.0);
    show_layout(&context, &layout);

    let mut file =
        File::create(target_path).map_err(|e| format!("Failed to create target file: {e}"))?;
    surface
        .write_to_png(&mut file)
        .map_err(|e| format!("Failed to save text thumbnail: {e}"))?;

    Ok(true)
}

fn generate_with_gdk_pixbuf(path: &Path, target_path: &Path) -> Result<bool, String> {
    let status = Command::new("gdk-pixbuf-thumbnailer")
        .arg("-s")
        .arg("512")
        .arg(path)
        .arg("--output")
        .arg(target_path)
        .status();

    match status {
        Ok(code) if code.success() && target_path.exists() => Ok(true),
        Ok(_) => Ok(false),
        Err(e) => {
            if cfg!(debug_assertions) {
                eprintln!("Failed to run gdk-pixbuf-thumbnailer: {e}");
            }
            Ok(false)
        }
    }
}
