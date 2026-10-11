use std::env::var_os;
use std::fs::{read_to_string, write};
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::clipboard::state::store_previous_app_pid;
use crate::commands::lab::{capture_screenshot_core, CaptureMode};
use crate::AppState;

/// Register a shortcut with a handler for toggling window visibility
/// This is used for dynamically changing the shortcut at runtime
pub fn register_shortcut(app: &AppHandle, shortcut_str: &str) -> Result<(), String> {
    let shortcut = shortcut_str
        .parse::<Shortcut>()
        .map_err(|e| format!("Invalid shortcut format: {e}"))?;

    let shortcut_display = shortcut_str.to_string();
    let app_clone = app.clone();

    // Register the shortcut handler FIRST
    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                println!("Dynamic hotkey pressed: {shortcut_display}");

                // Use the cloned app handle to get the window
                if let Some(window) = app_clone.get_webview_window("main") {
                    match window.is_visible() {
                        Ok(true) => {
                            println!("Hiding window...");
                            let _ = window.hide();
                        }
                        Ok(false) => {
                            println!("Showing window...");

                            #[cfg(target_os = "macos")]
                            {
                                use objc2_app_kit::NSWorkspace;

                                let workspace = NSWorkspace::sharedWorkspace();
                                if let Some(app) = workspace.frontmostApplication() {
                                    let pid = app.processIdentifier();
                                    store_previous_app_pid(pid);
                                    println!("Stored previous app PID: {pid}");
                                }
                            }
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                        Err(e) => {
                            eprintln!("Failed to check window visibility: {e}");
                        }
                    }
                } else {
                    eprintln!("Could not find main window!");
                }
            }
        })
        .map_err(|e| e.to_string())?;

    // THEN register the shortcut
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| e.to_string())?;

    println!("Registered dynamic shortcut: {shortcut_str}");
    Ok(())
}

/// Which mechanism registers the global toggle hotkey for this session.
///
/// `tauri-plugin-global-shortcut` relies on X11 grabs and does not work under
/// native Wayland, so on Hyprland we drive a compositor bind via `hyprctl`
/// instead (#7).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HotkeyBackend {
    GlobalShortcut,
    // Only detected on Linux, but matched in shared code on every platform.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    Hyprland,
}

/// Pick the hotkey mechanism for the current environment.
pub fn detect_backend() -> HotkeyBackend {
    #[cfg(target_os = "linux")]
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        return HotkeyBackend::Hyprland;
    }
    HotkeyBackend::GlobalShortcut
}

/// Initialize the global-shortcut plugin. Always needed (screenshot hotkeys and
/// the shortcut commands depend on it), even when the toggle uses Hyprland.
pub fn init_global_shortcut_plugin(app: &AppHandle) -> Result<(), String> {
    app.plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .map_err(|e| e.to_string())
}

/// Translate a Tauri accelerator ("CommandOrControl+Shift+V") into a Hyprland
/// `(mods, key)` pair ("CTRL SHIFT", "V").
fn to_hyprland_bind(hotkey: &str) -> Result<(String, String), String> {
    let mut mods: Vec<&str> = Vec::new();
    let mut key: Option<&str> = None;

    for part in hotkey.split('+') {
        let p = part.trim();
        match p.to_ascii_lowercase().as_str() {
            "commandorcontrol" | "cmdorctrl" | "control" | "ctrl" => mods.push("CTRL"),
            "alt" | "option" => mods.push("ALT"),
            "shift" => mods.push("SHIFT"),
            "super" | "meta" | "command" | "cmd" | "win" | "windows" => mods.push("SUPER"),
            "" => {}
            _ => key = Some(p),
        }
    }

    let key = key.ok_or_else(|| format!("No key found in hotkey '{hotkey}'"))?;
    Ok((mods.join(" "), key.to_string()))
}

/// How Hyprland should launch us when the bind fires. Prefer the real AppImage
/// path ($APPIMAGE) over the ephemeral mount point exposed by current_exe().
fn launch_command() -> String {
    let exe = std::env::var("APPIMAGE")
        .ok()
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "quakboard".to_string());
    format!("{exe} --toggle")
}

fn hyprctl(args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new("hyprctl")
        .args(args)
        .output()
        .map_err(|e| format!("hyprctl unavailable: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "hyprctl {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

// Hyprland's config dir: $XDG_CONFIG_HOME/hypr or $HOME/.config/hypr
fn hypr_config_dir() -> Option<PathBuf> {
    let base = var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
    Some(base.join("hypr"))
}

/// Write the managed config (bind + widnows rules)/ The live "hyprctl" bind only
/// lasts the session; this file is sourced on every Hyprland start, so the
/// hotkey and windows behavior survive a roboot. Regenerated on every bind so it
/// always matches the curren hotkey. Window class is 'quakboard' ( verified via
/// 'hyprctl clients')
fn write_managed_config(hotkey: &str) -> Result<(), String> {
    let dir = hypr_config_dir().ok_or("Could not resolve hyprland config directory")?;
    let (mods, key) = to_hyprland_bind(hotkey)?;
    let contents = format!(
        "# Managed by Quakboard — do not edit by hand.\n\
         # Regenerated whenever you change the hotkey in Settings.\n\
         \n\
         bind = {mods}, {key}, exec, {launch}\n\
         \n\
         windowrule {{\n\
         name = quakboard\n\
         match:class = (quakboard)\n\
         float = true\n\
         center = true\n\
         size = 750 480\n\
         rounding = 20\n\
         }}\n",
        launch = launch_command(),
    );

    write(dir.join("quakboard.conf"), contents).map_err(|e| e.to_string())
}

///Make sure custom.conf sourcs our managed file (append-once, idempotent).
/// Custom.conf is sourced last by hyperland.congf, sol our rules win.
fn ensure_custom_sources_managed() -> Result<(), String> {
    let dir = hypr_config_dir().ok_or("Could not resolve Hyprland config dir")?;
    let custom = dir.join("conf").join("custom.conf");
    let existing = read_to_string(&custom).unwrap_or_default();
    if existing.contains("quakboard.conf") {
        return Ok(());
    }
    let mut updated = existing;
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    updated.push_str(
        "\n# Quakboard global hotkey + window rules\nsource = ~/.config/hypr/quakboard.conf\n",
    );
    write(&custom, updated).map_err(|e| e.to_string())?;
    Ok(())
}

/// Bind `hotkey` to toggle the window via the compositor. Idempotent: drops any
/// identical pre-existing bind first so app relaunches don't stack duplicates.
pub fn hyprland_bind(hotkey: &str) -> Result<(), String> {
    let (mods, key) = to_hyprland_bind(hotkey)?;
    let _ = hyprctl(&["keyword", "unbind", &format!("{mods}, {key}")]);
    let value = format!("{mods}, {key}, exec, {}", launch_command());
    hyprctl(&["keyword", "bind", &value])?;

    // Persist so the bind + window rules survive a reboot / compositor reload.
    write_managed_config(hotkey)?;
    ensure_custom_sources_managed()?;
    Ok(())
}

/// Remove a previously applied Hyprland bind for `hotkey`.
pub fn hyprland_unbind(hotkey: &str) -> Result<(), String> {
    let (mods, key) = to_hyprland_bind(hotkey)?;
    hyprctl(&["keyword", "unbind", &format!("{mods}, {key}")])
}

/// Register a shortcut upon application start
/// This is used during the initial setup in lib.rs
pub fn register_shortcut_upon_start(app: &AppHandle, shortcut_str: &str) -> Result<(), String> {
    init_global_shortcut_plugin(app)?;
    println!("Global shortcut plugin initialized");

    match detect_backend() {
        HotkeyBackend::Hyprland => {
            println!("Hyprland detected — binding toggle via hyprctl");
            hyprland_bind(shortcut_str)
        }
        HotkeyBackend::GlobalShortcut => register_shortcut(app, shortcut_str),
    }
}

/// Unregister all shortcuts
pub fn unregister_all_shortcuts(app: &AppHandle) -> Result<(), String> {
    app.global_shortcut()
        .unregister_all()
        .map_err(|e| e.to_string())?;

    println!("Unregistered all shortcuts");
    Ok(())
}

/// Register screenshot shortcuts
pub fn register_screenshot_shortcuts(
    app: &AppHandle,
    full_hotkey: &str,
    region_hotkey: &str,
) -> Result<(), String> {
    let manager = app.global_shortcut();

    println!("=== Registering Screenshot Hotkeys ===");
    println!("Full hotkey: '{}'", full_hotkey);
    println!("Region hotkey: '{}'", region_hotkey);

    let mut errors: Vec<String> = Vec::new();

    // Parse and register full screenshot hotkey
    if !full_hotkey.is_empty() {
        if let Err(e) = register_single_screenshot_shortcut(
            app,
            manager,
            full_hotkey,
            CaptureMode::Full,
            "full",
        ) {
            println!("WARNING: Failed to register full screenshot hotkey: {}", e);
            errors.push(format!("Full: {}", e));
        }
    }

    // Parse and register region screenshot hotkey
    if !region_hotkey.is_empty() {
        if let Err(e) = register_single_screenshot_shortcut(
            app,
            manager,
            region_hotkey,
            CaptureMode::Region,
            "region",
        ) {
            println!(
                "WARNING: Failed to register region screenshot hotkey: {}",
                e
            );
            errors.push(format!("Region: {}", e));
        }
    }

    println!("=== Screenshot Hotkeys Registration Complete ===");

    // Only fail if both failed
    if errors.len() == 2 {
        return Err(errors.join("; "));
    }

    Ok(())
}

fn register_single_screenshot_shortcut(
    app: &AppHandle,
    manager: &tauri_plugin_global_shortcut::GlobalShortcut<tauri::Wry>,
    hotkey: &str,
    mode: CaptureMode,
    label: &str,
) -> Result<(), String> {
    let shortcut: Shortcut = hotkey
        .parse()
        .map_err(|e| format!("Invalid {} screenshot hotkey '{}': {}", label, hotkey, e))?;

    println!("Parsed {} shortcut: {:?}", label, shortcut);

    // Unregister if already registered
    if manager.is_registered(shortcut.clone()) {
        println!(
            "{} shortcut already registered, unregistering first...",
            label
        );
        manager
            .unregister(shortcut.clone())
            .map_err(|e| format!("Failed to unregister {} shortcut: {}", label, e))?;
    }

    // Set up the handler
    let app_handle = app.clone();
    let mode_clone = mode;
    let label_owned = label.to_string();

    manager
        .on_shortcut(shortcut.clone(), move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                println!(">>> {} screenshot hotkey PRESSED!", label_owned);

                if let Some(state) = app_handle.try_state::<Mutex<AppState>>() {
                    if let Ok(app_state) = state.lock() {
                        if let Err(e) = capture_screenshot_core(&app_handle, &app_state, mode_clone)
                        {
                            eprintln!(">>> Failed to capture {} screenshot: {}", label_owned, e);
                        } else {
                            println!(">>> {} screenshot captured successfully!", label_owned);
                        }
                    } else {
                        eprintln!(">>> Failed to lock AppState");
                    }
                } else {
                    eprintln!(">>> Could not access AppState");
                }
            }
        })
        .map_err(|e| format!("Failed to set handler for {} shortcut: {}", label, e))?;

    // Register the shortcut
    manager
        .register(shortcut)
        .map_err(|e| format!("Failed to register {} shortcut '{}': {}", label, hotkey, e))?;

    println!("✓ {} screenshot hotkey registered: {}", label, hotkey);
    Ok(())
}

/// Unregister all screenshot shortcuts
pub fn unregister_screenshot_shortcuts(app: &AppHandle) -> Result<(), String> {
    let manager = app.global_shortcut();

    // We need to try common screenshot hotkeys since we don't track them
    // This is a simplified approach - ideally you'd store the registered shortcuts
    let common_shortcuts = [
        "Command+Shift+3",
        "Command+Shift+4",
        "Control+Shift+3",
        "Control+Shift+4",
        "Alt+Shift+3",
        "Alt+Shift+4",
    ];

    for shortcut_str in common_shortcuts {
        if let Ok(shortcut) = shortcut_str.parse::<Shortcut>() {
            if manager.is_registered(shortcut) {
                let _ = manager.unregister(shortcut);
            }
        }
    }

    println!("Unregistered screenshot hotkeys");
    Ok(())
}
