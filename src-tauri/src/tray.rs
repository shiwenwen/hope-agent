use crate::menu_labels::{
    resolve_language, tray_labels, tray_status_labels, TrayLabels, TrayStatusLabels,
};
use ha_core::session::{ProjectFilter, SessionMeta};
use ha_core::{app_debug, app_info, app_warn};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const TRAY_STATUS_LINE_COUNT: usize = 5;
const TRAY_SESSION_PREFIX: &str = "tray_session:";
const TRAY_SESSION_MORE_ID: &str = "tray_session_more";
/// Cap on dynamic active-session entries shown directly in the tray menu.
/// Beyond this we render a single disabled "… {N} more" line so the menu
/// stays compact even with many concurrent streams.
const TRAY_ACTIVE_SESSIONS_CAP: usize = 5;
/// Page size used when sweeping recent sessions for pending-interaction
/// candidates. Bigger than CAP so the diff/sort step has room to work.
const TRAY_RECENT_SCAN_LIMIT: u32 = 50;
/// UTF-8 byte budget for the title shown inside a tray menu item. macOS and
/// Windows both render long menu items poorly; clamp to keep the menu narrow.
const TRAY_TITLE_BYTES: usize = 60;

/// Show and focus the main window if it already exists.
fn show_main_window(app_handle: &AppHandle) {
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

    let app_handle: AppHandle = app.handle().clone();
    let initial_menu =
        build_tray_menu(&app_handle, &labels, &status_labels, &status_lines, &[], 0)?;

    let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/menu.png")).unwrap();
    let icon_as_template = true;
    let show_menu_on_left_click = false;
    let initial_tooltip = build_tray_tooltip(&status_lines);

    let tray = TrayIconBuilder::new()
        .tooltip(&initial_tooltip)
        .icon(icon)
        .icon_as_template(icon_as_template)
        .show_menu_on_left_click(show_menu_on_left_click)
        .menu(&initial_menu)
        .on_menu_event(|app_handle, event| {
            let id = event.id().as_ref();
            app_debug!("tray", "menu", "Tray menu item clicked: {}", id);

            if let Some(session_id) = id.strip_prefix(TRAY_SESSION_PREFIX) {
                let session_id = session_id.to_string();
                app_info!(
                    "tray",
                    "focus_session",
                    "user clicked tray active-session entry session_id={}",
                    session_id
                );
                show_main_window(app_handle);
                if let Err(e) =
                    app_handle.emit("tray:focus-session", json!({ "sessionId": session_id }))
                {
                    app_warn!(
                        "tray",
                        "focus_session",
                        "failed to emit tray:focus-session: {:#}",
                        e
                    );
                }
                return;
            }

            match id {
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
                    let labels = tray_labels(&resolve_language());
                    let app = app_handle.clone();
                    app_handle
                        .dialog()
                        .message(labels.quit_confirm_body)
                        .title(labels.quit_confirm_title)
                        .kind(MessageDialogKind::Warning)
                        .buttons(MessageDialogButtons::OkCancelCustom(
                            labels.quit_confirm_ok.to_string(),
                            labels.quit_confirm_cancel.to_string(),
                        ))
                        .show(move |confirmed| {
                            if confirmed {
                                app_info!("tray", "quit_app", "user confirmed quit");
                                app.exit(0);
                            } else {
                                app_debug!("tray", "quit_app", "user cancelled quit");
                            }
                        });
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
        let loop_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            let mut last_signature: Option<TraySnapshotSig> = None;
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let lines = current_tray_status_lines(&status_labels);
                let (active, more) = compute_active_regular_sessions(&status_labels).await;
                let signature = TraySnapshotSig::from(&lines, &active, more);

                let menu_changed = last_signature.as_ref() != Some(&signature);
                if menu_changed {
                    match build_tray_menu(
                        &loop_handle,
                        &labels,
                        &status_labels,
                        &lines,
                        &active,
                        more,
                    ) {
                        Ok(menu) => {
                            if let Err(e) = tray_handle.set_menu(Some(menu)) {
                                app_warn!("tray", "refresh", "failed to swap tray menu: {:#}", e);
                            } else {
                                last_signature = Some(signature);
                            }
                        }
                        Err(e) => {
                            app_warn!("tray", "refresh", "failed to rebuild tray menu: {:#}", e);
                        }
                    }
                }

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

#[derive(Clone, Debug, PartialEq, Eq)]
struct ActiveRegularSession {
    session_id: String,
    label: String,
}

/// Tuple used to detect "did the dynamic menu actually change?" so we can
/// skip `tray.set_menu` when nothing moved (avoids menu flicker on macOS
/// when the user is mid-interaction).
#[derive(Debug, PartialEq, Eq)]
struct TraySnapshotSig {
    status_lines: Vec<String>,
    active: Vec<(String, String)>,
    more: usize,
}

impl TraySnapshotSig {
    fn from(lines: &[String], active: &[ActiveRegularSession], more: usize) -> Self {
        Self {
            status_lines: lines.to_vec(),
            active: active
                .iter()
                .map(|s| (s.session_id.clone(), s.label.clone()))
                .collect(),
            more,
        }
    }
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

/// Format a session entry as a tray menu label, including a status glyph
/// (`▶` streaming / `⏸` waiting on user / `▶⏸` both) and the title with
/// UTF-8-safe truncation. Falls back to `untitled_session` from the
/// `TrayStatusLabels` when the session has no title yet.
fn format_active_session_label(
    s: &SessionMeta,
    streaming: bool,
    pending: bool,
    labels: &TrayStatusLabels,
) -> String {
    let glyph = match (streaming, pending) {
        (true, true) => "▶⏸",
        (true, false) => "▶",
        (false, true) => "⏸",
        // Should not occur — caller filters out sessions that match neither.
        (false, false) => "•",
    };
    let raw_title = s
        .title
        .as_deref()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .unwrap_or(labels.untitled_session);
    let title = ha_core::truncate_utf8(raw_title, TRAY_TITLE_BYTES);
    format!("  {glyph} {title}")
}

/// Resolve the list of "in-progress regular conversations" for the tray
/// dropdown. Combines two condition sources:
///
/// 1. **Streaming**: session ids registered in `chat_engine::stream_seq`
///    with `source=Desktop` (LLM is currently producing tokens).
/// 2. **Pending interaction**: `SessionMeta.pending_interaction_count > 0`
///    (waiting on a tool approval or an `ask_user_question` answer).
///
/// Then filters to regular sessions only (see [`SessionMeta::is_regular_chat`])
/// and returns up to [`TRAY_ACTIVE_SESSIONS_CAP`] entries plus the count of
/// items truncated. Order: by `updated_at DESC` to mirror the sidebar.
async fn compute_active_regular_sessions(
    status_labels: &TrayStatusLabels,
) -> (Vec<ActiveRegularSession>, usize) {
    let streaming_ids: std::collections::HashSet<String> =
        ha_core::chat_engine::stream_seq::active_session_ids_by_source(
            ha_core::chat_engine::stream_seq::ChatSource::Desktop,
        )
        .into_iter()
        .collect();

    let Some(db) = ha_core::get_session_db() else {
        // Bootstrap window: SessionDB not yet initialized. Skip silently;
        // next 5s tick will retry.
        return (Vec::new(), 0);
    };
    let db: Arc<ha_core::session::SessionDB> = db.clone();

    // Pull recent sessions ordered by `updated_at DESC`. This is the primary
    // candidate pool for "pending_interaction_count > 0" sessions and also
    // covers the streaming case for any actively-touched session.
    let recent = match db.list_sessions_paged(
        None,
        ProjectFilter::All,
        Some(TRAY_RECENT_SCAN_LIMIT),
        Some(0),
        None,
    ) {
        Ok((rows, _total)) => rows,
        Err(e) => {
            app_warn!(
                "tray",
                "active_sessions",
                "list_sessions_paged failed: {:#}",
                e
            );
            return (Vec::new(), 0);
        }
    };

    let mut by_id: HashMap<String, SessionMeta> =
        recent.into_iter().map(|s| (s.id.clone(), s)).collect();

    // Pull any streaming session not in the recent slice (edge case: very
    // old session resumed after long idle — its `updated_at` may not yet
    // reflect the new turn at sweep time).
    for sid in &streaming_ids {
        if !by_id.contains_key(sid) {
            match db.get_session(sid) {
                Ok(Some(meta)) => {
                    by_id.insert(sid.clone(), meta);
                }
                Ok(None) => {}
                Err(e) => {
                    app_warn!(
                        "tray",
                        "active_sessions",
                        "get_session({sid}) failed: {:#}",
                        e
                    );
                }
            }
        }
    }

    // Enrich pending_interaction_count for the merged set.
    let mut sessions: Vec<SessionMeta> = by_id.into_values().collect();
    if let Err(e) = ha_core::session::enrich_pending_interactions(&mut sessions, &db).await {
        app_warn!(
            "tray",
            "active_sessions",
            "enrich_pending_interactions failed: {:#}",
            e
        );
    }

    // Keep only regular sessions that are streaming OR have a pending
    // interaction. Both filters are cheap; do them in one pass.
    let mut filtered: Vec<(SessionMeta, bool, bool)> = sessions
        .into_iter()
        .filter_map(|s| {
            if !s.is_regular_chat() {
                return None;
            }
            let streaming = streaming_ids.contains(&s.id);
            let pending = s.pending_interaction_count > 0;
            (streaming || pending).then_some((s, streaming, pending))
        })
        .collect();

    // Sort newest-first to mirror the sidebar.
    filtered.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));

    let total = filtered.len();
    let truncated = total.saturating_sub(TRAY_ACTIVE_SESSIONS_CAP);
    let kept: Vec<ActiveRegularSession> = filtered
        .into_iter()
        .take(TRAY_ACTIVE_SESSIONS_CAP)
        .map(|(s, streaming, pending)| ActiveRegularSession {
            label: format_active_session_label(&s, streaming, pending, status_labels),
            session_id: s.id,
        })
        .collect();

    (kept, truncated)
}

/// Build the full tray menu including the dynamic "in-progress regular
/// conversations" rows. Items appear in this order:
///
/// 1. 5 disabled status rows (unchanged from before).
/// 2. Per active session: one clickable item with id `tray_session:{id}`.
/// 3. Optional disabled "… {N} more" line if `more > 0`.
/// 4. Separator + the existing action items.
fn build_tray_menu<R: Runtime>(
    manager: &impl Manager<R>,
    labels: &TrayLabels,
    status_labels: &TrayStatusLabels,
    status_lines: &[String],
    active: &[ActiveRegularSession],
    more: usize,
) -> tauri::Result<Menu<R>> {
    if status_lines.len() != TRAY_STATUS_LINE_COUNT {
        return Err(tauri::Error::Anyhow(anyhow::anyhow!(
            "tray status lines expected {} entries, got {}",
            TRAY_STATUS_LINE_COUNT,
            status_lines.len()
        )));
    }

    let status_header = MenuItemBuilder::with_id("tray_status_header", &status_lines[0])
        .enabled(false)
        .build(manager)?;
    let status_bound_addr = MenuItemBuilder::with_id("tray_status_bound_addr", &status_lines[1])
        .enabled(false)
        .build(manager)?;
    let status_uptime = MenuItemBuilder::with_id("tray_status_uptime", &status_lines[2])
        .enabled(false)
        .build(manager)?;
    let status_connections = MenuItemBuilder::with_id("tray_status_connections", &status_lines[3])
        .enabled(false)
        .build(manager)?;
    let status_sessions = MenuItemBuilder::with_id("tray_status_sessions", &status_lines[4])
        .enabled(false)
        .build(manager)?;

    let session_items: Vec<_> = active
        .iter()
        .map(|s| {
            MenuItemBuilder::with_id(format!("{TRAY_SESSION_PREFIX}{}", s.session_id), &s.label)
                .build(manager)
        })
        .collect::<tauri::Result<Vec<_>>>()?;

    let more_item = if more > 0 {
        Some(
            MenuItemBuilder::with_id(
                TRAY_SESSION_MORE_ID,
                status_labels.more_sessions.replace("{}", &more.to_string()),
            )
            .enabled(false)
            .build(manager)?,
        )
    } else {
        None
    };

    let sep_status = PredefinedMenuItem::separator(manager)?;
    let show_main = MenuItemBuilder::with_id("show_main", labels.show_main).build(manager)?;
    let quick_chat = MenuItemBuilder::with_id("quick_chat", labels.quick_chat).build(manager)?;
    let sep1 = PredefinedMenuItem::separator(manager)?;
    let new_session = MenuItemBuilder::with_id("new_session", labels.new_session).build(manager)?;
    let open_settings =
        MenuItemBuilder::with_id("open_settings", labels.settings).build(manager)?;
    let sep2 = PredefinedMenuItem::separator(manager)?;
    let quit_app = MenuItemBuilder::with_id("quit_app", labels.quit).build(manager)?;

    let mut builder = MenuBuilder::new(manager)
        .item(&status_header)
        .item(&status_bound_addr)
        .item(&status_uptime)
        .item(&status_connections)
        .item(&status_sessions);
    for it in &session_items {
        builder = builder.item(it);
    }
    if let Some(ref m) = more_item {
        builder = builder.item(m);
    }
    builder = builder
        .item(&sep_status)
        .item(&show_main)
        .item(&quick_chat)
        .item(&sep1)
        .item(&new_session)
        .item(&open_settings)
        .item(&sep2)
        .item(&quit_app);
    builder.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu_labels::tray_status_labels;

    fn dummy_session(id: &str, title: Option<&str>) -> SessionMeta {
        SessionMeta {
            id: id.to_string(),
            title: title.map(|t| t.to_string()),
            title_source: "manual".to_string(),
            agent_id: "default".to_string(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            created_at: "2026-05-01T00:00:00Z".to_string(),
            updated_at: "2026-05-01T00:00:00Z".to_string(),
            message_count: 0,
            unread_count: 0,
            has_error: false,
            pending_interaction_count: 0,
            is_cron: false,
            parent_session_id: None,
            plan_mode: Default::default(),
            permission_mode: Default::default(),
            project_id: None,
            channel_info: None,
            incognito: false,
            working_dir: None,
        }
    }

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

    #[test]
    fn format_active_session_label_picks_glyph_by_state() {
        let labels = tray_status_labels("en");
        let s = dummy_session("a", Some("My session"));

        assert_eq!(
            format_active_session_label(&s, true, false, &labels),
            "  ▶ My session"
        );
        assert_eq!(
            format_active_session_label(&s, false, true, &labels),
            "  ⏸ My session"
        );
        assert_eq!(
            format_active_session_label(&s, true, true, &labels),
            "  ▶⏸ My session"
        );

        // Empty title falls back to localized placeholder.
        let untitled = dummy_session("b", None);
        assert_eq!(
            format_active_session_label(&untitled, true, false, &labels),
            "  ▶ Untitled"
        );
        // Whitespace-only title is treated as empty.
        let blank = dummy_session("c", Some("   "));
        assert_eq!(
            format_active_session_label(&blank, false, true, &labels),
            "  ⏸ Untitled"
        );
    }

    #[test]
    fn more_label_substitutes_count_for_each_locale() {
        for lang in [
            "zh", "zh-TW", "ja", "ko", "es", "pt", "ru", "ar", "tr", "vi", "ms", "en",
        ] {
            let labels = tray_status_labels(lang);
            let rendered = labels.more_sessions.replace("{}", "3");
            assert!(
                rendered.contains('3'),
                "lang {lang} more_sessions did not substitute count: {rendered}"
            );
            assert!(
                !rendered.contains("{}"),
                "lang {lang} more_sessions left unreplaced placeholder: {rendered}"
            );
        }
    }
}
