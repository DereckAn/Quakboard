use crate::clipboard::image_handler::{
    copy_image_file_to_storage, detect_mime_type, ensure_thumbnail, StoredImageInfo,
};
use crate::db::models::CreateClipboardItemDto;
use crate::db::repository::ClipboardRepository;
use crate::AppState;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::fs::{self};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
use std::process::Command;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Runtime, State};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LabFeatureId {
    Screenshot,
    Ocr,
    Translator,
    PickColor,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabFeatureMeta {
    pub id: LabFeatureId,
    pub title: String,
    pub description: String,
    pub icon: String,
    pub needs_download: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<ArtifactMap>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureState {
    pub id: LabFeatureId,
    pub installed_version: Option<String>,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabFeatureWithState {
    pub id: LabFeatureId,
    pub title: String,
    pub description: String,
    pub icon: String,
    pub needs_download: bool,
    pub status: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Artifact {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256_url: Option<String>, // URL para descargar el SHA256 dinámicamente
    pub sha256: String, // Puede estar vacío si se descarga dinámicamente
    pub version: String,
    pub file_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMap {
    pub macos: Option<Artifact>,
    pub windows: Option<Artifact>,
    pub linux: Option<Artifact>,
}

const LAB_STATE_FILE: &str = "lab_features.json";
const FEATURE_ROOT_DIR: &str = "lab_features";
const HELPER_VERSION: &str = "1.0.1";
const HELPER_RELEASE_TAG: &str = "helper-v1.0.1";
const GITHUB_REPO: &str = "DereckAn/quakboard";

/// Determina el sufijo del artifact según OS y arquitectura
fn get_platform_suffix() -> Option<&'static str> {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Some("macos-arm64");
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Some("macos-x64");
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Some("linux-arm64");
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Some("linux-x64");
    }

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Some("windows-arm64");
    }

    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Some("windows-x64");
    }

    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        return None;
    }
}

fn build_screenshot_artifact() -> Option<Artifact> {
    let suffix = get_platform_suffix()?;

    // Agregar .exe solo en windows
    let file_name = if cfg!(target_os = "windows") {
        format!("screenshot-helper-{}.exe", suffix)
    } else {
        format!("screenshot-helper-{}", suffix)
    };

    Some(Artifact {
        url: format!(
            "https://github.com/{}/releases/download/{}/{}",
            GITHUB_REPO, HELPER_RELEASE_TAG, file_name
        ),
        sha256_url: Some(format!(
            "https://github.com/{}/releases/download/{}/SHA256-{}.txt",
            GITHUB_REPO, HELPER_RELEASE_TAG, suffix
        )),
        sha256: String::new(),
        version: HELPER_VERSION.to_string(),
        file_name,
    })
}

fn registry() -> Vec<LabFeatureMeta> {
    let screenshot_artifact = build_screenshot_artifact();

    vec![
        LabFeatureMeta {
            id: LabFeatureId::Screenshot,
            title: "Screenshot capture".to_string(),
            description: "Take full or region screenshots and save to clipboard.".to_string(),
            icon: "image".to_string(),
            needs_download: true,
            artifacts: screenshot_artifact.map(|a| ArtifactMap {
                macos: if cfg!(target_os = "macos") {
                    Some(a.clone())
                } else {
                    None
                },
                windows: if cfg!(target_os = "windows") {
                    Some(a.clone())
                } else {
                    None
                },
                linux: if cfg!(target_os = "linux") {
                    Some(a)
                } else {
                    None
                },
            }),
        },
        LabFeatureMeta {
            id: LabFeatureId::Ocr,
            title: "Image OCR".to_string(),
            description: "Extract text from images (download helper).".to_string(),
            icon: "search".to_string(),
            needs_download: true,
            artifacts: None,
        },
        LabFeatureMeta {
            id: LabFeatureId::Translator,
            title: "Text Translator".to_string(),
            description: "Translate copied text to any language.".to_string(),
            icon: "text".to_string(),
            needs_download: true,
            artifacts: None,
        },
        LabFeatureMeta {
            id: LabFeatureId::PickColor,
            title: "Color Picker".to_string(),
            description: "Pick screen colors and copy HEX/RGB.".to_string(),
            icon: "color".to_string(),
            needs_download: true,
            artifacts: None,
        },
    ]
}

fn state_path(app_state: &AppState) -> PathBuf {
    PathBuf::from(&app_state.app_data_dir).join(LAB_STATE_FILE)
}

fn load_states(app_state: &AppState) -> Vec<FeatureState> {
    let path = state_path(app_state);
    if !path.exists() {
        return Vec::new();
    }

    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_states(app_state: &AppState, states: &[FeatureState]) -> Result<(), String> {
    let path = state_path(app_state);
    let json = serde_json::to_string_pretty(states).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())
}

fn to_feature(meta: &LabFeatureMeta, states: &[FeatureState]) -> LabFeatureWithState {
    let state = states
        .iter()
        .find(|s| s.id == meta.id)
        .cloned()
        .unwrap_or_else(|| FeatureState {
            id: meta.id.clone(),
            installed_version: None,
            enabled: false,
            progress: None,
            error: None,
        });

    LabFeatureWithState {
        id: meta.id.clone(),
        title: meta.title.clone(),
        description: meta.description.clone(),
        icon: meta.icon.clone(),
        needs_download: meta.needs_download,
        status: if state.installed_version.is_some() {
            "installed".to_string()
        } else {
            "available".to_string()
        },
        enabled: state.enabled,
        installed_version: state.installed_version.clone(),
        progress: state.progress,
        error: state.error.clone(),
    }
}

#[tauri::command]
pub fn get_lab_features(state: State<Mutex<AppState>>) -> Result<Vec<LabFeatureWithState>, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let states = load_states(&app_state);
    let features = registry()
        .iter()
        .map(|meta| to_feature(meta, &states))
        .collect();
    Ok(features)
}

#[tauri::command]
pub fn install_feature(
    app: AppHandle,
    state: State<Mutex<AppState>>,
    id: LabFeatureId,
) -> Result<LabFeatureWithState, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let mut states = load_states(&app_state);
    let meta = registry()
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "Feature not found".to_string())?;

    if meta.needs_download {
        let artifacts = meta.artifacts.as_ref().ok_or("No artifacts defined")?;
        let artifact = pick_artifact_for_os(artifacts)?;
        let feature_dir = PathBuf::from(&app_state.app_data_dir)
            .join(FEATURE_ROOT_DIR)
            .join("screenshot")
            .join(&artifact.version);
        download_artifact(&app, artifact, &feature_dir)?;
        update_state_entry(&mut states, &id, Some(artifact.version.clone()), true, None);
    } else {
        update_state_entry(&mut states, &id, Some("builtin".to_string()), true, None);
    }

    save_states(&app_state, &states)?;
    Ok(to_feature(&meta, &states))
}

#[tauri::command]
pub fn uninstall_feature(
    state: State<Mutex<AppState>>,
    id: LabFeatureId,
) -> Result<LabFeatureWithState, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let mut states = load_states(&app_state);
    let meta = registry()
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "Feature not found".to_string())?;

    // Remove installed payload if any
    let feature_dir = PathBuf::from(&app_state.app_data_dir)
        .join(FEATURE_ROOT_DIR)
        .join(match id {
            LabFeatureId::Screenshot => "screenshot",
            LabFeatureId::Ocr => "ocr",
            LabFeatureId::Translator => "translator",
            LabFeatureId::PickColor => "pickColor",
        });
    let _ = fs::remove_dir_all(&feature_dir);

    update_state_entry(&mut states, &id, None, false, None);
    save_states(&app_state, &states)?;
    Ok(to_feature(&meta, &states))
}

#[tauri::command]
pub fn enable_feature(
    state: State<Mutex<AppState>>,
    id: LabFeatureId,
    enabled: bool,
) -> Result<LabFeatureWithState, String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    let mut states = load_states(&app_state);
    let meta = registry()
        .into_iter()
        .find(|m| m.id == id)
        .ok_or_else(|| "Feature not found".to_string())?;

    if let Some(entry) = states.iter_mut().find(|s| s.id == id) {
        entry.enabled = enabled;
    } else {
        states.push(FeatureState {
            id: id.clone(),
            installed_version: None,
            enabled,
            progress: None,
            error: None,
        });
    }

    save_states(&app_state, &states)?;
    Ok(to_feature(&meta, &states))
}

fn update_state_entry(
    states: &mut Vec<FeatureState>,
    id: &LabFeatureId,
    installed_version: Option<String>,
    enabled: bool,
    error: Option<String>,
) {
    if let Some(entry) = states.iter_mut().find(|s| &s.id == id) {
        entry.installed_version = installed_version;
        entry.enabled = enabled;
        entry.error = error;
    } else {
        states.push(FeatureState {
            id: id.clone(),
            installed_version,
            enabled,
            progress: None,
            error,
        });
    }
}

fn pick_artifact_for_os(artifacts: &ArtifactMap) -> Result<&Artifact, String> {
    #[cfg(target_os = "macos")]
    {
        artifacts
            .macos
            .as_ref()
            .ok_or_else(|| "No macOS artifact available".to_string())
    }

    #[cfg(target_os = "windows")]
    {
        artifacts
            .windows
            .as_ref()
            .ok_or_else(|| "No Windows artifact available".to_string())
    }

    #[cfg(target_os = "linux")]
    {
        artifacts
            .linux
            .as_ref()
            .ok_or_else(|| "No Linux artifact available".to_string())
    }
}

/// Internal function that can be called from shortcuts or commands
pub fn capture_screenshot_core<R: Runtime>(
    app: &AppHandle<R>,
    app_state: &AppState,
    mode: CaptureMode,
) -> Result<(), String> {
    let states = load_states(app_state);
    let screenshot_entry = states
        .iter()
        .find(|s| s.id == LabFeatureId::Screenshot)
        .cloned();
    let screenshot_enabled = screenshot_entry
        .as_ref()
        .map(|s| s.enabled && s.installed_version.is_some())
        .unwrap_or(false);

    if !screenshot_enabled {
        return Err(
            "Screenshot helper is not installed or enabled. Please install it from Laboratory."
                .to_string(),
        );
    }

    let installed_version = screenshot_entry
        .and_then(|s| s.installed_version)
        .ok_or_else(|| "Screenshot helper not installed".to_string())?;

    let images_dir = PathBuf::from(&app_state.images_dir);
    fs::create_dir_all(&images_dir).map_err(|e| e.to_string())?;

    let temp_path = PathBuf::from(&app_state.app_data_dir)
        .join(FEATURE_ROOT_DIR)
        .join("screenshot");
    fs::create_dir_all(&temp_path).map_err(|e| e.to_string())?;
    let file_path = temp_path.join(format!("capture-{}.png", Uuid::new_v4()));

    match mode {
        CaptureMode::Full => {
            run_full_capture_with_helper(app_state, &installed_version, &file_path)?
        }
        CaptureMode::Region => {
            run_region_capture_with_helper(app_state, &installed_version, &file_path)?
        }
    }

    ingest_captured_image(app_state, app, &file_path, &images_dir)?;
    Ok(())
}

// Make CaptureMode public so shortcuts.rs can use it
#[derive(Clone, Copy, Debug)]
pub enum CaptureMode {
    Full,
    Region,
}

// Simplify the Tauri commands to use the core function
#[tauri::command]
pub fn capture_full_screenshot<R: Runtime>(
    app: AppHandle<R>,
    state: State<Mutex<AppState>>,
) -> Result<(), String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    capture_screenshot_core(&app, &app_state, CaptureMode::Full)
}

#[tauri::command]
pub fn capture_region_screenshot<R: Runtime>(
    app: AppHandle<R>,
    state: State<Mutex<AppState>>,
) -> Result<(), String> {
    let app_state = state.lock().map_err(|e| e.to_string())?;
    capture_screenshot_core(&app, &app_state, CaptureMode::Region)
}

fn ingest_captured_image<R: Runtime>(
    app_state: &AppState,
    app: &AppHandle<R>,
    file_path: &Path,
    images_dir: &Path,
) -> Result<(), String> {
    // Copy into images_dir to align with existing asset paths
    fs::create_dir_all(images_dir).map_err(|e| e.to_string())?;
    let dest = images_dir.join(
        file_path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("capture.png")),
    );
    fs::copy(file_path, &dest).map_err(|e| format!("Failed to copy capture: {e}"))?;
    let info: StoredImageInfo = copy_image_file_to_storage(&dest, images_dir)?;

    // Ensure thumbnail exists
    let thumb_path = if info.thumb_path.as_os_str().is_empty() {
        ensure_thumbnail(&info.full_path).ok()
    } else {
        Some(info.thumb_path.clone())
    };

    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "width".to_string(),
        serde_json::Value::Number(info.width.into()),
    );
    metadata.insert(
        "height".to_string(),
        serde_json::Value::Number(info.height.into()),
    );
    metadata.insert(
        "file_size".to_string(),
        serde_json::Value::Number(info.file_size.into()),
    );
    metadata.insert(
        "preview_type".to_string(),
        serde_json::Value::String("image".to_string()),
    );
    if let Some(thumb) = thumb_path {
        metadata.insert(
            "thumbnail_path".to_string(),
            serde_json::Value::String(thumb.to_string_lossy().to_string()),
        );
    } else {
        metadata.insert(
            "thumbnail_path".to_string(),
            serde_json::Value::String(info.full_path.to_string_lossy().to_string()),
        );
    }
    metadata.insert(
        "original_extension".to_string(),
        serde_json::Value::String(
            info.original_extension
                .clone()
                .unwrap_or_else(|| "png".to_string()),
        ),
    );
    if let Some(name) = info.original_name.clone() {
        metadata.insert("original_name".to_string(), serde_json::Value::String(name));
    }

    let repo =
        ClipboardRepository::new(&app_state.db_path).map_err(|e| format!("DB error: {e}"))?;
    let dto = CreateClipboardItemDto {
        content_type: "image".to_string(),
        content_text: format!("Image {}x{}", info.width, info.height),
        content_metadata: Some(serde_json::Value::Object(metadata).to_string()),
        source_app: None,
        code_language: None,
    };

    let mut item = repo.create_item(dto).map_err(|e| e.to_string())?;
    let mime_type = detect_mime_type(&info.file_name);

    item.file_url = Some(info.full_path.to_string_lossy().to_string());
    item.file_name = Some(info.file_name.clone());
    item.file_size_bytes = Some(info.file_size as i64);
    item.file_mime_type = Some(mime_type.clone());
    item.file_hash = Some(info.file_hash.clone());

    repo.update_file_info(
        &item.id,
        &info.full_path.to_string_lossy(),
        &info.file_name,
        info.file_size as i64,
        &mime_type,
        Some(&info.file_hash),
    )
    .map_err(|e| e.to_string())?;

    let _ = app.emit("clipboard-item-added", &item);
    Ok(())
}

fn download_artifact<R: Runtime>(
    app: &AppHandle<R>,
    artifact: &Artifact,
    feature_dir: &Path,
) -> Result<PathBuf, String> {
    // println!("=== Download Artifact Debug ===");
    // println!("Artifact URL: {}", artifact.url);
    // println!("Artifact filename: {}", artifact.file_name);
    // println!("Feature directory: {}", feature_dir.display());

    fs::create_dir_all(feature_dir).map_err(|e| e.to_string())?;
    let target_path = feature_dir.join(&artifact.file_name);
    // println!("Target path: {}", target_path.display());

    let client = Client::builder()
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    // Paso 1: Descargar el SHA256 desde GitHub si hay sha256_url
    let expected_sha256 = if let Some(sha256_url) = &artifact.sha256_url {
        let sha_res = client
            .get(sha256_url)
            .send()
            .map_err(|e| format!("Failed to download SHA256: {e}"))?;

        if !sha_res.status().is_success() {
            return Err(format!(
                "Failed to download SHA256: HTTP {}",
                sha_res.status()
            ));
        }

        let sha_text = sha_res
            .text()
            .map_err(|e| format!("Failed to read SHA256: {e}"))?;
        // El formato es: "hash  filename" - tomamos solo el hash
        sha_text
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_lowercase()
    } else if !artifact.sha256.is_empty() {
        artifact.sha256.to_lowercase()
    } else {
        // Sin verificación de integridad (no recomendado)
        String::new()
    };

    // Paso 2: Descargar el binario
    let mut res = client
        .get(&artifact.url)
        .send()
        .map_err(|e| format!("Failed to download artifact: {e}"))?;

    if !res.status().is_success() {
        return Err(format!(
            "Failed to download artifact: HTTP {}",
            res.status()
        ));
    }

    let mut file =
        fs::File::create(&target_path).map_err(|e| format!("Failed to create file: {e}"))?;

    let mut hasher = sha2::Sha256::new();
    let mut downloaded: u64 = 0;
    let total = res
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let mut buffer = [0u8; 8192];
    loop {
        let n = res
            .read(&mut buffer)
            .map_err(|e| format!("Failed to read download: {e}"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buffer[..n])
            .map_err(|e| format!("Failed to write download: {e}"))?;
        hasher.update(&buffer[..n]);
        downloaded += n as u64;
        if total > 0 {
            let percent = ((downloaded as f64 / total as f64) * 100.0).round() as u8;
            let _ = app.emit(
                "lab-feature-download-progress",
                serde_json::json!({
                    "id": "screenshot",
                    "progress": percent,
                }),
            );
        }
    }

    // Paso 3: Verificar integridad
    let computed_hash = format!("{:x}", hasher.finalize());
    if !expected_sha256.is_empty() && computed_hash != expected_sha256 {
        let _ = fs::remove_file(&target_path);
        return Err(format!(
            "Integrity check failed: expected {}, got {}",
            expected_sha256, computed_hash
        ));
    }

    // Paso 4: Hacer ejecutable en Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&target_path)
            .map_err(|e| e.to_string())?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&target_path, perms).map_err(|e| e.to_string())?;
        println!("Set executable permissions: 0o755");
    }

    // Paso 5: Renombrar el file a nombre generico
    let final_name = if cfg!(target_os = "windows") {
        "screenshot-helper.exe"
    } else {
        "screenshot-helper"
    };
    let final_path = feature_dir.join(final_name);

    // Si ya existe un archivo final, eliminarlo antes de renombrar
    if final_path.exists() {
        fs::remove_file(&final_path).map_err(|e| format!("Failed to remove existing file: {e}"))?;
    }

    // Renombrar el arcchivo descargado al nombre final
    fs::rename(&target_path, &final_path).map_err(|e| format!("Failed to rename file: {e}"))?;

    // println!("Renamed to: {}", final_path.display());

    // println!("Download completed successfully!");
    // println!("File exists: {}", final_path.exists());
    if let Ok(metadata) = fs::metadata(&final_path) {
        println!("File size: {} bytes", metadata.len());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            println!("Permissions: {:o}", metadata.permissions().mode());
        }
    }
    // println!("==============================");

    Ok(final_path)
}

fn helper_path(app_state: &AppState, feature: &str, version: &str) -> PathBuf {
    PathBuf::from(&app_state.app_data_dir)
        .join(FEATURE_ROOT_DIR)
        .join(feature)
        .join(version)
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
fn run_full_capture_with_helper(
    app_state: &AppState,
    version: &str,
    path: &Path,
) -> Result<(), String> {
    let helper_dir = helper_path(app_state, "screenshot", version);
    let helper_name = if cfg!(target_os = "windows") {
        "screenshot-helper.exe"
    } else {
        "screenshot-helper"
    };
    let helper = helper_dir.join(helper_name);

    // Debug logging detallado
    // println!("=== Screenshot Helper Debug ===");
    // println!("Helper directory: {}", helper_dir.display());
    // println!("Helper filename: {}", helper_name);
    // println!("Full helper path: {}", helper.display());
    // println!("Helper exists: {}", helper.exists());

    // Listar contenido del directorio para ver que hay realmente
    if let Ok(entries) = std::fs::read_dir(&helper_dir) {
        println!("Contents of helper directory:");
        for entry in entries.flatten() {
            println!("  - {}", entry.path().display());
            if let Ok(metadata) = entry.metadata() {
                println!(" Size: {} bytes", metadata.len());
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    println!("    Permissions: {:o}", metadata.permissions().mode());
                }
            }
        }
    } else {
        print!("Failed to read helper directory contents. Or the directory does not exist.");
    }

    // print!("Running helper command...\n");
    // print!("=========================\n");

    if !helper.exists() {
        return Err(format!(
            "Screenshot helper not found at {}. Reinstall from Laboratory.",
            helper.display()
        ));
    }

    let status = Command::new(&helper)
        .arg("--mode")
        .arg("full")
        .arg("--out")
        .arg(path)
        .status()
        .map_err(|e| format!("Failed to run helper: {e}"))?;

    if !status.success() {
        return Err(format!(
            "Screenshot helper exited with code {:?}",
            status.code()
        ));
    }
    Ok(())
}

// Similar para region:
fn run_region_capture_with_helper(
    app_state: &AppState,
    version: &str,
    path: &Path,
) -> Result<(), String> {
    let helper_dir = helper_path(app_state, "screenshot", version);
    let helper_name = if cfg!(target_os = "windows") {
        "screenshot-helper.exe"
    } else {
        "screenshot-helper"
    };
    let helper = helper_dir.join(helper_name);

    // Debug logging detallado
    // println!("=== Screenshot Helper Debug (Region) ===");
    // println!("Helper directory: {}", helper_dir.display());
    // println!("Helper filename: {}", helper_name);
    // println!("Full helper path: {}", helper.display());
    // println!("Helper exists: {}", helper.exists());

    if let Ok(entries) = std::fs::read_dir(&helper_dir) {
        println!("Directory contents:");
        for entry in entries.flatten() {
            println!("  - {}", entry.path().display());
            if let Ok(metadata) = entry.metadata() {
                println!("    Size: {} bytes", metadata.len());
            }
        }
    }
    // println!("==============================");

    if !helper.exists() {
        return Err(format!(
            "Screenshot helper not found at {}. Reinstall from Laboratory.",
            helper.display()
        ));
    }

    let status = Command::new(&helper)
        .arg("--mode")
        .arg("region")
        .arg("--out")
        .arg(path)
        .status()
        .map_err(|e| format!("Failed to run helper: {e}"))?;

    if !status.success() {
        return Err(format!(
            "Screenshot helper exited with code {:?}",
            status.code()
        ));
    }
    Ok(())
}
