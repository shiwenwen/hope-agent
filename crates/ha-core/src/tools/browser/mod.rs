//! Browser tool — collapsed 8-action surface.
//!
//! Top-level `action` selects one of:
//! - `status` — backend / connection / tab snapshot
//! - `profile` — list / launch / connect / disconnect / install_runtime
//! - `tabs` — list / new / select / close / open_user_tabs / claim / release /
//!   finalize
//! - `navigate` — go / back / forward / reload
//! - `snapshot` — role-based DOM tree / screenshot / pdf
//! - `act` — click / dblclick / fill (alias `type`) / hover / drag / select /
//!   press / upload
//! - `observe` — console / network / page_errors (ring buffer) / downloads
//! - `control` — resize / scroll / wait_for / handle_dialog / evaluate /
//!   raw_cdp / download_cancel
//!
//! Each handler grabs the active [`crate::browser::BrowserBackend`] via
//! [`crate::browser::acquire_backend`] and formats a string result for the
//! LLM. High-level URL actions run SSRF checks *before* the backend call.
//! `control.raw_cdp` is the advanced escape hatch, and it is *not* an unguarded
//! passthrough: a config kill switch, strict approval (no `Allow Always`),
//! backend-side method blocklists, and — in this file — per-method payload SSRF
//! scanning for `Runtime.evaluate` / `Runtime.callFunctionOn` / `Page.navigate`
//! all apply. Do not drop the scans in `control_raw_cdp` as redundant. The four
//! gates and their rationale live in `docs/architecture/browser.md`; the method
//! blocklists, and why `Network.*` is enumerated instead of prefix-blocked, are
//! documented on `BLOCKED_RAW_CDP_METHODS`.

use std::io::Cursor;
use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{self, MediaItem, MediaKind};
use crate::browser::{
    self, acquire_backend_for, reset_backend, ActKind, ActParams, BrowserBackend,
    BrowserBackendContext, BrowserBackendRequirement, DialogAction, ImageFormat, ObserveKind,
    PdfParams, RawCdpParams, ScreenshotParams, ScrollDirection, ScrollParams, Snapshot,
    SnapshotFormat, WaitParams,
};
use crate::tools::image_markers;

/// Image base64 prefix marker — detected by `agent.rs` for multimodal content.
pub const IMAGE_BASE64_PREFIX: &str = "__IMAGE_BASE64__";

pub(crate) async fn tool_browser(args: &Value, ctx: &super::ToolExecContext) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'action' parameter"))?;
    let session_id = ctx.session_id.as_deref();
    let started = std::time::Instant::now();
    let started_at = chrono::Utc::now().timestamp_millis();

    let result = match action {
        "status" => action_status(args).await,
        "profile" => action_profile(args, session_id).await,
        "tabs" => action_tabs(args, session_id).await,
        "navigate" => action_navigate(args, session_id).await,
        "snapshot" => action_snapshot(args, session_id).await,
        "act" => action_act(args, session_id).await,
        "observe" => action_observe(args, session_id).await,
        "control" => action_control(args, session_id).await,
        other => Err(anyhow!(
            "Unknown browser action: '{}'. Valid: status / profile / tabs / navigate / snapshot / act / observe / control",
            other
        )),
    };
    record_browser_action(args, ctx, action, &result, started, started_at);
    if result.is_ok() {
        emit_browser_activity_metadata(ctx, args, action).await;
    }
    result
}

/// Sub-operation for timeline purposes: `op` for tabs/navigate/control,
/// `kind` for act, `format` for snapshot.
fn browser_sub_op(args: &Value) -> Option<&str> {
    get_str(args, "op")
        .or_else(|| get_str(args, "kind"))
        .or_else(|| get_str(args, "format"))
}

/// Timeline whitelist — read-only queries (status/profile/observe/tabs.list/
/// snapshot.role) would spam the execution path and are skipped.
fn is_recordable_browser_action(action: &str, op: Option<&str>) -> bool {
    match action {
        "navigate" | "act" => true,
        "tabs" => matches!(
            op,
            Some("new" | "select" | "close" | "claim" | "release" | "finalize")
        ),
        "control" => matches!(
            op,
            Some(
                "resize"
                    | "scroll"
                    | "wait_for"
                    | "handle_dialog"
                    | "evaluate"
                    | "raw_cdp"
                    | "download_cancel"
            )
        ),
        "snapshot" => matches!(op, Some("screenshot" | "image" | "pdf")),
        _ => false,
    }
}

/// Central frame-emit policy — replicates the exact per-handler behaviour this
/// choke point absorbed: `act` emits even on failure (page state may have
/// partially changed), `navigate` / `tabs.new|select|claim` only on success.
fn should_emit_frame_after(action: &str, op: Option<&str>, ok: bool) -> bool {
    match action {
        "act" => true,
        "navigate" => ok,
        "tabs" => ok && matches!(op, Some("new" | "select" | "claim")),
        _ => false,
    }
}

/// Redacted human-oriented `(target, detail, url)` summary for the timeline.
/// `act.fill` text never enters the payload — length only.
fn browser_action_summary(
    args: &Value,
    action: &str,
    op: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    let target = get_u32(args, "ref")
        .map(|r| format!("ref={r}"))
        .or_else(|| {
            get_str(args, "target_id")
                .or_else(|| get_str(args, "page_id"))
                .map(str::to_string)
        });
    let url = get_str(args, "url").map(str::to_string);
    let detail = match (action, op) {
        ("act", Some("fill")) => {
            get_str_any(args, "text").map(crate::tool_actions::redacted_text_summary)
        }
        ("act", Some("press")) => get_str(args, "key").map(|k| format!("key={k}")),
        ("act", Some("select")) => {
            get_str_array(args, "values").map(|v| format!("values({})", v.len()))
        }
        ("act", Some("upload")) => get_str(args, "file_path").map(str::to_string),
        ("control", Some("evaluate")) => get_str(args, "expression")
            .or_else(|| get_str(args, "script"))
            .map(|s| format!("js({} chars)", s.chars().count())),
        ("control", Some("raw_cdp")) => get_str(args, "method").map(str::to_string),
        ("control", Some("scroll")) => get_str(args, "direction").map(str::to_string),
        _ => None,
    };
    (target, detail, url)
}

/// Choke-point recorder: builds the redacted action event, pushes it into the
/// ring buffer + EventBus, and triggers the follow-up frame capture that
/// backfills the thumbnail via `action_id`.
fn record_browser_action(
    args: &Value,
    ctx: &super::ToolExecContext,
    action: &str,
    result: &Result<String>,
    started: std::time::Instant,
    started_at: i64,
) {
    let op = browser_sub_op(args);
    let ok = result.is_ok();
    let emit_frame = should_emit_frame_after(action, op, ok);
    let recordable = is_recordable_browser_action(action, op);
    if !emit_frame && !recordable {
        return;
    }
    let action_id = crate::tool_actions::new_action_id();
    if recordable {
        let (target, detail, url) = browser_action_summary(args, action, op);
        crate::tool_actions::record_action(crate::tool_actions::ToolActionEvent {
            action_id: action_id.clone(),
            source: crate::tool_actions::ToolActionSource::Browser,
            session_id: ctx.session_id.clone(),
            action: action.to_string(),
            op: op.map(str::to_string),
            target,
            detail,
            url,
            app: None,
            ok,
            error: result
                .as_ref()
                .err()
                .map(|e| crate::tool_actions::clamp_error(&e.to_string())),
            duration_ms: started.elapsed().as_millis() as u64,
            started_at,
            tool_call_id: ctx.tool_call_id.clone(),
            has_frame: emit_frame,
        });
    }
    if emit_frame {
        browser::frame::emit_frame_async(
            ctx.session_id.clone(),
            recordable.then(|| action_id.clone()),
        );
    }
}

async fn emit_browser_activity_metadata(ctx: &super::ToolExecContext, args: &Value, action: &str) {
    if ctx.metadata_sink.is_none() {
        return;
    }
    let info = browser::frame::current_frame_info(ctx.session_id.as_deref())
        .await
        .ok()
        .flatten();
    let op = get_str(args, "op")
        .or_else(|| get_str(args, "kind"))
        .or_else(|| get_str(args, "format"));
    let arg_url = get_str(args, "url");
    let target_id = get_str(args, "target_id")
        .or_else(|| get_str(args, "page_id"))
        .map(str::to_string)
        .or_else(|| info.as_ref().and_then(|i| i.target_id.clone()));
    let url = arg_url
        .map(str::to_string)
        .or_else(|| info.as_ref().and_then(|i| i.url.clone()));
    ctx.emit_metadata(json!({
        "kind": "browser_activity",
        "action": action,
        "op": op,
        "targetId": target_id,
        "url": url,
        "title": info.as_ref().and_then(|i| i.title.clone()),
        "backend": info.as_ref().map(|i| i.backend.clone()),
        "sessionId": info.as_ref().and_then(|i| i.session_id.clone()).or_else(|| ctx.session_id.clone()),
        "callId": ctx.tool_call_id,
        "at": chrono::Utc::now().timestamp_millis(),
    }))
    .await;
}

// ── Param helpers ────────────────────────────────────────────────────────

fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| {
            v.as_str()
                .or_else(|| v.get("text").and_then(|t| t.as_str()))
        })
        // Codex-style providers serialise omitted fields as empty strings
        // rather than `null`. Treat `""` as "field not provided" so
        // downstream callers don't pass an empty `executable_path` /
        // `profile` / `url` to chromiumoxide (which then fails the spawn
        // with a confusing `No such file or directory` since `""` parses
        // as an explicit zero-length path).
        .filter(|s| !s.is_empty())
}

/// Like [`get_str`] but preserves empty strings. Use for fields where the
/// empty value carries meaning — e.g. `act.kind=fill text=""` clears an
/// input. [`get_str`]'s empty-string-as-missing filter would silently
/// turn that into a "requires 'text' parameter" error.
fn get_str_any<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| {
        v.as_str()
            .or_else(|| v.get("text").and_then(|t| t.as_str()))
    })
}

fn get_u32(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v as u32)
}

fn get_u64(args: &Value, key: &str) -> Option<u64> {
    args.get(key).and_then(|v| v.as_u64())
}

fn get_i64(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(|v| v.as_i64())
}

fn get_bool(args: &Value, key: &str) -> Option<bool> {
    args.get(key).and_then(|v| v.as_bool())
}

fn get_str_array(args: &Value, key: &str) -> Option<Vec<String>> {
    args.get(key).and_then(|v| v.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect()
    })
}

async fn check_url_via_ssrf(url: &str) -> Result<()> {
    let ssrf_cfg = &crate::config::cached_config().ssrf;
    crate::security::ssrf::check_url(url, ssrf_cfg.browser(), &ssrf_cfg.trusted_hosts).await?;
    Ok(())
}

// ── status ───────────────────────────────────────────────────────────────

async fn action_status(_args: &Value) -> Result<String> {
    // `status_backend` builds a cheap session-less probe when the extension is
    // the effective backend (it is never cached, so `peek_active` would miss it
    // and wrongly report "disconnected"); it never force-launches a CDP Chrome.
    let extension_status = browser::current_status();
    let active = browser::status_backend().await;
    let Some(backend) = active else {
        return Ok(format!(
            "Browser disconnected.\nExtension: {:?} — {}\nNext action: {}\nUse `profile.op=launch` to start an isolated CDP Chrome, or install/enable the extension for real Chrome tabs and logged-in sessions.",
            extension_status.kind,
            extension_status.message,
            extension_status.next_action.as_deref().unwrap_or("none")
        ));
    };
    let status = match backend.status().await {
        Ok(status) => status,
        Err(e) => {
            // The backend dropped between the readiness probe and the query
            // (e.g. the native host disconnected in the race window). Report a
            // friendly disconnected status instead of a hard tool error.
            return Ok(format!(
                "Browser disconnected.\nExtension: {:?} — {}\nNext action: {}\nDetail: {}",
                extension_status.kind,
                extension_status.message,
                extension_status.next_action.as_deref().unwrap_or("none"),
                e
            ));
        }
    };
    let mut out = format!(
        "Backend: {}\nConnected: {}\n",
        status.backend, status.connected
    );
    if status.backend == "cdp" && !extension_status.backend_available {
        out.push_str(&format!(
            "Extension: {:?} — {}\nFallback: CDP is active for actions that allow isolated-browser fallback.\n",
            extension_status.kind, extension_status.message
        ));
    }
    if let Some(active_id) = &status.active_target_id {
        out.push_str(&format!("Active tab: {}\n", active_id));
    }
    if let Some(diagnostics) = &status.diagnostics {
        if let Some(flat) = format_flat_session_diagnostics(diagnostics) {
            out.push_str(&flat);
        }
    }
    if !status.tabs.is_empty() {
        out.push_str(&format!("Tabs ({}):\n", status.tabs.len()));
        for tab in &status.tabs {
            let marker = if tab.is_active { " [active]" } else { "" };
            out.push_str(&format!(
                "  - {} {} \"{}\"{}\n",
                tab.target_id, tab.url, tab.title, marker
            ));
        }
    }
    Ok(out)
}

fn format_flat_session_diagnostics(diagnostics: &Value) -> Option<String> {
    let sessions = diagnostics.get("sessions").and_then(Value::as_array)?;
    let enabled = diagnostics
        .get("flatSessionEnabled")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut out = format!(
        "Flat sessions: {}{}\n",
        sessions.len(),
        if enabled { " (enabled)" } else { " (disabled)" }
    );
    if let Some(frame_tree) = diagnostics.get("frameTree") {
        let frames = frame_tree
            .get("frames")
            .and_then(Value::as_array)
            .map(Vec::len)
            .unwrap_or(0);
        let available = frame_tree
            .get("available")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        out.push_str(&format!(
            "Frame tree: {} frame(s){}\n",
            frames,
            if available {
                ""
            } else {
                " (webNavigation unavailable)"
            }
        ));
    }
    for session in sessions.iter().take(8) {
        let session_id = session
            .get("sessionId")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let target = session.get("targetInfo").unwrap_or(&Value::Null);
        let kind = target
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let url = target.get("url").and_then(Value::as_str).unwrap_or("");
        let mut suffix = String::new();
        if let Some(matched) = session.get("matchedFrame") {
            let status = matched
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            if status == "matched" {
                if let Some(frame_id) = matched.get("frameId").and_then(Value::as_i64) {
                    suffix.push_str(&format!(" frameId={frame_id}"));
                }
                if let Some(parent_frame_id) = matched.get("parentFrameId").and_then(Value::as_i64)
                {
                    suffix.push_str(&format!(" parentFrameId={parent_frame_id}"));
                }
                if let Some(document_id) = matched.get("documentId").and_then(Value::as_str) {
                    suffix.push_str(&format!(" documentId={document_id}"));
                }
            } else {
                suffix.push_str(&format!(" frameMatch={status}"));
            }
        }
        out.push_str(&format!("  - {session_id} {kind} {url}{suffix}\n"));
    }
    Some(out)
}

// ── profile ──────────────────────────────────────────────────────────────

async fn action_profile(args: &Value, session_id: Option<&str>) -> Result<String> {
    let op = get_str(args, "op").ok_or_else(|| {
        anyhow!(
            "profile requires 'op' parameter (list / launch / connect / disconnect / install_runtime)"
        )
    })?;

    match op {
        "list" => profile_list().await,
        "launch" => profile_launch(args, session_id).await,
        "connect" => profile_connect(args).await,
        "disconnect" => profile_disconnect().await,
        "install_runtime" => profile_install_runtime().await,
        other => Err(anyhow!(
            "Unknown profile.op: '{}'. Valid: list / launch / connect / disconnect / install_runtime",
            other
        )),
    }
}

async fn profile_list() -> Result<String> {
    let profiles = crate::browser::profile::list_profiles();
    if profiles.is_empty() {
        return Ok("No browser profiles found.".to_string());
    }
    let active_profile = {
        let state = crate::browser_state::get_browser_state().lock().await;
        state.profile.clone()
    };
    let mut lines = vec![format!("Browser profiles ({}):", profiles.len())];
    for profile in &profiles {
        let marker = if active_profile.as_deref() == Some(profile.name.as_str()) {
            " [active]"
        } else {
            ""
        };
        let kind = if profile.persistent {
            "persistent"
        } else {
            "ephemeral"
        };
        let headless = if profile.headless { ", headless" } else { "" };
        lines.push(format!("  - {} ({kind}{headless}){}", profile.name, marker));
    }
    Ok(lines.join("\n"))
}

/// Dispatch `profile.op=launch`. Accepts only `profile=<name>` going
/// forward; the legacy `target=managed|user_attach` parameter is removed
/// and returns a migration error pointing at the new parameter.
///
/// Built-in profile names: `managed` (default, ephemeral) and `user_attach`
/// (persistent, port 9222). Users can configure additional profiles in
/// `AppConfig.browser.profiles`.
async fn profile_launch(args: &Value, session_id: Option<&str>) -> Result<String> {
    let _ = session_id;

    if args.get("target").is_some_and(|v| !v.is_null()) {
        return Err(anyhow!(
            "The `target` parameter is no longer supported. Use \
             `profile=managed` (ephemeral) or `profile=user_attach` \
             (persistent, port 9222) instead. See settings → Browser → \
             Profiles for the full list."
        ));
    }

    let profile_name = get_str(args, "profile")
        .map(|s| s.to_string())
        .unwrap_or_else(crate::browser::profile::default_profile_name);

    let resolved = crate::browser::profile::resolve_profile(&profile_name)?;
    let exec_override = get_str(args, "executable_path").map(|s| s.to_string());
    let headless = get_bool(args, "headless").unwrap_or(resolved.headless);
    let port = match resolved.port {
        Some(p) => p,
        None => crate::browser::spawn::pick_managed_port().await?,
    };
    let exec_resolved = exec_override.or_else(|| resolved.executable.clone());
    let extra = resolved.extra_args.clone();
    let spec = crate::browser::spawn::LaunchSpec {
        profile: &resolved.name,
        executable: exec_resolved.as_deref(),
        user_data_dir: &resolved.user_data_dir,
        port,
        headless,
        extra_args: &extra,
    };

    let mut state = crate::browser_state::get_browser_state().lock().await;
    // `needs_cleanup` (not `is_connected`) — the ws may already be dead but
    // the Chrome process / handler task still owns the user-data-dir lock.
    if state.needs_cleanup() {
        state.disconnect().await;
    }
    state.spawn_chrome_and_connect(spec).await?;
    let page_count = state.pages.len();
    drop(state);

    reset_backend().await;
    let _ = acquire_cdp_backend().await?;

    let persistent_note = if resolved.persistent {
        " (persistent profile — cookies / logins survive disconnect)"
    } else {
        ""
    };
    Ok(format!(
        "Chrome launched successfully{} for profile '{}' on port {}{}. {} page(s) available.",
        if headless { " (headless)" } else { "" },
        profile_name,
        port,
        persistent_note,
        page_count
    ))
}

async fn profile_connect(args: &Value) -> Result<String> {
    let url = get_str(args, "url").unwrap_or("http://127.0.0.1:9222");
    // Treat the CDP endpoint as an outbound URL — refuse anything outside the
    // SSRF policy (defaults allow loopback; private network needs opt-in).
    // Shared helper so UI (`browser_ui::connect`) / HTTP
    // (`/api/browser/connect`) / tool (`profile.connect`) apply the same
    // scheme + SSRF gate.
    crate::browser::validate_cdp_endpoint_url(url).await?;

    let mut state = crate::browser_state::get_browser_state().lock().await;
    if state.needs_cleanup() {
        state.disconnect().await;
    }
    state.connect(url).await?;
    let page_count = state.pages.len();
    let active = state.active_page_id.clone().unwrap_or_default();
    drop(state);

    reset_backend().await;
    let _ = acquire_cdp_backend().await?;

    Ok(format!(
        "Connected to Chrome at {}. Found {} page(s). Active page: {}",
        url, page_count, active
    ))
}

async fn profile_disconnect() -> Result<String> {
    let mut state = crate::browser_state::get_browser_state().lock().await;
    // Use `needs_cleanup` instead of `is_connected` so disconnect runs even
    // when the heartbeat has marked the ws dead — Chrome may still be alive
    // (idle ws close doesn't kill the process) and we must reap it to free
    // the SingletonLock for the next launch.
    if !state.needs_cleanup() {
        return Ok("Not connected to any browser.".to_string());
    }
    state.disconnect().await;
    drop(state);
    reset_backend().await;
    Ok("Browser disconnected.".to_string())
}

/// Download + unpack the pinned Chromium snapshot so the agent can run
/// `profile.op=launch` on systems with no Chrome installed. Idempotent —
/// re-running once the binary exists returns immediately.
///
/// Progress is emitted on the `browser:chromium_download_progress`
/// EventBus channel; tool-level callers should treat this as
/// `async_capable=true` so completion can arrive through async job injection.
async fn profile_install_runtime() -> Result<String> {
    use crate::browser::runtime;

    if let Some(cached) = runtime::cached_binary_path() {
        return Ok(format!(
            "Chromium runtime already installed at {}.",
            cached.display()
        ));
    }

    let spec = runtime::spec_for_current_platform().ok_or_else(|| {
        anyhow!(
            "Chromium runtime is not available for this platform / architecture. \
             Install Google Chrome system-wide instead."
        )
    })?;
    crate::app_info!(
        "browser",
        "install_runtime",
        "downloading Chromium runtime rev={} for {}",
        spec.revision,
        spec.platform_key
    );

    let binary = runtime::install_with_event_bus_progress().await?;
    crate::app_info!(
        "browser",
        "install_runtime",
        "Chromium runtime ready at {}",
        binary.display()
    );
    Ok(format!(
        "Chromium runtime installed at {}. Subsequent `profile.op=launch` calls \
         will use this binary when no system Chrome is found.",
        binary.display()
    ))
}

// ── tabs ─────────────────────────────────────────────────────────────────

async fn action_tabs(args: &Value, session_id: Option<&str>) -> Result<String> {
    let op = get_str(args, "op").ok_or_else(|| {
        anyhow!(
            "tabs requires 'op' parameter (list / new / select / close / open_user_tabs / claim / release / finalize)"
        )
    })?;

    match op {
        "list" => tabs_list(session_id).await,
        "new" => tabs_new(args, session_id).await,
        "select" => tabs_select(args, session_id).await,
        "close" => tabs_close(args, session_id).await,
        "open_user_tabs" => tabs_open_user_tabs(args, session_id).await,
        "claim" => tabs_claim(args, session_id).await,
        "release" => tabs_release(args, session_id).await,
        "finalize" => tabs_finalize(args, session_id).await,
        other => Err(anyhow!(
            "Unknown tabs.op: '{}'. Valid: list / new / select / close / open_user_tabs / claim / release / finalize",
            other
        )),
    }
}

async fn tabs_list(session_id: Option<&str>) -> Result<String> {
    let backend = acquire_browser_backend(session_id, "tabs.list").await?;
    let tabs = backend.list_pages().await?;
    if tabs.is_empty() {
        return Ok("No pages open.".to_string());
    }
    let mut lines = vec!["Open pages:".to_string()];
    for t in &tabs {
        let marker = if t.is_active { " [active]" } else { "" };
        lines.push(format!(
            "  - {} {} \"{}\"{}",
            t.target_id, t.url, t.title, marker
        ));
    }
    Ok(lines.join("\n"))
}

async fn tabs_new(args: &Value, session_id: Option<&str>) -> Result<String> {
    let url = get_str(args, "url");
    if let Some(u) = url {
        if u != "about:blank" {
            check_url_via_ssrf(u).await?;
        }
    }
    let backend = acquire_browser_backend(session_id, "tabs.new").await?;
    let mut tab = backend.new_page(url).await?;
    // The backend's `new_page` may return a blank tab even when a URL was
    // requested (e.g. when Chrome opens its new-tab page first). Only follow
    // up when the tab clearly didn't load anything; legitimate redirects
    // (http→https, login-gate 302, one-time tokens) must NOT be re-navigated
    // or we risk consuming the token twice or stomping on the redirect chain.
    if let Some(target) = url {
        if target != "about:blank" && tab_url_indicates_blank_load(&tab.url) {
            backend.navigate(target).await?;
            tab.url = target.to_string();
        }
    }
    Ok(format!(
        "New page created: {} (url: {})",
        tab.target_id, tab.url
    ))
}

fn tab_url_indicates_blank_load(tab_url: &str) -> bool {
    let trimmed = tab_url.trim();
    if trimmed.is_empty() {
        return true;
    }
    matches!(
        trimmed,
        "about:blank"
            | "about:newtab"
            | "chrome://newtab/"
            | "chrome://newtab"
            | "chrome://new-tab-page/"
            | "chrome://new-tab-page"
            | "edge://newtab/"
            | "edge://newtab"
    ) || trimmed.starts_with("data:,")
}

async fn tabs_select(args: &Value, session_id: Option<&str>) -> Result<String> {
    let target = get_str(args, "target_id")
        .or_else(|| get_str(args, "page_id"))
        .ok_or_else(|| anyhow!("tabs.select requires 'target_id'"))?;
    let backend = acquire_browser_backend(session_id, "tabs.select").await?;
    backend.select_page(target).await?;
    Ok(format!("Switched to page: {}", target))
}

async fn tabs_close(args: &Value, session_id: Option<&str>) -> Result<String> {
    let target = get_str(args, "target_id")
        .or_else(|| get_str(args, "page_id"))
        .ok_or_else(|| anyhow!("tabs.close requires 'target_id'"))?;
    let backend = acquire_browser_backend(session_id, "tabs.close").await?;
    backend.close_page(target).await?;
    Ok(format!("Page '{}' closed.", target))
}

async fn tabs_open_user_tabs(_args: &Value, session_id: Option<&str>) -> Result<String> {
    let backend = require_extension_tabs("tabs.open_user_tabs", session_id).await?;
    let tabs = backend.list_pages().await?;
    if tabs.is_empty() {
        return Ok("No user Chrome tabs are currently visible to the extension.".to_string());
    }
    let mut lines = vec![format!("User Chrome tabs ({}):", tabs.len())];
    for tab in &tabs {
        let marker = if tab.is_active { " [active]" } else { "" };
        lines.push(format!(
            "  - {} {} \"{}\"{}",
            tab.target_id, tab.url, tab.title, marker
        ));
    }
    Ok(lines.join("\n"))
}

async fn tabs_claim(args: &Value, session_id: Option<&str>) -> Result<String> {
    let target = get_str(args, "target_id")
        .or_else(|| get_str(args, "page_id"))
        .ok_or_else(|| anyhow!("tabs.claim requires 'target_id'"))?;
    let steal = get_bool(args, "steal").unwrap_or(false);
    let backend = require_extension_tabs("tabs.claim", session_id).await?;
    backend.claim_page(target, steal).await?;
    Ok(if steal {
        format!("Claimed user Chrome tab with lease steal: {}", target)
    } else {
        format!("Claimed user Chrome tab: {}", target)
    })
}

async fn tabs_release(args: &Value, session_id: Option<&str>) -> Result<String> {
    let target = get_str(args, "target_id")
        .or_else(|| get_str(args, "page_id"))
        .ok_or_else(|| anyhow!("tabs.release requires 'target_id'"))?;
    let backend = require_extension_tabs("tabs.release", session_id).await?;
    backend.release_page(target).await
}

async fn tabs_finalize(args: &Value, session_id: Option<&str>) -> Result<String> {
    let keep = get_str_array(args, "keep").unwrap_or_default();
    let backend = require_extension_tabs("tabs.finalize", session_id).await?;
    backend.finalize_pages(&keep).await
}

// ── navigate ─────────────────────────────────────────────────────────────

async fn action_navigate(args: &Value, session_id: Option<&str>) -> Result<String> {
    let op = get_str(args, "op").unwrap_or("go");
    let backend = acquire_browser_backend(session_id, "navigate").await?;
    let result = match op {
        "go" => {
            let url = get_str(args, "url").ok_or_else(|| anyhow!("navigate.go requires 'url'"))?;
            check_url_via_ssrf(url).await?;
            backend.navigate(url).await
        }
        "back" => backend.go_back().await,
        "forward" => backend.go_forward().await,
        "reload" => backend.reload().await,
        other => {
            return Err(anyhow!(
                "Unknown navigate.op: '{}'. Valid: go / back / forward / reload",
                other
            ))
        }
    };
    result
}

// ── snapshot ─────────────────────────────────────────────────────────────

async fn action_snapshot(args: &Value, session_id: Option<&str>) -> Result<String> {
    let format = get_str(args, "format").unwrap_or("role");
    let backend = acquire_browser_backend(session_id, "snapshot").await?;

    match format {
        "role" | "aria" => snapshot_role(&*backend).await,
        "screenshot" | "image" => snapshot_screenshot(args, &*backend, session_id).await,
        "pdf" => snapshot_pdf(args, &*backend).await,
        other => Err(anyhow!(
            "Unknown snapshot.format: '{}'. Valid: role / screenshot / pdf",
            other
        )),
    }
}

async fn snapshot_role(backend: &dyn BrowserBackend) -> Result<String> {
    let snap = backend.take_snapshot(SnapshotFormat::Role).await?;
    let mut out = format!(
        "[Page Snapshot] {} - \"{}\"\nViewport: {}x{}\n\n",
        snap.url, snap.title, snap.viewport.0, snap.viewport.1
    );
    for el in &snap.elements {
        let indent = "  ".repeat(el.depth.min(10) as usize);
        let readonly = el.attrs.get("readonly").map(String::as_str) == Some("true");
        let mut line = if readonly {
            format!("{}[ax={}] {}", indent, el.ref_id, el.role)
        } else {
            format!("{}[ref={}] {}", indent, el.ref_id, el.role)
        };
        if !el.text.is_empty() {
            line.push_str(&format!(" \"{}\"", el.text));
        }
        if let Some(url) = el.attrs.get("url") {
            line.push_str(&format!(" url={}", url));
        }
        if let Some(value) = el.attrs.get("value") {
            line.push_str(&format!(" value=\"{}\"", value));
        }
        if let Some(placeholder) = el.attrs.get("placeholder") {
            line.push_str(&format!(" placeholder=\"{}\"", placeholder));
        }
        if el.attrs.get("checked").map(String::as_str) == Some("true") {
            line.push_str(" [checked]");
        }
        if el.attrs.get("disabled").map(String::as_str) == Some("true") {
            line.push_str(" [disabled]");
        }
        if readonly {
            line.push_str(" [read-only]");
        }
        out.push_str(&line);
        out.push('\n');
    }
    if snap.truncated {
        out.push_str(
            "\n[Truncated: max 300 elements. Narrow scope with `control.op=evaluate` if needed.]\n",
        );
    }
    Ok(out)
}

async fn snapshot_screenshot(
    args: &Value,
    backend: &dyn BrowserBackend,
    session_id: Option<&str>,
) -> Result<String> {
    let raw_format = get_str(args, "image_format").unwrap_or("png");
    let format = match raw_format.to_ascii_lowercase().as_str() {
        "jpeg" | "jpg" => ImageFormat::Jpeg,
        _ => ImageFormat::Png,
    };
    let full_page = get_bool(args, "full_page").unwrap_or(false);
    let ref_id = get_u32(args, "ref");
    let annotate = get_bool(args, "annotate")
        .or_else(|| get_bool(args, "annotated"))
        .unwrap_or(false)
        && ref_id.is_none();
    let snapshot_for_annotation = if annotate {
        Some(backend.take_snapshot(SnapshotFormat::Role).await?)
    } else {
        None
    };
    let mut bytes = backend
        .take_screenshot(ScreenshotParams {
            format,
            full_page,
            quality: None,
            ref_id,
        })
        .await?;
    let annotation_count = if let Some(snapshot) = snapshot_for_annotation.as_ref() {
        match annotate_screenshot_bytes(&bytes, format, snapshot) {
            Ok(Some((annotated, count))) => {
                bytes = annotated;
                count
            }
            Ok(None) => 0,
            Err(e) => {
                app_warn!(
                    "tool",
                    "browser",
                    "Failed to annotate browser screenshot: {}",
                    e
                );
                0
            }
        }
    } else {
        0
    };
    let mime = format.mime();
    let ext = format.extension();
    let display_filename = if annotation_count > 0 {
        format!("browser_screenshot_annotated.{ext}")
    } else {
        format!("browser_screenshot.{ext}")
    };
    let caption = format!(
        "Screenshot captured (format: {}{}{}{})",
        ext,
        if full_page { ", full page" } else { "" },
        ref_id
            .map(|id| format!(", ref={id} crop"))
            .unwrap_or_default(),
        if annotation_count > 0 {
            format!(", annotated refs={annotation_count}")
        } else {
            String::new()
        }
    );
    match attachments::save_attachment_bytes(session_id, &display_filename, &bytes) {
        Ok(saved_path) => {
            let item = MediaItem::from_saved_path(
                session_id,
                &saved_path,
                &display_filename,
                mime.to_string(),
                bytes.len() as u64,
                MediaKind::Image,
                Some(caption.clone()),
            );
            let items_json =
                serde_json::to_string(&vec![item]).unwrap_or_else(|_| "[]".to_string());
            let marker = image_markers::build_image_file_marker(mime, &saved_path, &caption);
            Ok(format!("{MEDIA_ITEMS_PREFIX}{items_json}\n{marker}"))
        }
        Err(e) => {
            app_warn!(
                "tool",
                "browser",
                "Failed to save screenshot as attachment; falling back to inline base64: {}",
                e
            );
            let b64 = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
            Ok(image_markers::build_image_base64_marker(
                mime, &b64, &caption,
            ))
        }
    }
}

fn annotate_screenshot_bytes(
    bytes: &[u8],
    format: ImageFormat,
    snapshot: &Snapshot,
) -> Result<Option<(Vec<u8>, usize)>> {
    let viewport_w = snapshot.viewport.0.max(1) as f32;
    let mut image = image::load_from_memory(bytes)
        .map_err(|e| anyhow!("decode screenshot for annotation: {e}"))?
        .to_rgba8();
    let scale = image.width() as f32 / viewport_w;
    let mut count = 0usize;
    for element in snapshot.elements.iter().take(160) {
        let Some(bounds) = element
            .attrs
            .get("bounds")
            .and_then(|raw| parse_bounds(raw))
        else {
            continue;
        };
        if bounds.2 < 4.0 || bounds.3 < 4.0 {
            continue;
        }
        let x = (bounds.0 * scale).round() as i32;
        let y = (bounds.1 * scale).round() as i32;
        let w = (bounds.2 * scale).round().max(1.0) as i32;
        let h = (bounds.3 * scale).round().max(1.0) as i32;
        if x >= image.width() as i32 || y >= image.height() as i32 || x + w <= 0 || y + h <= 0 {
            continue;
        }
        let color = if element.attrs.get("readonly").map(String::as_str) == Some("true") {
            image::Rgba([130, 130, 130, 255])
        } else {
            image::Rgba([255, 42, 91, 255])
        };
        draw_rect_outline(&mut image, x, y, w, h, color);
        draw_ref_label(&mut image, element.ref_id, x.max(0), y.max(0), color);
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }

    let mut out = Cursor::new(Vec::new());
    let dynamic = image::DynamicImage::ImageRgba8(image);
    let output_format = match format {
        ImageFormat::Png => image::ImageFormat::Png,
        ImageFormat::Jpeg => image::ImageFormat::Jpeg,
    };
    dynamic
        .write_to(&mut out, output_format)
        .map_err(|e| anyhow!("encode annotated screenshot: {e}"))?;
    Ok(Some((out.into_inner(), count)))
}

fn parse_bounds(raw: &str) -> Option<(f32, f32, f32, f32)> {
    let parts = raw
        .split(',')
        .map(str::trim)
        .map(str::parse::<f32>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    if parts.len() != 4 {
        return None;
    }
    Some((parts[0], parts[1], parts[2], parts[3]))
}

fn draw_rect_outline(
    image: &mut image::RgbaImage,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    color: image::Rgba<u8>,
) {
    for stroke in 0..2 {
        draw_hline(image, x, y + stroke, w, color);
        draw_hline(image, x, y + h - 1 - stroke, w, color);
        draw_vline(image, x + stroke, y, h, color);
        draw_vline(image, x + w - 1 - stroke, y, h, color);
    }
}

fn draw_hline(image: &mut image::RgbaImage, x: i32, y: i32, w: i32, color: image::Rgba<u8>) {
    if y < 0 || y >= image.height() as i32 {
        return;
    }
    let start = x.max(0);
    let end = (x + w).min(image.width() as i32);
    for px in start..end {
        image.put_pixel(px as u32, y as u32, color);
    }
}

fn draw_vline(image: &mut image::RgbaImage, x: i32, y: i32, h: i32, color: image::Rgba<u8>) {
    if x < 0 || x >= image.width() as i32 {
        return;
    }
    let start = y.max(0);
    let end = (y + h).min(image.height() as i32);
    for py in start..end {
        image.put_pixel(x as u32, py as u32, color);
    }
}

fn draw_ref_label(
    image: &mut image::RgbaImage,
    ref_id: u32,
    x: i32,
    y: i32,
    color: image::Rgba<u8>,
) {
    let text = ref_id.to_string();
    let scale = 2i32;
    let digit_w = 3 * scale;
    let digit_h = 5 * scale;
    let gap = scale;
    let pad = 2;
    let width = text.len() as i32 * digit_w + (text.len().saturating_sub(1) as i32 * gap) + pad * 2;
    let height = digit_h + pad * 2;
    fill_rect(image, x, y, width, height, image::Rgba([20, 20, 20, 220]));
    for (index, ch) in text.chars().enumerate() {
        if let Some(pattern) = digit_pattern(ch) {
            draw_digit(
                image,
                pattern,
                x + pad + index as i32 * (digit_w + gap),
                y + pad,
                scale,
                color,
            );
        }
    }
}

fn fill_rect(image: &mut image::RgbaImage, x: i32, y: i32, w: i32, h: i32, color: image::Rgba<u8>) {
    let start_x = x.max(0);
    let end_x = (x + w).min(image.width() as i32);
    let start_y = y.max(0);
    let end_y = (y + h).min(image.height() as i32);
    for py in start_y..end_y {
        for px in start_x..end_x {
            image.put_pixel(px as u32, py as u32, color);
        }
    }
}

fn draw_digit(
    image: &mut image::RgbaImage,
    pattern: [&str; 5],
    x: i32,
    y: i32,
    scale: i32,
    color: image::Rgba<u8>,
) {
    for (row, line) in pattern.iter().enumerate() {
        for (col, bit) in line.chars().enumerate() {
            if bit != '1' {
                continue;
            }
            fill_rect(
                image,
                x + col as i32 * scale,
                y + row as i32 * scale,
                scale,
                scale,
                color,
            );
        }
    }
}

fn digit_pattern(ch: char) -> Option<[&'static str; 5]> {
    Some(match ch {
        '0' => ["111", "101", "101", "101", "111"],
        '1' => ["010", "110", "010", "010", "111"],
        '2' => ["111", "001", "111", "100", "111"],
        '3' => ["111", "001", "111", "001", "111"],
        '4' => ["101", "101", "111", "001", "001"],
        '5' => ["111", "100", "111", "001", "111"],
        '6' => ["111", "100", "111", "101", "111"],
        '7' => ["111", "001", "001", "010", "010"],
        '8' => ["111", "101", "111", "101", "111"],
        '9' => ["111", "101", "111", "001", "111"],
        _ => return None,
    })
}

async fn snapshot_pdf(args: &Value, backend: &dyn BrowserBackend) -> Result<String> {
    let bytes = backend
        .save_pdf(PdfParams {
            paper_format: get_str(args, "paper_format").map(String::from),
            landscape: get_bool(args, "landscape"),
            print_background: get_bool(args, "print_background"),
            ..Default::default()
        })
        .await?;
    // `output_path` is LLM-controlled: a prompt-injected page could ask
    // the agent to write the PDF to `~/.ssh/authorized_keys`, the user's
    // shell rc, etc. Run the same protected-paths gate `act.upload` uses
    // for the inverse (file → page) direction. The default path under
    // `share_dir()` skips the check because share_dir is by definition the
    // sandboxed write target.
    let output_path: PathBuf = if let Some(path) = get_str(args, "output_path") {
        browser::authorise_pdf_output_path(path)?
    } else {
        let share_dir = crate::paths::share_dir()?;
        std::fs::create_dir_all(&share_dir)?;
        let ts = chrono::Local::now().format("%Y%m%d_%H%M%S");
        share_dir.join(format!("page_{}.pdf", ts))
    };
    // `authorise_pdf_output_path` already created the parent for the
    // LLM-supplied path. The default branch above created `share_dir`.
    std::fs::write(&output_path, &bytes)?;
    Ok(format!(
        "PDF saved: {} ({} bytes)",
        output_path.display(),
        bytes.len()
    ))
}

// ── act ──────────────────────────────────────────────────────────────────

async fn action_act(args: &Value, session_id: Option<&str>) -> Result<String> {
    let kind_str = get_str(args, "kind").ok_or_else(|| anyhow!("act requires 'kind' parameter"))?;
    let kind = ActKind::parse(kind_str)
        .ok_or_else(|| anyhow!(
            "Unknown act.kind: '{}'. Valid: click / dblclick / fill / hover / drag / select / press / upload",
            kind_str
        ))?;
    let params = ActParams {
        ref_id: get_u32(args, "ref"),
        target_ref: get_u32(args, "target_ref"),
        // `text` uses `get_str_any` so `act.fill text=""` (clear input)
        // survives the empty-string-as-missing filter that `get_str`
        // applies to path-like params.
        text: get_str_any(args, "text").map(String::from),
        key: get_str(args, "key").map(String::from),
        file_path: get_str(args, "file_path").map(String::from),
        values: get_str_array(args, "values"),
    };
    let backend = acquire_browser_backend(session_id, "act").await?;
    // Frame emit happens at the tool_browser choke point (even on failure —
    // the page state may have changed: partial fill, click that did nothing).
    backend.act(kind, params).await
}

// ── observe ──────────────────────────────────────────────────────────────

async fn action_observe(args: &Value, session_id: Option<&str>) -> Result<String> {
    let kind_str = get_str(args, "kind").unwrap_or("console");
    let kind = match kind_str {
        "console" => ObserveKind::Console,
        "network" => ObserveKind::Network,
        "page_errors" | "errors" => ObserveKind::PageErrors,
        "downloads" | "download" => ObserveKind::Downloads,
        other => {
            return Err(anyhow!(
                "Unknown observe.kind: '{}'. Valid: console / network / page_errors / downloads",
                other
            ))
        }
    };
    let since = get_i64(args, "since");
    let backend = if kind == ObserveKind::Downloads {
        require_extension_tabs("observe.downloads", session_id).await?
    } else {
        acquire_browser_backend(session_id, "observe").await?
    };
    let entries = backend.observe(kind, since).await?;
    if entries.is_empty() {
        return Ok(format!(
            "No '{}' observations recorded yet. The buffer fills as the page runs scripts, makes network requests, or throws errors.",
            kind_str
        ));
    }
    let mut lines = Vec::with_capacity(entries.len() + 1);
    lines.push(format!(
        "Observed {} '{}' entries:",
        entries.len(),
        kind_str
    ));
    for e in &entries {
        let mut line = format!("[{}] {} {}", e.at, e.level, e.text);
        if let Some(url) = &e.url {
            line.push_str(&format!(" ({})", url));
        }
        lines.push(line);
    }
    Ok(lines.join("\n"))
}

// ── control ──────────────────────────────────────────────────────────────

async fn action_control(args: &Value, session_id: Option<&str>) -> Result<String> {
    let op = get_str(args, "op").ok_or_else(|| {
        anyhow!(
            "control requires 'op' (resize / scroll / wait_for / handle_dialog / evaluate / raw_cdp / download_cancel)"
        )
    })?;
    if op == "raw_cdp" {
        return control_raw_cdp(args, session_id).await;
    }
    if op == "download_cancel" {
        return control_download_cancel(args, session_id).await;
    }
    let backend = acquire_browser_backend(session_id, "control").await?;
    match op {
        "resize" => {
            let width = get_u32(args, "width")
                .ok_or_else(|| anyhow!("control.resize requires 'width'"))?;
            let height = get_u32(args, "height")
                .ok_or_else(|| anyhow!("control.resize requires 'height'"))?;
            backend.resize(width, height).await
        }
        "scroll" => {
            let direction = match get_str(args, "direction").unwrap_or("down") {
                "up" => ScrollDirection::Up,
                "down" => ScrollDirection::Down,
                "left" => ScrollDirection::Left,
                "right" => ScrollDirection::Right,
                other => {
                    return Err(anyhow!(
                        "Unknown scroll direction: '{}'. Use up/down/left/right",
                        other
                    ))
                }
            };
            let amount = get_i64(args, "amount").unwrap_or(500);
            backend.scroll(ScrollParams { direction, amount }).await
        }
        "wait_for" => {
            let text = get_str(args, "text").map(String::from);
            let timeout_ms = get_u64(args, "timeout").unwrap_or(30_000);
            backend.wait_for(WaitParams { text, timeout_ms }).await
        }
        "handle_dialog" => {
            let accept = get_bool(args, "accept").ok_or_else(|| {
                anyhow!("control.handle_dialog requires 'accept' (true/false)")
            })?;
            let action = if accept {
                DialogAction::Accept
            } else {
                DialogAction::Dismiss
            };
            let prompt = get_str(args, "dialog_text");
            backend.handle_dialog(action, prompt).await
        }
        "evaluate" => {
            let script = get_str(args, "expression")
                .or_else(|| get_str(args, "script"))
                .ok_or_else(|| anyhow!("control.evaluate requires 'expression' or 'script'"))?;
            evaluate_with_ssrf_scan(script).await?;
            let result = backend.evaluate(script).await?;
            let display = if result.is_string() {
                result.as_str().unwrap_or("").to_string()
            } else if result.is_null() {
                "undefined".to_string()
            } else {
                serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string())
            };
            Ok(format!("Result: {}", display))
        }
        other => Err(anyhow!(
            "Unknown control.op: '{}'. Valid: resize / scroll / wait_for / handle_dialog / evaluate / raw_cdp / download_cancel",
            other
        )),
    }
}

async fn control_download_cancel(args: &Value, session_id: Option<&str>) -> Result<String> {
    let download_id = get_i64(args, "download_id")
        .or_else(|| get_i64(args, "downloadId"))
        .or_else(|| get_i64(args, "id"))
        .ok_or_else(|| anyhow!("control.download_cancel requires 'download_id'"))?;
    if download_id < 0 {
        return Err(anyhow!(
            "control.download_cancel requires a non-negative download_id"
        ));
    }
    let backend = require_extension_tabs("control.download_cancel", session_id).await?;
    backend.cancel_download(download_id).await
}

async fn control_raw_cdp(args: &Value, session_id: Option<&str>) -> Result<String> {
    // Honor the kill switch: browser.extension.allowRawCdp = false disables the
    // raw CDP escape hatch entirely (defaults to enabled when unset).
    let raw_cdp_enabled = crate::config::cached_config()
        .browser
        .as_ref()
        .and_then(|b| b.extension.as_ref())
        .map_or(true, |ext| ext.allow_raw_cdp());
    if !raw_cdp_enabled {
        return Err(anyhow!(
            "control.raw_cdp is disabled by configuration (browser.extension.allowRawCdp = false)"
        ));
    }
    let method = get_str(args, "method")
        .ok_or_else(|| anyhow!("control.raw_cdp requires 'method'"))?
        .to_string();
    let params = args
        .get("params")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    if !params.is_object() {
        return Err(anyhow!("control.raw_cdp 'params' must be a JSON object"));
    }
    // Runtime.evaluate / Runtime.callFunctionOn execute arbitrary JS in the
    // page; apply the same SSRF scan that control.op=evaluate enforces so
    // raw_cdp can't be used to bypass the outbound URL policy.
    if matches!(
        method.as_str(),
        "Runtime.evaluate" | "Runtime.callFunctionOn"
    ) {
        if let Some(script) = params
            .get("expression")
            .or_else(|| params.get("functionDeclaration"))
            .and_then(Value::as_str)
        {
            evaluate_with_ssrf_scan(script).await?;
        }
    }
    // Page.navigate drives the real Chrome to an arbitrary URL; run the same
    // SSRF scan the curated `navigate` action enforces so raw_cdp can't reach
    // internal / metadata endpoints (e.g. 169.254.169.254) unchecked.
    if method == "Page.navigate" {
        if let Some(url) = params.get("url").and_then(Value::as_str) {
            check_url_via_ssrf(url).await?;
        }
    }
    let backend = require_extension_tabs("control.raw_cdp", session_id).await?;
    let result = backend
        .raw_cdp(RawCdpParams {
            method: method.clone(),
            params,
        })
        .await?;
    let display = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
    Ok(format!("Raw CDP `{method}` result:\n{display}"))
}

async fn acquire_cdp_backend() -> Result<std::sync::Arc<dyn BrowserBackend>> {
    acquire_backend_for(
        BrowserBackendContext::default(),
        BrowserBackendRequirement::CdpAllowed,
    )
    .await
}

fn browser_backend_context(session_id: Option<&str>, source: &str) -> BrowserBackendContext {
    BrowserBackendContext {
        session_id: session_id.map(ToString::to_string),
        source: Some(source.to_string()),
        ..BrowserBackendContext::default()
    }
}

async fn acquire_browser_backend(
    session_id: Option<&str>,
    source: &str,
) -> Result<std::sync::Arc<dyn BrowserBackend>> {
    acquire_backend_for(
        browser_backend_context(session_id, source),
        BrowserBackendRequirement::ExtensionPreferred,
    )
    .await
}

async fn require_extension_tabs(
    op: &str,
    session_id: Option<&str>,
) -> Result<std::sync::Arc<dyn BrowserBackend>> {
    acquire_backend_for(
        browser_backend_context(session_id, op),
        BrowserBackendRequirement::ExtensionRequired,
    )
    .await
}

/// Best-effort SSRF scan over a JS evaluation payload. Catches URL literals
/// inside `fetch("...")`, `import("...")`, `XMLHttpRequest().open(_, "...")`,
/// and `new URL("...")`. Anything that the SSRF policy rejects bubbles up as
/// an error so the backend never sees the script. Dynamic URL construction
/// (template literals, base64-encoded, `window.location.host`, etc.) is out
/// of scope by design — document this limitation in the skill.
async fn evaluate_with_ssrf_scan(script: &str) -> Result<()> {
    // URL schemes are case-insensitive in browsers (`HTTP://...` resolves), so
    // both the quick path and the regex use case-insensitive matching to
    // prevent a trivial bypass via uppercase.
    let lower = script.to_ascii_lowercase();
    if !lower.contains("http") {
        return Ok(());
    }
    let re = regex::Regex::new(r#"(?i)["'`](https?://[^"'`\s]+)["'`]"#)
        .expect("static regex must compile");
    let cfg = crate::config::cached_config();
    for cap in re.captures_iter(script) {
        let url = match cap.get(1) {
            Some(m) => m.as_str(),
            None => continue,
        };
        crate::security::ssrf::check_url(url, cfg.ssrf.browser(), &cfg.ssrf.trusted_hosts)
            .await
            .map_err(|e| {
                anyhow!(
                    "control.evaluate refused: URL literal '{}' rejected by SSRF policy ({}). \
                     Dynamic URL construction is not checked — keep that in mind.",
                    url,
                    e
                )
            })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn evaluate_ssrf_scan_blocks_uppercase_scheme() {
        // Uppercase HTTP:// resolves the same way in browsers; the scan must
        // not be bypassable by casing.
        let script = r#"fetch('HTTP://169.254.169.254/latest/meta-data/')"#;
        let res = evaluate_with_ssrf_scan(script).await;
        assert!(res.is_err(), "expected scan to block uppercase HTTP scheme");
    }

    #[tokio::test]
    async fn evaluate_ssrf_scan_blocks_metadata_url() {
        // cached_config() initialises lazy to defaults — Default policy blocks metadata.
        let script = r#"fetch("http://169.254.169.254/latest/meta-data/")"#;
        let res = evaluate_with_ssrf_scan(script).await;
        assert!(res.is_err(), "expected SSRF scan to block metadata URL");
    }

    #[tokio::test]
    async fn evaluate_ssrf_scan_allows_public_url() {
        let script = r#"fetch("https://1.1.1.1/")"#;
        let res = evaluate_with_ssrf_scan(script).await;
        assert!(res.is_ok(), "public URL must not be blocked: {res:?}");
    }

    #[tokio::test]
    async fn evaluate_ssrf_scan_skips_payloads_without_http() {
        let script = "document.title";
        assert!(evaluate_with_ssrf_scan(script).await.is_ok());
    }

    #[tokio::test]
    async fn download_cancel_rejects_negative_id_before_backend() {
        let args = serde_json::json!({
            "download_id": -1,
        });
        let res = control_download_cancel(&args, Some("test-session")).await;
        assert!(res.is_err());
        assert!(
            res.unwrap_err().to_string().contains("non-negative"),
            "negative download ids should fail during argument validation"
        );
    }

    #[test]
    fn tab_url_indicates_blank_load_for_empty_and_about_blank() {
        assert!(tab_url_indicates_blank_load(""));
        assert!(tab_url_indicates_blank_load("   "));
        assert!(tab_url_indicates_blank_load("about:blank"));
    }

    #[test]
    fn tab_url_indicates_blank_load_for_browser_newtab_urls() {
        // Chrome can hand back the browser's new-tab page instead of the
        // requested URL; treat those as blank loads so we navigate to the
        // target.
        assert!(tab_url_indicates_blank_load("chrome://newtab/"));
        assert!(tab_url_indicates_blank_load("chrome://new-tab-page/"));
        assert!(tab_url_indicates_blank_load("edge://newtab/"));
        assert!(tab_url_indicates_blank_load("data:,"));
    }

    #[test]
    fn tab_url_indicates_blank_load_false_for_redirected_url() {
        // If the server redirected (e.g. http→https, login gate, one-time
        // token), the tab url won't match the request — but it's loaded.
        // Re-navigating the original URL would consume the redirect
        // chain twice or break login flows, so this MUST stay false.
        assert!(!tab_url_indicates_blank_load("https://example.com/login"));
        assert!(!tab_url_indicates_blank_load(
            "https://example.com/auth?token=abc"
        ));
        assert!(!tab_url_indicates_blank_load("https://www.lingotech.xyz/"));
    }

    #[test]
    fn annotate_screenshot_draws_ref_boxes() {
        let mut image = image::RgbaImage::new(80, 60);
        for pixel in image.pixels_mut() {
            *pixel = image::Rgba([255, 255, 255, 255]);
        }
        let mut input = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut input, image::ImageFormat::Png)
            .unwrap();

        let mut attrs = std::collections::HashMap::new();
        attrs.insert("bounds".to_string(), "10,8,30,20".to_string());
        let snapshot = Snapshot {
            url: "https://example.test".to_string(),
            title: "Example".to_string(),
            viewport: (80, 60),
            elements: vec![crate::browser::backend::ElementRef {
                ref_id: 12,
                role: "button".to_string(),
                text: "Pay".to_string(),
                locator: "#pay".to_string(),
                depth: 0,
                attrs,
            }],
            truncated: false,
        };
        let input_bytes = input.into_inner();
        let (annotated, count) =
            annotate_screenshot_bytes(&input_bytes, ImageFormat::Png, &snapshot)
                .unwrap()
                .expect("annotated image");
        assert_eq!(count, 1);
        assert_ne!(annotated, input_bytes);
        assert!(image::load_from_memory(&annotated).is_ok());
    }

    #[test]
    fn formats_flat_session_diagnostics() {
        let diagnostics = serde_json::json!({
            "tabId": 7,
            "flatSessionEnabled": true,
            "frameTree": {
                "available": true,
                "frames": [
                    { "frameId": 0, "parentFrameId": -1, "url": "https://page.example/" },
                    {
                        "frameId": 17,
                        "parentFrameId": 0,
                        "url": "https://frame.example/",
                        "documentId": "doc-1"
                    }
                ]
            },
            "sessions": [
                {
                    "sessionId": "session-1",
                    "targetInfo": {
                        "type": "iframe",
                        "url": "https://frame.example/"
                    },
                    "matchedFrame": {
                        "status": "matched",
                        "frameId": 17,
                        "parentFrameId": 0,
                        "documentId": "doc-1",
                        "url": "https://frame.example/"
                    }
                }
            ]
        });
        let formatted = format_flat_session_diagnostics(&diagnostics).unwrap();
        assert!(formatted.contains("Flat sessions: 1 (enabled)"));
        assert!(formatted.contains("Frame tree: 2 frame(s)"));
        assert!(formatted.contains(
            "session-1 iframe https://frame.example/ frameId=17 parentFrameId=0 documentId=doc-1"
        ));
    }
}
