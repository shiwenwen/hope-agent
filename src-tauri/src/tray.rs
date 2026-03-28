use tauri::menu::{MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent, MouseButton, MouseButtonState};
use tauri::{Manager, Emitter};

/// Show and focus the main window (creates it if needed).
fn show_main_window(app_handle: &tauri::AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Set up the system tray icon with context menu.
pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    // Build menu items
    let show_main = MenuItemBuilder::with_id("show_main", "Show Main Window")
        .build(app)?;
    let quick_chat = MenuItemBuilder::with_id("quick_chat", "Quick Chat")
        .build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let new_session = MenuItemBuilder::with_id("new_session", "New Session")
        .build(app)?;
    let open_settings = MenuItemBuilder::with_id("open_settings", "Settings")
        .build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_app = MenuItemBuilder::with_id("quit_app", "Quit OpenComputer")
        .build(app)?;

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

    let _tray = TrayIconBuilder::new()
        .tooltip("OpenComputer")
        .menu(&menu)
        .on_menu_event(|app_handle, event| {
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
            // Left click on tray icon → show main window
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
