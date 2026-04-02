#[macro_use]
mod logging;
pub mod acp;
pub(crate) mod acp_control;
mod agent;
mod agent_config;
mod agent_loader;
pub mod backup;
mod browser_state;
mod canvas_db;
pub mod channel;
mod chat_engine;
mod commands;
mod context_compact;
pub mod crash_journal;
mod cron;
mod dashboard;
mod dev_tools;
mod docker;
mod failover;
mod file_extract;
mod memory;
mod memory_extract;
mod oauth;
pub mod paths;
mod permissions;
mod plan;
mod process_registry;
pub mod provider;
mod sandbox;
pub mod self_diagnosis;
pub mod session;
mod skills;
mod slash_commands;
mod subagent;
mod system_prompt;
mod tools;
mod tray;
mod url_preview;
mod user_config;
mod weather;
#[cfg(target_os = "macos")]
mod weather_location_macos;

use agent::AssistantAgent;
use logging::{AppLogger, LogDB};
use oauth::TokenData;
use provider::ProviderStore;
use session::SessionDB;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Truncate a string to at most `max_bytes` bytes on a valid UTF-8 char boundary.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    // floor_char_boundary is nightly-only, so do it manually
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ── Chord shortcut state machine ────────────────────────────────
/// Tracks pending first-part of a chord shortcut (e.g. after Ctrl+K in "Ctrl+K Ctrl+C").
struct ChordPending {
    /// Action IDs and their expected second-shortcut strings
    completions: Vec<(String, String)>,
    /// Deadline after which the pending state expires
    deadline: std::time::Instant,
}

static CHORD_STATE: std::sync::OnceLock<std::sync::Mutex<Option<ChordPending>>> =
    std::sync::OnceLock::new();

fn chord_state() -> &'static std::sync::Mutex<Option<ChordPending>> {
    CHORD_STATE.get_or_init(|| std::sync::Mutex::new(None))
}

/// Timeout for the second part of a chord shortcut.
const CHORD_TIMEOUT_MS: u64 = 1500;

static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();
static APP_LOGGER: std::sync::OnceLock<AppLogger> = std::sync::OnceLock::new();
static MEMORY_BACKEND: std::sync::OnceLock<Arc<dyn memory::MemoryBackend>> =
    std::sync::OnceLock::new();
static CRON_DB: std::sync::OnceLock<Arc<cron::CronDB>> = std::sync::OnceLock::new();
static SESSION_DB: std::sync::OnceLock<Arc<SessionDB>> = std::sync::OnceLock::new();
static SUBAGENT_CANCELS: std::sync::OnceLock<Arc<subagent::SubagentCancelRegistry>> =
    std::sync::OnceLock::new();
static ACP_MANAGER: std::sync::OnceLock<Arc<acp_control::AcpSessionManager>> =
    std::sync::OnceLock::new();
static CHANNEL_REGISTRY: std::sync::OnceLock<Arc<channel::ChannelRegistry>> =
    std::sync::OnceLock::new();
static CHANNEL_DB: std::sync::OnceLock<Arc<channel::ChannelDB>> = std::sync::OnceLock::new();

/// Get stored AppLogger for global logging
pub fn get_logger() -> Option<&'static AppLogger> {
    APP_LOGGER.get()
}

/// Get stored AppHandle for global event emission (e.g., command approval)
pub fn get_app_handle() -> Option<&'static tauri::AppHandle> {
    APP_HANDLE.get()
}

/// Get stored MemoryBackend for memory operations
pub fn get_memory_backend() -> Option<&'static Arc<dyn memory::MemoryBackend>> {
    MEMORY_BACKEND.get()
}

/// Get stored CronDB for cron operations (used by agent tool)
pub fn get_cron_db() -> Option<&'static Arc<cron::CronDB>> {
    CRON_DB.get()
}

/// Get stored SessionDB for sub-agent operations
pub fn get_session_db() -> Option<&'static Arc<SessionDB>> {
    SESSION_DB.get()
}

/// Get stored SubagentCancelRegistry for sub-agent cancellation
pub fn get_subagent_cancels() -> Option<&'static Arc<subagent::SubagentCancelRegistry>> {
    SUBAGENT_CANCELS.get()
}

/// Get stored AcpSessionManager for ACP control plane operations
pub fn get_acp_manager() -> Option<&'static Arc<acp_control::AcpSessionManager>> {
    ACP_MANAGER.get()
}

/// Get stored ChannelRegistry for IM channel operations
pub fn get_channel_registry() -> Option<&'static Arc<channel::ChannelRegistry>> {
    CHANNEL_REGISTRY.get()
}

/// Get stored ChannelDB for channel conversation management
pub fn get_channel_db() -> Option<&'static Arc<channel::ChannelDB>> {
    CHANNEL_DB.get()
}

/// If SearXNG is docker-managed and enabled, auto-start the container on app launch.
fn auto_start_searxng_docker() {
    let store = match provider::load_store() {
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

    // Spawn background task — don't block app startup
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new()
            .expect("Failed to create runtime for SearXNG auto-start");
        rt.block_on(async {
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
    });
}

pub(crate) struct AppState {
    pub(crate) agent: Mutex<Option<AssistantAgent>>,
    pub(crate) auth_result: Arc<Mutex<Option<anyhow::Result<TokenData>>>>,
    /// Provider configuration store
    pub(crate) provider_store: Mutex<ProviderStore>,
    /// Reasoning effort for Codex models
    pub(crate) reasoning_effort: Mutex<String>,
    /// Store token info so we can rebuild agent when model changes
    pub(crate) codex_token: Mutex<Option<(String, String)>>, // (access_token, account_id)
    /// Currently active agent ID
    pub(crate) current_agent_id: Mutex<String>,
    /// Session database
    pub(crate) session_db: Arc<SessionDB>,
    /// Cancel flag for stopping ongoing chat
    pub(crate) chat_cancel: Arc<AtomicBool>,
    /// Log database
    pub(crate) log_db: Arc<LogDB>,
    /// Async logger
    pub(crate) logger: AppLogger,
    /// Cron database
    pub(crate) cron_db: Arc<cron::CronDB>,
    /// Sub-agent cancel registry
    pub(crate) subagent_cancels: Arc<subagent::SubagentCancelRegistry>,
    /// Channel stream cancel registry
    pub(crate) channel_cancels: Arc<channel::ChannelCancelRegistry>,
}

/// Execute a shortcut action by its id (shared by single-combo and chord paths).
fn execute_shortcut_action(app_handle: &tauri::AppHandle, action_id: &str, _shortcut_str: &str) {
    use tauri::Emitter;
    use tauri::Manager;
    match action_id {
        "quickChat" => {
            toggle_quickchat_window(app_handle);
        }
        "openSettings" => {
            if let Some(window) = app_handle.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            let _ = app_handle.emit("open-settings", ());
        }
        other => {
            let _ = app_handle.emit("shortcut-triggered", other.to_string());
        }
    }
}

/// Toggle the independent quick-chat window. Creates it on first use.
pub(crate) fn toggle_quickchat_window(app_handle: &tauri::AppHandle) {
    use tauri::Manager;

    if let Some(win) = app_handle.get_webview_window("quickchat") {
        // Window exists — toggle visibility
        if win.is_visible().unwrap_or(false) {
            let _ = win.hide();
        } else {
            let _ = win.show();
            let _ = win.set_focus();
        }
        return;
    }

    // Create the quick-chat window for the first time
    let url = tauri::WebviewUrl::App("index.html?window=quickchat".into());
    match tauri::WebviewWindowBuilder::new(app_handle, "quickchat", url)
        .title("Quick Chat")
        .inner_size(680.0, 460.0)
        .min_inner_size(500.0, 420.0)
        .resizable(false)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .visible(true)
        .center()
        .build()
    {
        Ok(win) => {
            #[cfg(target_os = "macos")]
            {
                let _ = win.with_webview(|webview| unsafe {
                    let ns_window: &objc2_app_kit::NSWindow = &*webview.ns_window().cast();

                    // Transparent background so CSS border-radius works
                    let clear_color = objc2_app_kit::NSColor::colorWithSRGBRed_green_blue_alpha(
                        0.0, 0.0, 0.0, 0.0,
                    );
                    ns_window.setBackgroundColor(Some(&clear_color));

                    // Highest window level — above almost everything (including other always-on-top windows)
                    ns_window.setLevel(
                        objc2_app_kit::NSWindowLevel::from(25_isize), // NSStatusWindowLevel
                    );

                    // Visible on ALL Spaces / desktops, including full-screen apps
                    ns_window.setCollectionBehavior(
                        objc2_app_kit::NSWindowCollectionBehavior::CanJoinAllSpaces
                            | objc2_app_kit::NSWindowCollectionBehavior::FullScreenAuxiliary,
                    );
                });
            }
            let _ = win.set_focus();
        }
        Err(e) => {
            if let Some(logger) = crate::get_logger() {
                logger.log(
                    "error",
                    "shortcut",
                    "toggle_quickchat_window",
                    &format!("Failed to create quickchat window: {}", e),
                    None,
                    None,
                    None,
                );
            }
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize directory structure
    // NOTE: log::error! is intentional here — AppLogger is not yet initialized at this point
    if let Err(e) = paths::ensure_dirs() {
        log::error!("Failed to initialize data directories: {}", e);
    }

    // Ensure default agent exists
    if let Err(e) = agent_loader::ensure_default_agent() {
        log::error!("Failed to ensure default agent: {}", e);
    }

    // Load provider store at startup
    let initial_store = provider::load_store().unwrap_or_default();

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a second instance is launched, show and focus the existing window
            use tauri::Manager;
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app_handle, shortcut, event| {
                    if event.state != tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        return;
                    }
                    use tauri::Emitter;
                    use tauri_plugin_global_shortcut::GlobalShortcutExt;

                    let shortcut_str = shortcut.to_string();
                    let store = provider::load_store().unwrap_or_default();

                    // ── Step 1: Check if this completes a pending chord ──
                    {
                        let mut state = chord_state().lock().unwrap_or_else(|e| e.into_inner());
                        if let Some(pending) = state.as_ref() {
                            if std::time::Instant::now() < pending.deadline {
                                // Check if this shortcut matches any expected second part
                                if let Some((action_id, _second_str)) = pending
                                    .completions
                                    .iter()
                                    .find(|(_, s)| {
                                        s.parse::<tauri_plugin_global_shortcut::Shortcut>()
                                            .map(|parsed| parsed == *shortcut)
                                            .unwrap_or(false)
                                    })
                                    .cloned()
                                {
                                    let action_id_clone = action_id.clone();
                                    // Unregister temporary second-part shortcuts
                                    let manager = app_handle.global_shortcut();
                                    for (_, s) in &pending.completions {
                                        if let Ok(sc) =
                                            s.parse::<tauri_plugin_global_shortcut::Shortcut>()
                                        {
                                            let _ = manager.unregister(sc);
                                        }
                                    }
                                    *state = None;
                                    drop(state);

                                    // Execute the chord action
                                    execute_shortcut_action(
                                        app_handle,
                                        &action_id_clone,
                                        &shortcut_str,
                                    );
                                    return;
                                }
                            }
                            // Pending expired or no match — clean up temporary registrations
                            let manager = app_handle.global_shortcut();
                            for (_, s) in &pending.completions {
                                if let Ok(sc) = s.parse::<tauri_plugin_global_shortcut::Shortcut>()
                                {
                                    let _ = manager.unregister(sc);
                                }
                            }
                            *state = None;
                        }
                    }

                    // ── Step 2: Check if this is the first part of any chord binding ──
                    let chord_matches: Vec<(String, String)> = store
                        .shortcuts
                        .bindings
                        .iter()
                        .filter(|b| b.enabled && b.is_chord())
                        .filter_map(|b| {
                            let parts = b.chord_parts();
                            if parts.len() == 2 {
                                if let Ok(first) =
                                    parts[0].parse::<tauri_plugin_global_shortcut::Shortcut>()
                                {
                                    if first == *shortcut {
                                        return Some((b.id.clone(), parts[1].to_string()));
                                    }
                                }
                            }
                            None
                        })
                        .collect();

                    if !chord_matches.is_empty() {
                        // Register second-part shortcuts temporarily
                        let manager = app_handle.global_shortcut();
                        for (_, second_str) in &chord_matches {
                            if let Ok(sc) =
                                second_str.parse::<tauri_plugin_global_shortcut::Shortcut>()
                            {
                                let _ = manager.register(sc);
                            }
                        }
                        // Set pending state
                        let deadline = std::time::Instant::now()
                            + std::time::Duration::from_millis(CHORD_TIMEOUT_MS);
                        *chord_state().lock().unwrap_or_else(|e| e.into_inner()) = Some(ChordPending {
                            completions: chord_matches.clone(),
                            deadline,
                        });
                        // Emit visual feedback to frontend
                        let _ = app_handle.emit("chord-first-pressed", shortcut_str.clone());
                        // Spawn timeout cleanup thread
                        let app_clone = app_handle.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(std::time::Duration::from_millis(
                                CHORD_TIMEOUT_MS + 50,
                            ));
                            let mut state = chord_state().lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(pending) = state.take() {
                                let manager = app_clone.global_shortcut();
                                for (_, s) in &pending.completions {
                                    if let Ok(sc) =
                                        s.parse::<tauri_plugin_global_shortcut::Shortcut>()
                                    {
                                        let _ = manager.unregister(sc);
                                    }
                                }
                                let _ = app_clone.emit("chord-timeout", ());
                            }
                        });
                        return;
                    }

                    // ── Step 3: Single-combo binding — look up directly ──
                    let action_id = store
                        .shortcuts
                        .bindings
                        .iter()
                        .find(|b| {
                            b.enabled
                                && !b.is_chord()
                                && b.keys
                                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                                    .map(|s| s == *shortcut)
                                    .unwrap_or(false)
                        })
                        .map(|b| b.id.clone());

                    if let Some(ref id) = action_id {
                        execute_shortcut_action(app_handle, id, &shortcut_str);
                    } else {
                        let _ = app_handle.emit("shortcut-triggered", shortcut_str);
                    }
                })
                .build(),
        )
        .on_window_event(|window, event| {
            // Intercept window close → hide instead of quit (app stays resident in tray)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let label = window.label();
                if label == "main" || label == "quickchat" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .setup(|app| {
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
                use tauri::menu::{
                    MenuBuilder, MenuItemBuilder, PredefinedMenuItem, SubmenuBuilder,
                };
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

            // Start cron scheduler on dedicated thread with its own tokio runtime
            if let (Some(cron_db), Ok(db_path)) = (CRON_DB.get(), session::db_path()) {
                if let Ok(session_db) = SessionDB::open(&db_path) {
                    let _handle = cron::start_scheduler(cron_db.clone(), Arc::new(session_db));
                    // Thread runs until app exits
                }
            }

            // Auto-start Docker SearXNG if previously configured
            auto_start_searxng_docker();

            // Start background weather cache refresh
            weather::start_background_refresh();

            // Register global shortcuts from config (chord-aware: only first parts for chords)
            {
                use tauri_plugin_global_shortcut::GlobalShortcutExt;
                let store = provider::load_store().unwrap_or_default();
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
                    if let Ok(shortcut) =
                        key_to_register.parse::<tauri_plugin_global_shortcut::Shortcut>()
                    {
                        if let Err(e) = app.global_shortcut().register(shortcut) {
                            log::warn!(
                                "Failed to register shortcut '{}' ({}): {}",
                                binding.id,
                                key_to_register,
                                e
                            );
                        }
                    }
                }
            }

            Ok(())
        })
        .manage({
            // Initialize the SessionDB
            let db_path = session::db_path().expect("Failed to resolve database path");
            let session_db =
                Arc::new(SessionDB::open(&db_path).expect("Failed to open session database"));

            // Initialize the LogDB and AppLogger
            let log_db_path = logging::db_path().expect("Failed to resolve log database path");
            let log_db = Arc::new(LogDB::open(&log_db_path).expect("Failed to open log database"));

            // Load log config and cleanup old logs
            let log_config = logging::load_log_config().unwrap_or_default();
            let _ = log_db.cleanup_old(log_config.max_age_days);
            // Clean up old log files
            let _ = logging::cleanup_old_log_files(log_config.max_age_days);
            let logs_dir = paths::logs_dir().expect("Failed to resolve logs directory");
            let logger = AppLogger::new(log_db.clone(), logs_dir);
            logger.update_config(log_config);

            // Store logger globally for access from non-State contexts
            let _ = APP_LOGGER.set(logger.clone());

            // Initialize the MemoryDB
            let memory_db_path =
                paths::memory_db_path().expect("Failed to resolve memory database path");
            let memory_backend: Arc<dyn memory::MemoryBackend> = Arc::new(
                memory::SqliteMemoryBackend::open(&memory_db_path)
                    .expect("Failed to open memory database"),
            );
            let _ = MEMORY_BACKEND.set(memory_backend);

            // Auto-initialize embedder if enabled in config
            if let Some(backend) = MEMORY_BACKEND.get() {
                match provider::load_store() {
                    Ok(store) if store.embedding.enabled => {
                        match memory::create_embedding_provider(&store.embedding) {
                            Ok(emb_provider) => {
                                backend.set_embedder(emb_provider);
                                logger.log(
                                    "info",
                                    "memory",
                                    "embedding",
                                    "Embedding provider auto-initialized on startup",
                                    None,
                                    None,
                                    None,
                                );
                            }
                            Err(e) => {
                                logger.log(
                                    "warn",
                                    "memory",
                                    "embedding",
                                    &format!("Failed to auto-initialize embedding provider: {}", e),
                                    None,
                                    None,
                                    None,
                                );
                            }
                        }
                    }
                    _ => {} // Embedding not enabled or config load failed — skip silently
                }
            }

            // Initialize the CronDB (scheduler started in .setup() where tokio runtime is available)
            let cron_db_path = paths::cron_db_path().expect("Failed to resolve cron database path");
            let cron_db =
                Arc::new(cron::CronDB::open(&cron_db_path).expect("Failed to open cron database"));
            let _ = CRON_DB.set(cron_db.clone());

            // Log system startup
            logger.log(
                "info",
                "system",
                "lib::run",
                "OpenComputer started",
                None,
                None,
                None,
            );

            // Send welcome notification on startup
            if let Some(handle) = APP_HANDLE.get() {
                use tauri::Emitter;
                let payload = serde_json::json!({
                    "type": "agent_notification",
                    "title": "欢迎使用 OpenComputer",
                    "body": "文文，准备好开始今天的工作了吗？",
                });
                let _ = handle.emit("agent:send_notification", payload);
            }

            // Initialize sub-agent cancel registry
            let subagent_cancels = Arc::new(subagent::SubagentCancelRegistry::new());
            let _ = SUBAGENT_CANCELS.set(subagent_cancels.clone());
            let _ = SESSION_DB.set(session_db.clone());

            // Initialize channel cancel registry
            let channel_cancels = Arc::new(channel::ChannelCancelRegistry::new());

            // Clean up orphan sub-agent runs from previous app session
            subagent::cleanup_orphan_runs(&session_db);

            // Initialize IM Channel system
            {
                let (mut registry, inbound_rx) = channel::ChannelRegistry::new(256);

                // Register built-in channel plugins
                registry.register_plugin(Arc::new(channel::telegram::TelegramPlugin::new()));
                registry.register_plugin(Arc::new(channel::wechat::WeChatPlugin::new()));
                registry.register_plugin(Arc::new(channel::slack::SlackPlugin::new()));
                registry.register_plugin(Arc::new(channel::feishu::FeishuPlugin::new()));
                registry.register_plugin(Arc::new(channel::discord::DiscordPlugin::new()));
                registry.register_plugin(Arc::new(channel::qqbot::QqBotPlugin::new()));

                let registry = Arc::new(registry);
                let channel_db = Arc::new(channel::ChannelDB::new(session_db.clone()));

                // Run channel DB migration
                if let Err(e) = channel_db.migrate() {
                    app_error!(
                        "channel",
                        "init",
                        "Failed to run channel DB migration: {}",
                        e
                    );
                }

                // Spawn the inbound message dispatcher
                channel::worker::spawn_dispatcher(registry.clone(), channel_db.clone(), inbound_rx);

                // Auto-start enabled channel accounts
                let channel_registry_clone = registry.clone();
                let store_for_channels = provider::load_store().unwrap_or_default();
                tauri::async_runtime::spawn(async move {
                    for account in store_for_channels.channels.enabled_accounts() {
                        if let Err(e) = channel_registry_clone.start_account(account).await {
                            app_error!(
                                "channel",
                                "init",
                                "Failed to auto-start channel account '{}': {}",
                                account.label,
                                e
                            );
                        }
                    }
                });

                let _ = CHANNEL_REGISTRY.set(registry);
                let _ = CHANNEL_DB.set(channel_db);
            }

            // Initialize ACP control plane
            {
                let store = provider::load_store().unwrap_or_default();
                if store.acp_control.enabled {
                    let registry = Arc::new(acp_control::AcpRuntimeRegistry::new());
                    let registry_clone = Arc::clone(&registry);
                    let acp_config = store.acp_control.clone();
                    // Auto-discover backends in background
                    tokio::spawn(async move {
                        acp_control::registry::auto_discover_and_register(
                            &registry_clone,
                            &acp_config,
                        )
                        .await;
                    });
                    let manager = Arc::new(acp_control::AcpSessionManager::new(registry));
                    let _ = ACP_MANAGER.set(manager);
                }
            }

            AppState {
                agent: Mutex::new(None),
                auth_result: Arc::new(Mutex::new(None)),
                provider_store: Mutex::new(initial_store),
                reasoning_effort: Mutex::new("medium".to_string()),
                codex_token: Mutex::new(None),
                current_agent_id: Mutex::new("default".to_string()),
                session_db,
                chat_cancel: Arc::new(AtomicBool::new(false)),
                log_db,
                logger,
                cron_db,
                subagent_cancels,
                channel_cancels,
            }
        })
        .invoke_handler(tauri::generate_handler![
            // Provider management
            commands::provider::get_providers,
            commands::provider::add_provider,
            commands::provider::update_provider,
            commands::provider::reorder_providers,
            commands::provider::delete_provider,
            commands::provider::test_provider,
            commands::provider::test_model,
            commands::provider::test_embedding,
            commands::provider::test_image_generate,
            commands::provider::get_available_models,
            commands::provider::get_active_model,
            commands::provider::set_active_model,
            commands::provider::get_fallback_models,
            commands::provider::set_fallback_models,
            commands::provider::has_providers,
            // Legacy auth
            commands::auth::initialize_agent,
            commands::auth::start_codex_auth,
            commands::auth::check_auth_status,
            commands::auth::finalize_codex_auth,
            commands::auth::try_restore_session,
            commands::auth::logout_codex,
            // Model & settings (legacy)
            commands::auth::get_codex_models,
            commands::auth::get_current_settings,
            commands::auth::set_codex_model,
            commands::auth::set_reasoning_effort,
            // Chat
            commands::chat::save_attachment,
            commands::chat::chat,
            commands::chat::stop_chat,
            // Command approval
            commands::chat::respond_to_approval,
            // System prompt
            commands::chat::get_system_prompt,
            // Tools info
            commands::chat::list_builtin_tools,
            // Skills
            commands::skills::get_skills,
            commands::skills::get_skill_detail,
            commands::skills::get_extra_skills_dirs,
            commands::skills::add_extra_skills_dir,
            commands::skills::remove_extra_skills_dir,
            commands::skills::toggle_skill,
            commands::skills::get_skill_env_check,
            commands::skills::set_skill_env_check,
            commands::skills::get_skill_env,
            commands::skills::set_skill_env_var,
            commands::skills::remove_skill_env_var,
            commands::skills::get_skills_env_status,
            commands::skills::get_skills_status,
            commands::skills::install_skill_dependency,
            commands::misc::open_directory,
            commands::misc::reveal_in_folder,
            commands::misc::open_url,
            commands::misc::write_export_file,
            // Agent management
            commands::agent_mgmt::list_agents,
            commands::agent_mgmt::get_agent_config,
            commands::agent_mgmt::get_agent_markdown,
            commands::agent_mgmt::save_agent_config_cmd,
            commands::agent_mgmt::save_agent_markdown,
            commands::agent_mgmt::delete_agent,
            commands::agent_mgmt::get_agent_template,
            // Memory management
            commands::memory::memory_add,
            commands::memory::memory_update,
            commands::memory::memory_toggle_pin,
            commands::memory::memory_delete,
            commands::memory::memory_get,
            commands::memory::memory_list,
            commands::memory::memory_search,
            commands::memory::memory_count,
            commands::memory::memory_export,
            commands::memory::memory_find_similar,
            commands::memory::memory_delete_batch,
            commands::memory::memory_import,
            commands::memory::memory_reembed,
            commands::memory::get_global_memory_md,
            commands::memory::save_global_memory_md,
            commands::memory::get_agent_memory_md,
            commands::memory::save_agent_memory_md,
            commands::config::get_web_search_config,
            commands::config::save_web_search_config,
            commands::config::get_web_fetch_config,
            commands::config::save_web_fetch_config,
            commands::config::get_image_generate_config,
            commands::config::save_image_generate_config,
            commands::config::get_proxy_config,
            commands::config::save_proxy_config,
            commands::config::test_proxy,
            commands::docker::searxng_docker_status,
            commands::docker::searxng_docker_deploy,
            commands::docker::searxng_docker_start,
            commands::docker::searxng_docker_stop,
            commands::docker::searxng_docker_remove,
            commands::memory::memory_stats,
            commands::memory::get_extract_config,
            commands::memory::save_extract_config,
            commands::memory::get_dedup_config,
            commands::memory::save_dedup_config,
            commands::memory::get_hybrid_search_config,
            commands::memory::save_hybrid_search_config,
            commands::memory::get_temporal_decay_config,
            commands::memory::save_temporal_decay_config,
            commands::memory::get_mmr_config,
            commands::memory::save_mmr_config,
            commands::memory::get_embedding_cache_config,
            commands::memory::save_embedding_cache_config,
            commands::memory::get_multimodal_config,
            commands::memory::save_multimodal_config,
            commands::memory::get_embedding_config,
            commands::memory::save_embedding_config,
            commands::memory::get_embedding_presets,
            commands::config::get_compact_config,
            commands::config::save_compact_config,
            commands::config::get_notification_config,
            commands::config::save_notification_config,
            commands::config::compact_context_now,
            commands::memory::list_local_embedding_models,
            // Theme & Language
            commands::config::get_theme,
            commands::config::set_theme,
            commands::config::get_language,
            commands::config::set_language,
            commands::config::get_ui_effects_enabled,
            commands::config::set_ui_effects_enabled,
            // User config
            commands::config::get_user_config,
            commands::config::save_user_config,
            commands::config::save_avatar,
            commands::config::get_system_timezone,
            // Tool timeout
            commands::config::get_tool_timeout,
            commands::config::set_tool_timeout,
            // Tool limits (image/pdf)
            commands::config::get_tool_limits,
            commands::config::set_tool_limits,
            // Temperature
            commands::config::get_global_temperature,
            commands::config::set_global_temperature,
            commands::config::get_plan_subagent,
            commands::config::set_plan_subagent,
            // Shortcuts
            commands::config::get_shortcut_config,
            commands::config::save_shortcut_config,
            commands::config::set_shortcuts_paused,
            // Weather
            commands::config::geocode_search,
            commands::config::preview_weather,
            commands::config::get_current_weather,
            commands::config::refresh_weather,
            commands::config::detect_location,
            // Autostart
            commands::config::get_autostart_enabled,
            commands::config::set_autostart_enabled,
            // Permissions
            permissions::check_all_permissions,
            permissions::check_permission,
            permissions::request_permission,
            // Session management
            commands::session::create_session_cmd,
            commands::session::list_sessions_cmd,
            commands::session::load_session_messages_cmd,
            commands::session::load_session_messages_latest_cmd,
            commands::session::load_session_messages_before_cmd,
            commands::session::get_session_cmd,
            commands::session::delete_session_cmd,
            commands::session::rename_session_cmd,
            commands::session::mark_session_read_cmd,
            commands::session::mark_session_read_batch_cmd,
            commands::session::mark_all_sessions_read_cmd,
            // Window theme
            commands::misc::set_window_theme,
            // Logging
            commands::logging::query_logs_cmd,
            commands::logging::get_log_stats_cmd,
            commands::logging::clear_logs_cmd,
            commands::logging::get_log_config_cmd,
            commands::logging::save_log_config_cmd,
            commands::logging::export_logs_cmd,
            commands::logging::list_log_files_cmd,
            commands::logging::read_log_file_cmd,
            commands::logging::get_log_file_path_cmd,
            commands::logging::frontend_log,
            commands::logging::frontend_log_batch,
            // Cron management
            commands::cron::cron_list_jobs,
            commands::cron::cron_get_job,
            commands::cron::cron_create_job,
            commands::cron::cron_update_job,
            commands::cron::cron_delete_job,
            commands::cron::cron_toggle_job,
            commands::cron::cron_run_now,
            commands::cron::cron_get_run_logs,
            commands::cron::cron_get_calendar_events,
            // Sub-agent management
            commands::subagent::list_subagent_runs,
            commands::subagent::get_subagent_run,
            commands::subagent::kill_subagent,
            // Crash recovery & backup
            commands::crash::get_crash_recovery_info,
            commands::crash::get_crash_history,
            commands::crash::clear_crash_history,
            commands::crash::request_app_restart,
            commands::crash::list_backups_cmd,
            commands::crash::restore_backup_cmd,
            commands::crash::create_backup_cmd,
            commands::crash::get_guardian_enabled,
            commands::crash::set_guardian_enabled,
            // Sandbox
            sandbox::get_sandbox_config,
            sandbox::set_sandbox_config,
            sandbox::check_sandbox_available,
            // Slash commands
            slash_commands::list_slash_commands,
            slash_commands::execute_slash_command,
            slash_commands::is_slash_command,
            // Canvas
            tools::canvas::canvas_submit_snapshot,
            tools::canvas::canvas_submit_eval_result,
            tools::canvas::get_canvas_config,
            tools::canvas::save_canvas_config,
            tools::canvas::list_canvas_projects,
            tools::canvas::get_canvas_project,
            tools::canvas::delete_canvas_project,
            tools::canvas::show_canvas_panel,
            // Dashboard analytics
            commands::dashboard::dashboard_overview,
            commands::dashboard::dashboard_token_usage,
            commands::dashboard::dashboard_tool_usage,
            commands::dashboard::dashboard_sessions,
            commands::dashboard::dashboard_errors,
            commands::dashboard::dashboard_tasks,
            commands::dashboard::dashboard_system_metrics,
            commands::dashboard::dashboard_session_list,
            commands::dashboard::dashboard_message_list,
            commands::dashboard::dashboard_tool_call_list,
            commands::dashboard::dashboard_error_list,
            commands::dashboard::dashboard_agent_list,
            // Developer tools
            dev_tools::dev_clear_sessions,
            dev_tools::dev_clear_cron,
            dev_tools::dev_clear_memory,
            dev_tools::dev_reset_config,
            dev_tools::dev_clear_all,
            // Plan mode
            commands::plan::get_plan_mode,
            commands::plan::set_plan_mode,
            commands::plan::get_plan_content,
            commands::plan::save_plan_content,
            commands::plan::get_plan_steps,
            commands::plan::update_plan_step_status,
            commands::plan::respond_plan_question,
            commands::plan::get_plan_versions,
            commands::plan::load_plan_version_content,
            commands::plan::restore_plan_version,
            commands::plan::plan_rollback,
            commands::plan::get_plan_checkpoint,
            commands::plan::get_plan_file_path,
            commands::plan::cancel_plan_subagent,
            // ACP control plane
            commands::acp_control::acp_list_backends,
            commands::acp_control::acp_health_check,
            commands::acp_control::acp_refresh_backends,
            commands::acp_control::acp_list_runs,
            commands::acp_control::acp_kill_run,
            commands::acp_control::acp_get_run_result,
            commands::acp_control::acp_get_config,
            commands::acp_control::acp_set_config,
            // URL preview
            commands::url_preview::fetch_url_preview,
            commands::url_preview::fetch_url_previews,
            // IM Channel management
            commands::channel::channel_list_plugins,
            commands::channel::channel_list_accounts,
            commands::channel::channel_add_account,
            commands::channel::channel_update_account,
            commands::channel::channel_remove_account,
            commands::channel::channel_start_account,
            commands::channel::channel_stop_account,
            commands::channel::channel_health,
            commands::channel::channel_health_all,
            commands::channel::channel_validate_credentials,
            commands::channel::channel_send_test_message,
            commands::channel::channel_list_sessions,
            commands::channel::channel_wechat_start_login,
            commands::channel::channel_wechat_wait_login,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // macOS: clicking Dock icon when all windows are hidden → show main window
            if let tauri::RunEvent::Reopen { .. } = event {
                use tauri::Manager;
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        });
}
