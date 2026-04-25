use crate::menu_labels::{resolve_language, tray_labels, tray_status_labels, TrayStatusLabels};
use ha_core::{app_debug, app_info};
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Emitter, Manager, Runtime};

const TRAY_STATUS_LINE_COUNT: usize = 5;

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
    let status_labels = tray_status_labels(&lang);
    let status_lines = current_tray_status_lines(&status_labels);

    // Build menu items
    let status_header = MenuItemBuilder::with_id("tray_status_header", &status_lines[0])
        .enabled(false)
        .build(app)?;
    let status_bound_addr = MenuItemBuilder::with_id("tray_status_bound_addr", &status_lines[1])
        .enabled(false)
        .build(app)?;
    let status_uptime = MenuItemBuilder::with_id("tray_status_uptime", &status_lines[2])
        .enabled(false)
        .build(app)?;
    let status_connections = MenuItemBuilder::with_id("tray_status_connections", &status_lines[3])
        .enabled(false)
        .build(app)?;
    let status_sessions = MenuItemBuilder::with_id("tray_status_sessions", &status_lines[4])
        .enabled(false)
        .build(app)?;
    let sep_status = PredefinedMenuItem::separator(app)?;
    let show_main = MenuItemBuilder::with_id("show_main", labels.show_main).build(app)?;
    let quick_chat = MenuItemBuilder::with_id("quick_chat", labels.quick_chat).build(app)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let new_session = MenuItemBuilder::with_id("new_session", labels.new_session).build(app)?;
    let open_settings = MenuItemBuilder::with_id("open_settings", labels.settings).build(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    let quit_app = MenuItemBuilder::with_id("quit_app", labels.quit).build(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[
            &status_header,
            &status_bound_addr,
            &status_uptime,
            &status_connections,
            &status_sessions,
            &sep_status,
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
    let initial_tooltip = build_tray_tooltip(&status_lines);

    let tray = TrayIconBuilder::new()
        .tooltip(&initial_tooltip)
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
        let status_items = TrayStatusMenuItems {
            header: status_header,
            bound_addr: status_bound_addr,
            uptime: status_uptime,
            connections: status_connections,
            sessions: status_sessions,
        };
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let lines = current_tray_status_lines(&status_labels);
                update_tray_status_menu(&status_items, &lines);
                let tooltip = build_tray_tooltip(&lines);
                let _ = tray_handle.set_tooltip(Some(&tooltip));
            }
        });
    }

    Ok(())
}

#[derive(Clone, Copy)]
struct TrayRuntimeStatus<'a> {
    bound_addr: Option<&'a str>,
    uptime_secs: Option<u64>,
    startup_error: Option<&'a str>,
    events_ws_count: u32,
    chat_ws_count: u32,
    local_desktop_client: bool,
    active_chat_total: u32,
}

struct TrayStatusMenuItems<R: Runtime> {
    header: MenuItem<R>,
    bound_addr: MenuItem<R>,
    uptime: MenuItem<R>,
    connections: MenuItem<R>,
    sessions: MenuItem<R>,
}

fn current_tray_status_lines(labels: &TrayStatusLabels) -> Vec<String> {
    let snap = ha_core::server_status::snapshot();
    let counts = ha_core::chat_engine::stream_seq::active_counts();
    format_tray_status_lines(
        labels,
        TrayRuntimeStatus {
            bound_addr: snap.bound_addr.as_deref(),
            uptime_secs: snap.uptime_secs,
            startup_error: snap.startup_error.as_deref(),
            events_ws_count: snap.events_ws_count,
            chat_ws_count: snap.chat_ws_count,
            local_desktop_client: true,
            active_chat_total: counts.total,
        },
    )
}

fn update_tray_status_menu<R: Runtime>(items: &TrayStatusMenuItems<R>, lines: &[String]) {
    if lines.len() != TRAY_STATUS_LINE_COUNT {
        app_debug!(
            "tray",
            "status",
            "Skipping tray status update with unexpected line count: {}",
            lines.len()
        );
        return;
    }
    let _ = items.header.set_text(&lines[0]);
    let _ = items.bound_addr.set_text(&lines[1]);
    let _ = items.uptime.set_text(&lines[2]);
    let _ = items.connections.set_text(&lines[3]);
    let _ = items.sessions.set_text(&lines[4]);
}

fn build_tray_tooltip(lines: &[String]) -> String {
    format!("Hope Agent\n{}", lines.join("\n"))
}

fn format_tray_status_lines(
    labels: &TrayStatusLabels,
    status: TrayRuntimeStatus<'_>,
) -> Vec<String> {
    let bound_addr = match status.startup_error {
        Some(err) => {
            let first_line = err.lines().next().unwrap_or(err);
            let truncated = ha_core::truncate_utf8(first_line, 80);
            format!("{}: {}", labels.startup_error, truncated)
        }
        None => format!(
            "{}: {}",
            labels.bound_addr,
            status.bound_addr.unwrap_or(labels.not_started)
        ),
    };
    let uptime = status
        .uptime_secs
        .map(format_short_uptime)
        .unwrap_or_else(|| "-".to_string());
    let local_count = u32::from(status.local_desktop_client);
    let connection_total = status.events_ws_count + status.chat_ws_count + local_count;
    let mut connection_parts = vec![
        format!("{} {}", status.events_ws_count, labels.event_unit),
        format!("{} {}", status.chat_ws_count, labels.chat_unit),
    ];
    if status.local_desktop_client {
        connection_parts.push(format!("{} {}", local_count, labels.local_unit));
    }

    vec![
        labels.runtime_status.to_string(),
        bound_addr,
        format!("{}: {}", labels.uptime, uptime),
        format!(
            "{}: {} ({})",
            labels.active_connections,
            connection_total,
            connection_parts.join(" · ")
        ),
        format!("{}: {}", labels.active_sessions, status.active_chat_total),
    ]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu_labels::tray_status_labels;

    #[test]
    fn status_menu_lines_match_simplified_chinese_sidebar_wording() {
        let labels = tray_status_labels("zh");

        let lines = format_tray_status_lines(
            &labels,
            TrayRuntimeStatus {
                bound_addr: Some("127.0.0.1:8420"),
                uptime_secs: Some(687),
                startup_error: None,
                events_ws_count: 0,
                chat_ws_count: 0,
                local_desktop_client: true,
                active_chat_total: 0,
            },
        );

        assert_eq!(
            lines,
            vec![
                "运行时状态".to_string(),
                "绑定地址: 127.0.0.1:8420".to_string(),
                "运行时长: 11m 27s".to_string(),
                "活跃连接: 1 (0 事件 · 0 会话 · 1 本机)".to_string(),
                "活跃会话: 0".to_string(),
            ]
        );
    }
}
