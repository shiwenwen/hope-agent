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

/// Clear any pending chord state (used when shortcuts are paused or reconfigured).
pub(crate) fn clear_chord_state() {
    *chord_state().lock().unwrap_or_else(|e| e.into_inner()) = None;
}

// ── Shortcut handler ───────────────────────────────────────────

/// Global shortcut handler — dispatches single-combo and chord shortcuts.
pub(crate) fn handle_shortcut(
    app_handle: &tauri::AppHandle,
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
    event: tauri_plugin_global_shortcut::ShortcutEvent,
) {
    if event.state != tauri_plugin_global_shortcut::ShortcutState::Pressed {
        return;
    }
    use tauri::Emitter;
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let shortcut_str = shortcut.to_string();
    let store = oc_core::config::cached_config();

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
                        if let Ok(sc) = s.parse::<tauri_plugin_global_shortcut::Shortcut>() {
                            let _ = manager.unregister(sc);
                        }
                    }
                    *state = None;
                    drop(state);

                    // Execute the chord action
                    execute_shortcut_action(app_handle, &action_id_clone, &shortcut_str);
                    return;
                }
            }
            // Pending expired or no match — clean up temporary registrations
            let manager = app_handle.global_shortcut();
            for (_, s) in &pending.completions {
                if let Ok(sc) = s.parse::<tauri_plugin_global_shortcut::Shortcut>() {
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
                if let Ok(first) = parts[0].parse::<tauri_plugin_global_shortcut::Shortcut>() {
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
            if let Ok(sc) = second_str.parse::<tauri_plugin_global_shortcut::Shortcut>() {
                let _ = manager.register(sc);
            }
        }
        // Set pending state
        let deadline =
            std::time::Instant::now() + std::time::Duration::from_millis(CHORD_TIMEOUT_MS);
        *chord_state().lock().unwrap_or_else(|e| e.into_inner()) = Some(ChordPending {
            completions: chord_matches.clone(),
            deadline,
        });
        // Emit visual feedback to frontend
        let _ = app_handle.emit("chord-first-pressed", shortcut_str.clone());
        // Spawn timeout cleanup thread
        let app_clone = app_handle.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(CHORD_TIMEOUT_MS + 50));
            let mut state = chord_state().lock().unwrap_or_else(|e| e.into_inner());
            if let Some(pending) = state.take() {
                let manager = app_clone.global_shortcut();
                for (_, s) in &pending.completions {
                    if let Ok(sc) = s.parse::<tauri_plugin_global_shortcut::Shortcut>() {
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
}

// ── Action dispatch ────────────────────────────────────────────

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

                    // Floating window level — above normal windows while allowing IME candidate popups
                    ns_window.setLevel(
                        objc2_app_kit::NSWindowLevel::from(3_isize), // NSFloatingWindowLevel
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
