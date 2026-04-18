use crate::globals::APP_HANDLE;
use crate::{cron, docker, get_logger, session, tools, tray, weather, CRON_DB};
use oc_core::app_warn;
use session::SessionDB;
use std::sync::Arc;

/// Main application setup — called from `.setup()` in the Tauri builder chain.
pub(crate) fn app_setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Store global AppHandle for event emission
    let _ = APP_HANDLE.set(app.handle().clone());
    if cfg!(debug_assertions) {
        app.handle().plugin(
            tauri_plugin_log::Builder::default()
                .level(log::LevelFilter::Info)
                .build(),
        )?;
    }

    // macOS: custom app menu — Cmd+Q hides window instead of quitting
    #[cfg(target_os = "macos")]
    {
        use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder};
        let hide_quit = MenuItemBuilder::with_id("hide_quit", "Hide OpenComputer")
            .accelerator("CmdOrCtrl+Q")
            .build(app)?;
        let app_submenu = SubmenuBuilder::new(app, "OpenComputer")
            .about(None)
            .separator()
            .item(&hide_quit)
            .build()?;
        let edit_submenu = SubmenuBuilder::new(app, "Edit")
            .undo()
            .redo()
            .separator()
            .cut()
            .copy()
            .paste()
            .select_all()
            .build()?;
        let view_submenu = SubmenuBuilder::new(app, "View")
            .item(&PredefinedMenuItem::fullscreen(app, None)?)
            .build()?;
        let window_submenu = SubmenuBuilder::new(app, "Window")
            .minimize()
            .item(&PredefinedMenuItem::maximize(app, None)?)
            .close_window()
            .build()?;
        let menu = MenuBuilder::new(app)
            .item(&app_submenu)
            .item(&edit_submenu)
            .item(&view_submenu)
            .item(&window_submenu)
            .build()?;
        app.set_menu(menu)?;
        app.on_menu_event(|app_handle, event| {
            if event.id().as_ref() == "hide_quit" {
                use tauri::Manager;
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.hide();
                }
            }
        });
    }

    // Set up system tray icon with context menu
    tray::setup_tray(app)?;

    // Fix macOS theme-aware background to prevent flash on window resize
    #[cfg(target_os = "macos")]
    {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.with_webview(|webview| unsafe {
                let ns_window: &objc2_app_kit::NSWindow = &*webview.ns_window().cast();
                // Detect system dark mode via appearance name
                let is_dark = {
                    use objc2_app_kit::NSAppearanceCustomization;
                    let appearance = ns_window.effectiveAppearance();
                    let name = appearance.name();
                    name.to_string().contains("Dark")
                };
                let (r, g, b) = if is_dark {
                    (15.0 / 255.0, 15.0 / 255.0, 15.0 / 255.0)
                } else {
                    (1.0, 1.0, 1.0)
                };
                let bg_color =
                    objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, 1.0);
                ns_window.setBackgroundColor(Some(&bg_color));
            });
        }
    }

    // Start embedded HTTP/WS server for web clients and external tools
    {
        let session_db = oc_core::get_session_db().cloned().unwrap_or_else(|| {
            let db_path = session::db_path().expect("session db path");
            Arc::new(SessionDB::open(&db_path).expect("open session db"))
        });
        let event_bus = oc_core::get_event_bus().cloned().unwrap_or_else(|| {
            let bus: Arc<dyn oc_core::event_bus::EventBus> =
                Arc::new(oc_core::event_bus::BroadcastEventBus::new(256));
            oc_core::set_event_bus(bus.clone());
            bus
        });
        let project_db = oc_core::get_project_db().cloned().unwrap_or_else(|| {
            let db = Arc::new(oc_core::project::ProjectDB::new(session_db.clone()));
            let _ = db.migrate();
            db
        });
        // Read server config from config.json (bind address, API key)
        let store = oc_core::config::load_config().unwrap_or_default();
        let api_key = store.server.api_key.clone();
        let ctx = Arc::new(oc_server::AppContext {
            session_db,
            project_db,
            event_bus,
            chat_streams: Arc::new(oc_server::ws::chat_stream::ChatStreamRegistry::new()),
            chat_cancels: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
            api_key: api_key.clone(),
        });
        let config = oc_server::ServerConfig {
            bind_addr: store.server.bind_addr.clone(),
            api_key,
            cors_origins: Vec::new(),
        };
        tauri::async_runtime::spawn(async move {
            if let Err(e) = oc_server::start_server(config, ctx).await {
                eprintln!("[embedded-server] Failed to start: {}", e);
            }
        });
    }

    // Bridge oc-core EventBus → Tauri frontend (app_handle.emit).
    // Without this, events like `ask_user_request` / `plan_amended` emitted
    // from oc-core never reach the WebView.
    {
        use tauri::Emitter;
        use tokio::sync::broadcast::error::RecvError;
        let app_handle = app.handle().clone();
        let bus = oc_core::get_event_bus()
            .cloned()
            .expect("EventBus must be initialized before bridge spawn");
        tauri::async_runtime::spawn(async move {
            let mut rx = bus.subscribe();
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        // Hot-reload shortcuts when config:changed with category=shortcuts
                        if event.name == "config:changed" {
                            if let Some(cat) = event.payload.get("category").and_then(|v| v.as_str())
                            {
                                if cat == "shortcuts" {
                                    use tauri_plugin_global_shortcut::GlobalShortcutExt;
                                    crate::shortcuts::clear_chord_state();
                                    let manager = app_handle.global_shortcut();
                                    let _ = manager.unregister_all();
                                    if let Ok(store) = oc_core::config::load_config() {
                                        for binding in &store.shortcuts.bindings {
                                            if !binding.enabled || binding.keys.is_empty() {
                                                continue;
                                            }
                                            let key = if binding.is_chord() {
                                                binding.chord_parts()[0].to_string()
                                            } else {
                                                binding.keys.clone()
                                            };
                                            if let Ok(sc) = key.parse::<tauri_plugin_global_shortcut::Shortcut>() {
                                                let _ = manager.register(sc);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        let _ = app_handle.emit(&event.name, &event.payload);
                    }
                    Err(RecvError::Lagged(n)) => {
                        app_warn!(
                            "event_bus",
                            "tauri_bridge",
                            "Tauri bridge lagged {} events — some UI updates may be missed",
                            n
                        );
                        let _ = app_handle.emit(
                            "_event_bus_lagged",
                            serde_json::json!({ "missed": n }),
                        );
                        continue;
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        });
    }

    // Start cron scheduler on dedicated thread with its own tokio runtime
    if let (Some(cron_db), Ok(db_path)) = (CRON_DB.get(), session::db_path()) {
        if let Ok(session_db) = SessionDB::open(&db_path) {
            let _handle = cron::start_scheduler(cron_db.clone(), Arc::new(session_db));
            // Thread runs until app exits
        }
    }

    // Start background async tasks (channel auto-start, ACP discovery) — requires async runtime
    tauri::async_runtime::spawn(async {
        oc_core::start_background_tasks().await;
    });

    // Auto-start Docker SearXNG if previously configured
    auto_start_searxng_docker();

    // Start background weather cache refresh
    weather::start_background_refresh();

    // Register global shortcuts from config (chord-aware: only first parts for chords)
    {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let store = oc_core::config::load_config().unwrap_or_default();
        for binding in &store.shortcuts.bindings {
            if !binding.enabled || binding.keys.is_empty() {
                continue;
            }
            // For chord bindings, only register the first part
            let key_to_register = if binding.is_chord() {
                binding.chord_parts()[0].to_string()
            } else {
                binding.keys.clone()
            };
            if let Ok(shortcut) = key_to_register.parse::<tauri_plugin_global_shortcut::Shortcut>()
            {
                if let Err(e) = app.global_shortcut().register(shortcut) {
                    eprintln!(
                        "[setup] Failed to register shortcut '{}' ({}): {}",
                        binding.id, key_to_register, e
                    );
                }
            }
        }
    }

    Ok(())
}

/// If SearXNG is docker-managed and enabled, auto-start the container on app launch.
fn auto_start_searxng_docker() {
    let store = match oc_core::config::load_config() {
        Ok(s) => s,
        Err(_) => return,
    };

    // Check: docker-managed + SearXNG enabled
    let docker_managed = store.web_search.searxng_docker_managed.unwrap_or(false);
    let searxng_enabled = store
        .web_search
        .providers
        .iter()
        .any(|e| e.id == tools::web_search::WebSearchProvider::Searxng && e.enabled);

    if !docker_managed || !searxng_enabled {
        return;
    }

    // Spawn background task — don't block app startup (reuse existing Tauri runtime)
    tauri::async_runtime::spawn(async {
        let status = docker::status().await;
        if !status.docker_installed || status.docker_not_running {
            if let Some(logger) = get_logger() {
                logger.log(
                    "warn",
                    "docker",
                    "auto_start",
                    "Docker not available, skipping SearXNG auto-start",
                    None,
                    None,
                    None,
                );
            }
            return;
        }
        if status.container_running && status.health_ok {
            // Already running, nothing to do
            return;
        }
        if status.container_exists && !status.container_running {
            if let Some(logger) = get_logger() {
                logger.log(
                    "info",
                    "docker",
                    "auto_start",
                    "Auto-starting SearXNG container...",
                    None,
                    None,
                    None,
                );
            }
            if let Err(e) = docker::start().await {
                if let Some(logger) = get_logger() {
                    logger.log(
                        "error",
                        "docker",
                        "auto_start",
                        "Failed to auto-start SearXNG",
                        Some(e.to_string()),
                        None,
                        None,
                    );
                }
            }
        }
    });
}
