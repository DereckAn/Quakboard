#![allow(unexpected_cfgs)]

mod cleanup;
mod clipboard;
mod colors;
mod commands;
mod db;
mod shortcuts;
mod sync;

use clipboard::{spawn_clipboard_listener, ClipboardMonitor};
use commands::AppState;
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
#[cfg(target_os = "macos")]
use tauri::path::BaseDirectory;
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // WebKitGTK's DMABUF renderer is the correct path on AMD/Intel (and modern
    // NVIDIA), and it's what makes transparent windows repaint cleanly. But it
    // blank-screens on the NVIDIA *proprietary* driver under Wayland. Disable it
    // only for that exact combo so everyone else keeps GPU rendering. Respect an
    // explicit user override. /proc/driver/nvidia/version exists only with the
    // proprietary driver (not nouveau, which handles DMABUF fine).
    #[cfg(target_os = "linux")]
    {
        let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some()
            || std::env::var("XDG_SESSION_TYPE").as_deref() == Ok("wayland");
        let nvidia_proprietary = std::path::Path::new("/proc/driver/nvidia/version").exists();

        if on_wayland
            && nvidia_proprietary
            && std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none()
        {
            std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }

    tauri::Builder::default()
        // Debe registrarse PRIMERO. En Wayland (Hyprland) los hotkeys globales no
        // funcionan, así que el atajo se define en el compositor y lanza el binario
        // con `--toggle`; esa segunda instancia reenvía el argumento a la que ya
        // está corriendo para mostrar/ocultar la ventana.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                if argv.iter().any(|arg| arg == "--toggle") {
                    match window.is_visible() {
                        Ok(true) => {
                            let _ = window.hide();
                        }
                        _ => {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                } else {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            #[cfg(target_os = "macos")]
            Some(vec!["--hidden"]),
            #[cfg(not(target_os = "macos"))]
            Some(vec![]),
        ))
        .setup(|app| {
            // ================================================================
            // CONFIGURACIÓN ESPECÍFICA POR PLATAFORMA
            // ================================================================

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            // Abrir DevTools automáticamente en desarrollo
            #[cfg(debug_assertions)]
            app.get_webview_window("main").unwrap().open_devtools();

            // HACK(dereck): WebKitGTK 2.54 refuses asset:// images on a page
            // from another origin. Only `tauri dev` hits it: its page comes
            // from http://localhost:1420, while release builds load from
            // tauri://. Remove this block and the `webkit2gtk` line in
            // Cargo.toml once Tauri allows it itself; check with `tauri dev`
            // that images still show
            // (https://github.com/tauri-apps/tauri/issues/16201).
            #[cfg(all(target_os = "linux", debug_assertions))]
            {
                use webkit2gtk::WebViewExt;
                let window = app.get_webview_window("main").unwrap();
                if let Err(e) = window.with_webview(|webview| {
                    webview.inner().set_cors_allowlist(&["asset://*/*"]);
                }) {
                    eprintln!("Failed to allow asset:// images in dev: {e}");
                }
            }

            // ================================================================
            // CONFIGURACIÓN DEL SYSTEM TRAY
            // ================================================================

            // Crear menú del tray
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let show_item = MenuItem::with_id(app, "show", "Show Window", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[&show_item, &quit_item])?;

            #[cfg(target_os = "macos")]
            let tray_icon = {
                // Compile-time embedded bytes to avoid filesystem lookups.
                const ICON_BYTES: &[u8] = include_bytes!("../assets/images/tray_icon_template.png");

                let template_icon = Image::from_bytes(ICON_BYTES).ok().or_else(|| {
                    app.path()
                        .resolve("icons/tray_icon_template.png", BaseDirectory::Resource)
                        .ok()
                        .and_then(|path| Image::from_path(&path).ok())
                });

                template_icon
                    .or_else(|| app.default_window_icon().cloned())
                    .expect("Failed to obtain tray icon image")
            };

            #[cfg(not(target_os = "macos"))]
            let tray_icon = app.default_window_icon().unwrap().clone();

            // Crear el tray icon
            let tray_builder = TrayIconBuilder::new()
                .icon(tray_icon)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        // println!("🛑 Quit clicked from tray");
                        app.exit(0);
                    }
                    "show" => {
                        // println!("👁️ Show window clicked from tray");
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    _ => {}
                });

            #[cfg(target_os = "macos")]
            let tray_builder = tray_builder.icon_as_template(true);

            let tray = tray_builder.build(app)?;

            app.manage(std::sync::Mutex::new(tray));

            // Arranque de la ventana principal.
            // En Linux release la app se inicia con el sistema (Hyprland exec-once)
            // y debe quedarse oculta en segundo plano hasta que se presione el hotkey.
            // En dev se muestra para poder trabajar con ella.
            //
            // Excepción (#8): si el binario se arranca en frío CON `--toggle` (p.ej.
            // SUPER+V cuando la app no estaba corriendo), mostramos la ventana en vez
            // de quedar como un proceso invisible en segundo plano.
            let cold_started_with_toggle = std::env::args().skip(1).any(|arg| arg == "--toggle");
            if let Some(main_window) = app.get_webview_window("main") {
                #[cfg(all(target_os = "linux", not(debug_assertions)))]
                {
                    if cold_started_with_toggle {
                        let _ = main_window.show();
                        let _ = main_window.set_focus();
                    } else {
                        let _ = main_window.hide();
                    }
                }

                #[cfg(not(all(target_os = "linux", not(debug_assertions))))]
                {
                    let _ = cold_started_with_toggle;
                    let _ = main_window.show();
                    let _ = main_window.set_focus();
                    let _ = main_window.set_always_on_top(true);
                    let _ = main_window.set_always_on_top(false);
                }
            }

            // ================================================================
            // CONFIGURACIÓN COMÚN (todas las plataformas)
            // ================================================================

            let app_data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");

            std::fs::create_dir_all(&app_data_dir).expect("Failed to create app data dir");

            // Create images directory for storing clipboard images
            let images_dir = app_data_dir.join("images");
            std::fs::create_dir_all(&images_dir).expect("Failed to create images dir");
            println!("Images directory: {:?}", images_dir);

            let db_path = app_data_dir.join("clipboard.db");
            let db_path_str = db_path.to_str().unwrap().to_string();
            let images_dir_str = images_dir.to_string_lossy().to_string();
            let app_data_dir_str = app_data_dir.to_string_lossy().to_string();

            println!("Database path: {db_path_str}");

            app.manage(Mutex::new(AppState {
                db_path: db_path_str.clone(),
                images_dir: images_dir_str,
                app_data_dir: app_data_dir_str,
            }));

            // Always registered, so the Devices screen can turn sync on later.
            let sync_runtime = sync::runtime::SyncRuntime::new(&app_data_dir);
            let start_sync = sync_runtime.is_enabled();
            app.manage(sync_runtime);
            if start_sync {
                let sync_app = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let runtime = sync_app.state::<sync::runtime::SyncRuntime>();
                    if let Err(e) = runtime.start(&sync_app).await {
                        eprintln!("Failed to start LAN sync: {e}");
                    }
                });
            }

            let app_handle = app.handle().clone();
            let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
            let monitor = ClipboardMonitor::new(db_path_str.clone(), app_handle.clone(), event_rx);

            spawn_clipboard_listener(event_tx);

            tauri::async_runtime::spawn(async move {
                monitor.start().await;
            });

            // Opt-in capture-folder watcher (Lab feature). Default OFF.
            // Held in managed state so it can be toggled live via set_folder_watcher.
            app.manage(Mutex::new(clipboard::folder_watcher::WatcherState::default()));
            if clipboard::folder_watcher::is_enabled(&app_data_dir) {
                let watcher_config =
                    clipboard::folder_watcher::config_from_settings(&app_data_dir, &db_path_str);
                match clipboard::folder_watcher::build(app.handle().clone(), watcher_config) {
                    Ok(debouncer) => {
                        app.state::<Mutex<clipboard::folder_watcher::WatcherState>>()
                            .lock()
                            .unwrap()
                            .set(debouncer);
                        clipboard::state::set_suppress_screenshot_bytes(true);
                        println!("📂 Capture-folder watcher enabled");
                    }
                    Err(e) => eprintln!("📂 Failed to start capture-folder watcher: {e}"),
                }
            }

            // Tarea automática de limpieza (cada 24 horas)
            let cleanup_db_path = db_path_str.clone();
            let cleanup_app_data_dir = app_data_dir.clone();
            tauri::async_runtime::spawn(async move {
                use tokio::time::{sleep, Duration};

                loop {
                    // Esperar 24 horas (86400 segundos)
                    sleep(Duration::from_secs(86400)).await;

                    println!("🧹 Running daily automatic cleanup...");

                    // Leer settings desde localStorage (formato JSON)
                    let settings_path = cleanup_app_data_dir.join("settings.json");
                    let settings =
                        std::fs::read_to_string(&settings_path)
                            .ok()
                            .and_then(|contents| {
                                serde_json::from_str::<serde_json::Value>(&contents).ok()
                            });

                    if let Ok(repo) =
                        crate::db::repository::ClipboardRepository::new(&cleanup_db_path)
                    {
                        let conn = &repo.conn;

                        // Limpiar items por retención de días (si está habilitado)
                        if let Some(ref settings_json) = settings {
                            if settings_json
                                .get("retentionEnabled")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                let retention_days = settings_json
                                    .get("retentionDays")
                                    .and_then(|v| v.as_i64())
                                    .map(|d| d as i32);

                                match crate::cleanup::cleanup_old_items(conn, retention_days) {
                                    Ok(deleted) if deleted > 0 => {
                                        println!(
                                            "🧹 Deleted {} old items (retention policy)",
                                            deleted
                                        );
                                    }
                                    Err(e) => eprintln!("❌ Failed to cleanup old items: {}", e),
                                    _ => {}
                                }
                            }

                            // Limpiar items excedentes (si está habilitado)
                            if settings_json
                                .get("maxItemsEnabled")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                let max_items = settings_json
                                    .get("maxLocalItems")
                                    .and_then(|v| v.as_i64())
                                    .map(|m| m as i32);

                                match crate::cleanup::cleanup_excess_items(conn, max_items) {
                                    Ok(deleted) if deleted > 0 => {
                                        println!("🧹 Deleted {} excess items (max limit)", deleted);
                                    }
                                    Err(e) => eprintln!("❌ Failed to cleanup excess items: {}", e),
                                    _ => {}
                                }
                            }
                        }
                    }

                    println!("✅ Daily cleanup completed");
                }
            });

            // Cargar hotkey guardado o usar el predeterminado.
            // En Linux el default es Super+Shift+V (encaja con Hyprland); en
            // macOS/Windows se mantiene CommandOrControl+Shift+V.
            #[cfg(target_os = "linux")]
            let default_hotkey = "Super+Shift+V";
            #[cfg(not(target_os = "linux"))]
            let default_hotkey = "CommandOrControl+Shift+V";
            let saved_hotkey = app_data_dir
                .join("settings.json")
                .exists()
                .then(|| {
                    std::fs::read_to_string(app_data_dir.join("settings.json"))
                        .ok()
                        .and_then(|contents| {
                            serde_json::from_str::<serde_json::Value>(&contents).ok()
                        })
                        .and_then(|settings| {
                            settings
                                .get("hotkey")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                        })
                })
                .flatten()
                .unwrap_or_else(|| default_hotkey.to_string());

            println!("Registering global hotkey: {saved_hotkey}");

            let app_handle = app.handle().clone();
            match shortcuts::register_shortcut_upon_start(&app_handle, &saved_hotkey) {
                Ok(_) => {
                    println!("Clipboard manager initialized");
                    println!("Global hotkey registered:{saved_hotkey}");
                }
                Err(e) => {
                    eprintln!("Warning: Failed to register globalshortcut '{saved_hotkey}': {e}");
                    eprintln!("The application will continue without the global shortcut.");
                    eprintln!("You can try changing it in Settings.");
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                window.hide().unwrap();
                api.prevent_close();
            }
            tauri::WindowEvent::Focused(focused) => {
                if !focused {
                    if let Some(state) = window.try_state::<Mutex<AppState>>() {
                        if let Ok(app_state) = state.lock() {
                            let db_path = app_state.db_path.clone();
                            let event_window = window.clone();
                            tauri::async_runtime::spawn(async move {
                                if let Ok(repo) =
                                    crate::db::repository::ClipboardRepository::new(&db_path)
                                {
                                    if let Ok(removed) = repo.cleanup_missing_file_records() {
                                        if !removed.is_empty() {
                                            let _ = event_window
                                                .emit("clipboard-items-removed", removed);
                                        }
                                    }
                                }
                            });
                        }
                    }

                    #[cfg(target_os = "windows")]
                    {
                        // En Windows: delay para evitar ocultar durante drag
                        let window_clone = window.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(100));
                            // Verificar si la ventana sigue sin foco
                            if let Ok(is_focused) = window_clone.is_focused() {
                                if !is_focused && !clipboard::state::should_keep_window_open() {
                                    let _ = window_clone.hide();
                                }
                            }
                        });
                    }

                    #[cfg(not(target_os = "windows"))]
                    {
                        // macOS y Linux: ocultar inmediatamente, unless one of our
                        // own dialogs (e.g. a folder picker) took the focus.
                        if !clipboard::state::should_keep_window_open() {
                            let _ = window.hide();
                        }
                    }
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_clipboard_item,
            commands::create_clipboard_item,
            commands::upsert_clipboard_item,
            commands::bump_clipboard_item,
            commands::update_clipboard_item,
            commands::delete_clipboard_item,
            commands::clear_all_clipboard_items,
            commands::read_from_clipboard,
            commands::write_to_clipboard,
            commands::paste_item,
            commands::convert_color_formats,
            commands::extract_domain_from_url,
            commands::fetch_link_metadata,
            commands::remove_duplicate_items,
            commands::cleanup_missing_clipboard_files,
            commands::toggle_window_visibility,
            commands::get_setting,
            commands::save_setting,
            commands::set_folder_watcher,
            commands::save_cleanup_settings,
            commands::get_cleanup_settings,
            commands::update_global_hotkey,
            commands::get_current_shortcut,
            commands::unregister_shortcut,
            commands::get_clipboard_items_paginated,
            commands::count_clipboard_items,
            commands::search_clipboard_items_fts,
            commands::count_search_results_fts,
            commands::cleanup_old_items,
            commands::cleanup_excess_items,
            commands::get_database_size,
            commands::optimize_database,
            commands::get_database_stats,
            commands::test_cleanup_preview,
            commands::test_force_cleanup,
            commands::enable_autostart,
            commands::disable_autostart,
            commands::is_autostart_enabled,
            commands::quit_app,
            commands::set_tray_visible,
            commands::is_tray_visible,
            commands::write_image_to_clipboard,
            commands::ensure_thumbnail,
            commands::write_file_to_clipboard,
            commands::get_lab_features,
            commands::install_feature,
            commands::uninstall_feature,
            commands::enable_feature,
            commands::capture_full_screenshot,
            commands::capture_region_screenshot,
            commands::get_platform,
            commands::sync_get_status,
            commands::sync_set_enabled,
            commands::sync_set_images_enabled,
            commands::sync_list_peers,
            commands::sync_list_nearby,
            commands::sync_set_accepting_pairing,
            commands::sync_pair_start,
            commands::sync_pair_by_address,
            commands::sync_pair_submit_code,
            commands::sync_pair_cancel,
            commands::sync_unpair,
            commands::sync_send_file,
            commands::sync_fetch_file,
            commands::sync_cancel_fetch,
            commands::sync_get_download_dir,
            commands::sync_choose_download_dir,
            commands::choose_folder,
            commands::update_screenshot_hotkeys,
            commands::unregister_screenshot_hotkeys,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
