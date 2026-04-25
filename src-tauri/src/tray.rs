use crate::menu_labels::{resolve_language, tray_labels};
use ha_core::{app_debug, app_info};
use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager};

/// Show and focus the main window if it already exists.
fn show_main_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Set up the system tray icon with context menu.
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let lang = resolve_language();
    let labels = tray_labels(&lang);

    // Build menu items
    let show_main = MenuItemBuilder::with_id("show_main", labels.show_main).build(app)?;
    let quick_chat = MenuItemBuilder::with_id("quick_chat", labels.quick_chat).build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let new_session = MenuItemBuilder::with_id("new_session", labels.new_session).build(app)?;
    let open_settings = MenuItemBuilder::with_id("open_settings", labels.settings).build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_app = MenuItemBuilder::with_id("quit_app", labels.quit).build(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &show_main,
            &quick_chat,
            &sep1,
            &new_session,
            &open_settings,
            &sep2,
            &quit_app,
        ])
        .build()?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/menu.png")).unwrap();
    let icon_as_template = true;
    let show_menu_on_left_click = false;

    let tray = TrayIconBuilder::new()
        .tooltip("Hope Agent")
        .icon(icon)
        .icon_as_template(icon_as_template)
        .show_menu_on_left_click(show_menu_on_left_click)
        .menu(&menu)
        .on_menu_event(|app_handle, event| {
            app_debug!(
                "tray",
                "menu",
                "Tray menu item clicked: {}",
                event.id().as_ref()
            );
            match event.id().as_ref() {
                "show_main" => {
                    show_main_window(app_handle);
                }
                "quick_chat" => {
                    crate::toggle_quickchat_window(app_handle);
                }
                "new_session" => {
                    show_main_window(app_handle);
                    let _ = app_handle.emit("new-session", ());
                }
                "open_settings" => {
                    show_main_window(app_handle);
                    let _ = app_handle.emit("open-settings", ());
                }
                "quit_app" => {
                    app_handle.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                app_debug!(
                    "tray",
                    "icon",
                    "Tray icon click: button={:?}, state={:?}",
                    button,
                    button_state
                );

                // Left click on tray icon → show main window.
                if button == MouseButton::Left && button_state == MouseButtonState::Up {
                    show_main_window(tray.app_handle());
                }
            }
        })
        .build(app)?;

    app_info!(
        "tray",
        "setup",
        "Tray initialized: id={}, show_menu_on_left_click={}, icon_as_template={}",
        tray.id().as_ref(),
        show_menu_on_left_click,
        icon_as_template
    );

    {
        let tray_handle = tray.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let tooltip = build_tray_tooltip();
                let _ = tray_handle.set_tooltip(Some(&tooltip));
            }
        });
    }

    Ok(())
}

fn build_tray_tooltip() -> String {
    let snap = ha_core::server_status::snapshot();
    if let Some(err) = &snap.startup_error {
        let first_line = err.lines().next().unwrap_or(err);
        let truncated = ha_core::truncate_utf8(first_line, 80);
        return format!("Hope Agent\nServer failed: {}", truncated);
    }
    let addr = snap
        .bound_addr
        .as_deref()
        .unwrap_or("starting...")
        .to_string();
    let ws_total = snap.events_ws_count + snap.chat_ws_count;
    let uptime = snap
        .uptime_secs
        .map(format_short_uptime)
        .unwrap_or_else(|| "-".to_string());
    format!("Hope Agent\n{} · {} WS · up {}", addr, ws_total, uptime)
}

fn format_short_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}h {}m", h, m)
    } else if m > 0 {
        format!("{}m {}s", m, s)
    } else {
        format!("{}s", s)
    }
}
