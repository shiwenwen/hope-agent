use crate::menu_labels::{
    resolve_language, tray_labels, tray_status_labels, TrayLabels, TrayStatusLabels,
};
use ha_core::session::{ProjectFilter, SessionMeta};
use ha_core::{app_debug, app_info, app_warn};
use serde_json::json;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

const TRAY_STATUS_LINE_COUNT: usize = 5;
const TRAY_STATUS_UPTIME_LINE_INDEX: usize = 2;
const TRAY_SESSION_PREFIX: &str = "tray_session:";
const TRAY_SESSION_DETAIL_PREFIX: &str = "tray_session_detail:";
const TRAY_SESSION_MORE_ID: &str = "tray_session_more";
/// Cap on dynamic regular-session entries shown directly in the tray menu.
/// Beyond this we render a single disabled "… {N} more" line so the menu
/// stays compact even with many concurrent streams.
const TRAY_ACTIVE_SESSIONS_CAP: usize = 5;
/// Page size used when sweeping recent sessions for pending-interaction
/// candidates. Bigger than CAP so the diff/sort step has room to work.
const TRAY_RECENT_SCAN_LIMIT: u32 = 50;
/// UTF-8 byte budget for the title shown inside a tray menu item. macOS and
/// Windows both render long menu items poorly; clamp to keep the menu narrow.
const TRAY_TITLE_BYTES: usize = 72;
const TRAY_AGENT_BYTES: usize = 28;
const TRAY_MODEL_BYTES: usize = 36;
/// Native tray menus close on macOS when their backing menu is replaced. After
/// the user opens the tray menu, defer structural menu refreshes briefly so
/// session metadata changes do not close the menu while it is being read.
const TRAY_MENU_REFRESH_HOLD_MS: u64 = 5 * 60_000;

/// Show and focus the main window if it already exists.
fn show_main_window(app_handle: &AppHandle) {
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or(0)
}

fn hold_tray_menu_refresh(hold_until_ms: &AtomicU64) {
    hold_until_ms.store(
        now_millis().saturating_add(TRAY_MENU_REFRESH_HOLD_MS),
        Ordering::Relaxed,
    );
}

fn is_tray_menu_refresh_held(hold_until_ms: &AtomicU64) -> bool {
    hold_until_ms.load(Ordering::Relaxed) > now_millis()
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
    let show_menu_on_left_click = true;
    let initial_tooltip = build_tray_tooltip(&status_lines);
    let initial_signature = TraySnapshotSig::from(&status_lines, &[], 0);
    let menu_refresh_hold_until_ms = Arc::new(AtomicU64::new(0));

    let menu_event_refresh_hold = Arc::clone(&menu_refresh_hold_until_ms);
    let tray_click_refresh_hold = Arc::clone(&menu_refresh_hold_until_ms);
    let tray = TrayIconBuilder::new()
        .tooltip(&initial_tooltip)
        .icon(icon)
        .icon_as_template(icon_as_template)
        .show_menu_on_left_click(show_menu_on_left_click)
        .menu(&initial_menu)
        .on_menu_event(move |app_handle, event| {
            // A menu item can only be clicked after the native menu closed, so
            // allow the next poll to apply any pending structure refresh.
            menu_event_refresh_hold.store(0, Ordering::Relaxed);
            let id = event.id().as_ref();
            app_debug!("tray", "menu", "Tray menu item clicked: {}", id);

            if let Some(session_id) = id.strip_prefix(TRAY_SESSION_PREFIX) {
                let session_id = session_id.to_string();
                app_info!(
                    "tray",
                    "focus_session",
                    "user clicked tray session entry session_id={}",
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
        .on_tray_icon_event(move |_tray, event| {
            if let TrayIconEvent::Click {
                button,
                button_state,
                ..
            } = event
            {
                hold_tray_menu_refresh(&tray_click_refresh_hold);
                app_debug!(
                    "tray",
                    "icon",
                    "Tray icon click: button={:?}, state={:?}",
                    button,
                    button_state
                );
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
        let loop_refresh_hold = Arc::clone(&menu_refresh_hold_until_ms);
        tauri::async_runtime::spawn(async move {
            let mut last_signature: Option<TraySnapshotSig> = Some(initial_signature);
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let lines = current_tray_status_lines(&status_labels);
                let (entries, more) = compute_tray_regular_sessions(&status_labels).await;
                let signature = TraySnapshotSig::from(&lines, &entries, more);

                let menu_changed = last_signature.as_ref() != Some(&signature);
                if menu_changed && !is_tray_menu_refresh_held(&loop_refresh_hold) {
                    match build_tray_menu(
                        &loop_handle,
                        &labels,
                        &status_labels,
                        &lines,
                        &entries,
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
struct TraySessionEntry {
    session_id: String,
    title_label: String,
    detail_label: String,
}

/// Tuple used to detect "did the dynamic menu actually change?" so we can
/// skip `tray.set_menu` when nothing moved (avoids menu flicker on macOS
/// when the user is mid-interaction).
#[derive(Debug, PartialEq, Eq)]
struct TraySnapshotSig {
    /// Status lines that should cause a menu rebuild. Uptime is intentionally
    /// excluded because it changes every poll and replacing an open native
    /// tray menu closes it on macOS.
    stable_status_lines: Vec<String>,
    entries: Vec<(String, String)>,
    more: usize,
}

impl TraySnapshotSig {
    fn from(lines: &[String], entries: &[TraySessionEntry], more: usize) -> Self {
        Self {
            stable_status_lines: lines
                .iter()
                .enumerate()
                .filter_map(|(idx, line)| {
                    (idx != TRAY_STATUS_UPTIME_LINE_INDEX).then_some(line.clone())
                })
                .collect(),
            entries: entries
                .iter()
                .map(|s| {
                    (
                        s.session_id.clone(),
                        format!("{}\n{}", s.title_label, s.detail_label),
                    )
                })
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

/// Format the primary session row for the tray menu. This mirrors the sidebar's
/// first row as closely as a native menu allows: state badges first, then the
/// UTF-8-safe title. Detailed metadata is rendered as a disabled second row.
fn format_active_session_title_label(
    s: &SessionMeta,
    streaming: bool,
    pending: bool,
    labels: &TrayStatusLabels,
) -> String {
    let mut badges = Vec::new();
    if streaming {
        badges.push("▶".to_string());
    }
    if pending {
        badges.push(format_count_badge("⏸", s.pending_interaction_count));
    }
    if s.unread_count > 0 {
        badges.push(format_count_badge("●", s.unread_count));
    }
    if s.has_error {
        badges.push("⚠".to_string());
    }
    if s.pinned_at.is_some() {
        badges.push("★".to_string());
    }
    if badges.is_empty() {
        badges.push("•".to_string());
    }

    let raw_title = s
        .title
        .as_deref()
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .unwrap_or(labels.untitled_session);
    let title = ha_core::truncate_utf8(raw_title, TRAY_TITLE_BYTES);
    format!("  {} {title}", badges.join(" "))
}

/// Format the secondary tray row for a session. The sidebar shows agent + time;
/// the tray also includes the pinned model and message count because there is
/// no hover state or avatar to carry that context.
fn format_active_session_detail_label(s: &SessionMeta, agent_name: &str) -> String {
    let mut parts = Vec::new();

    let agent = agent_name.trim();
    if !agent.is_empty() {
        parts.push(ha_core::truncate_utf8(agent, TRAY_AGENT_BYTES).to_string());
    }

    if let Some(model) = format_model_summary(s) {
        parts.push(model);
    }

    if let Some(updated) = format_updated_time_for_tray(&s.updated_at) {
        parts.push(updated);
    }

    if s.message_count > 0 {
        parts.push(format!("#{}", s.message_count));
    }

    if parts.is_empty() {
        parts.push(s.id.chars().take(8).collect());
    }

    format!("    {}", parts.join(" · "))
}

fn format_count_badge(prefix: &str, count: i64) -> String {
    if count <= 1 {
        prefix.to_string()
    } else if count > 99 {
        format!("{prefix}99+")
    } else {
        format!("{prefix}{count}")
    }
}

fn format_model_summary(s: &SessionMeta) -> Option<String> {
    let provider = s
        .provider_name
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());
    let model = s
        .model_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty());

    let summary = match (provider, model) {
        (Some(provider), Some(model)) => format!("{provider}/{model}"),
        (Some(provider), None) => provider.to_string(),
        (None, Some(model)) => model.to_string(),
        (None, None) => return None,
    };

    Some(ha_core::truncate_utf8(&summary, TRAY_MODEL_BYTES).to_string())
}

fn format_updated_time_for_tray(updated_at: &str) -> Option<String> {
    let updated = chrono::DateTime::parse_from_rfc3339(updated_at)
        .ok()?
        .with_timezone(&chrono::Local);
    Some(updated.format("%m-%d %H:%M").to_string())
}

fn resolve_agent_display_name(agent_id: &str, cache: &mut HashMap<String, String>) -> String {
    if let Some(name) = cache.get(agent_id) {
        return name.clone();
    }

    let name = ha_core::agent_loader::load_agent(agent_id)
        .ok()
        .map(|def| def.config.name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| agent_id.to_string());
    cache.insert(agent_id.to_string(), name.clone());
    name
}

fn select_tray_session_candidates(
    sessions: Vec<SessionMeta>,
    streaming_ids: &std::collections::HashSet<String>,
) -> Vec<(SessionMeta, bool, bool)> {
    let mut in_progress = Vec::new();
    let mut recent_idle = Vec::new();
    for s in sessions {
        if !s.is_regular_chat() {
            continue;
        }
        let streaming = streaming_ids.contains(&s.id);
        let pending = s.pending_interaction_count > 0;
        if streaming || pending {
            in_progress.push((s, streaming, pending));
        } else {
            recent_idle.push((s, false, false));
        }
    }

    // Sort newest-first to mirror the sidebar. When no in-progress session
    // exists, the same ordering gives the tray a compact "recent chats" list.
    in_progress.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));
    recent_idle.sort_by(|a, b| b.0.updated_at.cmp(&a.0.updated_at));

    if in_progress.is_empty() {
        recent_idle
    } else {
        in_progress
    }
}

/// Resolve the regular conversations shown in the tray dropdown.
///
/// In-progress conversations win. They combine two condition sources:
///
/// 1. **Streaming**: session ids registered in `chat_engine::stream_seq`
///    with `source=Desktop` (LLM is currently producing tokens).
/// 2. **Pending interaction**: `SessionMeta.pending_interaction_count > 0`
///    (waiting on a tool approval or an `ask_user_question` answer).
///
/// If there are no in-progress regular conversations, the tray falls back to
/// the most recent regular conversations. Both modes return up to
/// [`TRAY_ACTIVE_SESSIONS_CAP`] entries plus the count of items truncated.
/// Order: by `updated_at DESC` to mirror the sidebar.
async fn compute_tray_regular_sessions(
    status_labels: &TrayStatusLabels,
) -> (Vec<TraySessionEntry>, usize) {
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

    let selected = select_tray_session_candidates(sessions, &streaming_ids);
    let total = selected.len();
    let truncated = total.saturating_sub(TRAY_ACTIVE_SESSIONS_CAP);
    let mut agent_names = HashMap::new();
    let kept: Vec<TraySessionEntry> = selected
        .into_iter()
        .take(TRAY_ACTIVE_SESSIONS_CAP)
        .map(|(s, streaming, pending)| TraySessionEntry {
            title_label: format_active_session_title_label(&s, streaming, pending, status_labels),
            detail_label: format_active_session_detail_label(
                &s,
                &resolve_agent_display_name(&s.agent_id, &mut agent_names),
            ),
            session_id: s.id,
        })
        .collect();

    (kept, truncated)
}

/// Build the full tray menu including the dynamic regular-conversation rows.
/// Items appear in this order:
///
/// 1. 5 disabled status rows (unchanged from before).
/// 2. Per regular session entry: one clickable item with id `tray_session:{id}`.
/// 3. Optional disabled "… {N} more" line if `more > 0`.
/// 4. Separator + the existing action items.
fn build_tray_menu<R: Runtime>(
    manager: &impl Manager<R>,
    labels: &TrayLabels,
    status_labels: &TrayStatusLabels,
    status_lines: &[String],
    entries: &[TraySessionEntry],
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

    let session_items: Vec<_> = entries
        .iter()
        .map(|s| {
            let title = MenuItemBuilder::with_id(
                format!("{TRAY_SESSION_PREFIX}{}", s.session_id),
                &s.title_label,
            )
            .build(manager)?;
            let detail = MenuItemBuilder::with_id(
                format!("{TRAY_SESSION_DETAIL_PREFIX}{}", s.session_id),
                &s.detail_label,
            )
            .enabled(false)
            .build(manager)?;
            Ok((title, detail))
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
    for (title, detail) in &session_items {
        builder = builder.item(title).item(detail);
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
            agent_id: ha_core::agent_loader::DEFAULT_AGENT_ID.to_string(),
            provider_id: None,
            provider_name: None,
            model_id: None,
            reasoning_effort: None,
            created_at: "2026-05-01T00:00:00Z".to_string(),
            updated_at: "2026-05-01T00:00:00Z".to_string(),
            pinned_at: None,
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
            kind: Default::default(),
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
    fn tray_signature_ignores_uptime_line() {
        let mut first = vec![
            "status".to_string(),
            "addr".to_string(),
            "uptime: 1s".to_string(),
            "connections".to_string(),
            "sessions".to_string(),
        ];
        let mut second = first.clone();
        second[TRAY_STATUS_UPTIME_LINE_INDEX] = "uptime: 6s".to_string();

        assert_eq!(
            TraySnapshotSig::from(&first, &[], 0),
            TraySnapshotSig::from(&second, &[], 0)
        );

        first[1] = "addr changed".to_string();
        assert_ne!(
            TraySnapshotSig::from(&first, &[], 0),
            TraySnapshotSig::from(&second, &[], 0)
        );
    }

    #[test]
    fn format_active_session_title_label_matches_sidebar_state_badges() {
        let labels = tray_status_labels("en");
        let mut s = dummy_session("a", Some("My session"));

        assert_eq!(
            format_active_session_title_label(&s, true, false, &labels),
            "  ▶ My session"
        );
        assert_eq!(
            format_active_session_title_label(&s, false, true, &labels),
            "  ⏸ My session"
        );
        s.pending_interaction_count = 2;
        s.unread_count = 7;
        s.has_error = true;
        s.pinned_at = Some("2026-05-01T01:00:00Z".to_string());
        assert_eq!(
            format_active_session_title_label(&s, true, true, &labels),
            "  ▶ ⏸2 ●7 ⚠ ★ My session"
        );

        // Empty title falls back to localized placeholder.
        let untitled = dummy_session("b", None);
        assert_eq!(
            format_active_session_title_label(&untitled, true, false, &labels),
            "  ▶ Untitled"
        );
        // Whitespace-only title is treated as empty.
        let blank = dummy_session("c", Some("   "));
        assert_eq!(
            format_active_session_title_label(&blank, false, true, &labels),
            "  ⏸ Untitled"
        );
    }

    #[test]
    fn select_tray_sessions_falls_back_to_recent_idle_when_none_in_progress() {
        let mut older = dummy_session("older", Some("Older"));
        older.updated_at = "2026-05-01T00:00:00Z".to_string();
        let mut newer = dummy_session("newer", Some("Newer"));
        newer.updated_at = "2026-05-02T00:00:00Z".to_string();

        let selected =
            select_tray_session_candidates(vec![older, newer], &std::collections::HashSet::new());

        assert_eq!(
            selected
                .iter()
                .map(|(s, streaming, pending)| (s.id.as_str(), *streaming, *pending))
                .collect::<Vec<_>>(),
            vec![("newer", false, false), ("older", false, false)]
        );
    }

    #[test]
    fn select_tray_sessions_prefers_in_progress_over_recent_idle() {
        let mut idle_newer = dummy_session("idle-newer", Some("Idle"));
        idle_newer.updated_at = "2026-05-02T00:00:00Z".to_string();
        let mut pending_older = dummy_session("pending-older", Some("Pending"));
        pending_older.updated_at = "2026-05-01T00:00:00Z".to_string();
        pending_older.pending_interaction_count = 1;

        let selected = select_tray_session_candidates(
            vec![idle_newer, pending_older],
            &std::collections::HashSet::new(),
        );

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].0.id, "pending-older");
        assert!(!selected[0].1);
        assert!(selected[0].2);
    }

    #[test]
    fn format_active_session_detail_label_adds_agent_model_and_message_context() {
        let mut s = dummy_session("a", Some("My session"));
        s.provider_name = Some("OpenAI".to_string());
        s.model_id = Some("gpt-5.4".to_string());
        s.message_count = 12;

        let detail = format_active_session_detail_label(&s, "Hope");

        assert!(detail.starts_with("    Hope · OpenAI/gpt-5.4 · "));
        assert!(detail.ends_with(" · #12"));
    }

    #[test]
    fn tray_updated_time_is_stable_for_signature_comparison() {
        assert_eq!(
            format_updated_time_for_tray("2026-05-01T00:00:00Z"),
            format_updated_time_for_tray("2026-05-01T00:00:00Z")
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
