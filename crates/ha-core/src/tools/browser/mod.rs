//! Browser tool — collapsed 8-action surface.
//!
//! Top-level `action` selects one of:
//! - `status` — backend / connection / tab snapshot
//! - `profile` — launch / connect / disconnect / list managed profiles
//! - `tabs` — list / new / select / close
//! - `navigate` — go / back / forward / reload
//! - `snapshot` — role-based DOM tree / screenshot / pdf
//! - `act` — click / type / hover / drag / select / fill / press / upload
//! - `observe` — console / network / page_errors (ring buffer)
//! - `control` — resize / scroll / wait_for / handle_dialog / evaluate
//!
//! Each handler grabs the active [`crate::browser::BrowserBackend`] via
//! [`crate::browser::acquire_backend`] and formats a string result for the
//! LLM. SSRF checks for any URL field happen *before* the backend call so the
//! CDP layer never sees a URL that policy rejected.

use std::path::PathBuf;

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::agent::MEDIA_ITEMS_PREFIX;
use crate::attachments::{self, MediaItem, MediaKind};
use crate::browser::{
    self, acquire_backend, reset_backend, ActKind, ActParams, BrowserBackend, DialogAction,
    ImageFormat, ObserveKind, PdfParams, ScreenshotParams, ScrollDirection, ScrollParams,
    SnapshotFormat, WaitParams,
};
use crate::tools::image_markers;

/// Image base64 prefix marker — detected by `agent.rs` for multimodal content.
pub const IMAGE_BASE64_PREFIX: &str = "__IMAGE_BASE64__";

pub(crate) async fn tool_browser(args: &Value, session_id: Option<&str>) -> Result<String> {
    let action = args
        .get("action")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("Missing 'action' parameter"))?;

    match action {
        "status" => action_status(args).await,
        "profile" => action_profile(args, session_id).await,
        "tabs" => action_tabs(args).await,
        "navigate" => action_navigate(args).await,
        "snapshot" => action_snapshot(args, session_id).await,
        "act" => action_act(args).await,
        "observe" => action_observe(args).await,
        "control" => action_control(args, session_id).await,
        other => Err(anyhow!(
            "Unknown browser action: '{}'. Valid: status / profile / tabs / navigate / snapshot / act / observe / control",
            other
        )),
    }
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
    // We avoid forcing a backend creation here — `status` should be cheap and
    // honest about "not connected yet".
    let active = browser::peek_active().await;
    let Some(backend) = active else {
        return Ok(
            "Browser disconnected. Use `profile.op=launch` to start a managed Chrome, \
             or `profile.op=connect` to attach to an existing Chrome on a CDP port."
                .to_string(),
        );
    };
    let status = backend.status().await?;
    let mut out = format!(
        "Backend: {}\nConnected: {}\n",
        status.backend, status.connected
    );
    if let Some(active_id) = &status.active_target_id {
        out.push_str(&format!("Active tab: {}\n", active_id));
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
    let profiles_dir = crate::paths::browser_profiles_dir()?;
    if !profiles_dir.exists() {
        return Ok(
            "No browser profiles found. Use `profile.op=launch` with `profile=<name>` to create one."
                .to_string(),
        );
    }
    let mut profiles = Vec::new();
    for entry in std::fs::read_dir(&profiles_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            profiles.push(entry.file_name().to_string_lossy().to_string());
        }
    }
    if profiles.is_empty() {
        return Ok("No browser profiles found.".to_string());
    }
    profiles.sort();
    let active_profile = {
        let state = crate::browser_state::get_browser_state().lock().await;
        state.profile.clone()
    };
    let mut lines = vec![format!("Browser profiles ({}):", profiles.len())];
    for name in &profiles {
        let marker = if active_profile.as_deref() == Some(name.as_str()) {
            " [active]"
        } else {
            ""
        };
        lines.push(format!("  - {}{}", name, marker));
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
    let _ = acquire_backend().await?;

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
    let _ = acquire_backend().await?;

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
/// `async_capable=true` so the LLM can poll `job_status`.
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

async fn action_tabs(args: &Value) -> Result<String> {
    let op = get_str(args, "op")
        .ok_or_else(|| anyhow!("tabs requires 'op' parameter (list / new / select / close)"))?;

    match op {
        "list" => tabs_list().await,
        "new" => tabs_new(args).await,
        "select" => tabs_select(args).await,
        "close" => tabs_close(args).await,
        other => Err(anyhow!(
            "Unknown tabs.op: '{}'. Valid: list / new / select / close",
            other
        )),
    }
}

async fn tabs_list() -> Result<String> {
    let backend = acquire_backend().await?;
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

async fn tabs_new(args: &Value) -> Result<String> {
    let url = get_str(args, "url");
    if let Some(u) = url {
        if u != "about:blank" {
            check_url_via_ssrf(u).await?;
        }
    }
    let backend = acquire_backend().await?;
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
    browser::frame::emit_frame_async();
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

async fn tabs_select(args: &Value) -> Result<String> {
    let target = get_str(args, "target_id")
        .or_else(|| get_str(args, "page_id"))
        .ok_or_else(|| anyhow!("tabs.select requires 'target_id'"))?;
    let backend = acquire_backend().await?;
    backend.select_page(target).await?;
    browser::frame::emit_frame_async();
    Ok(format!("Switched to page: {}", target))
}

async fn tabs_close(args: &Value) -> Result<String> {
    let target = get_str(args, "target_id")
        .or_else(|| get_str(args, "page_id"))
        .ok_or_else(|| anyhow!("tabs.close requires 'target_id'"))?;
    let backend = acquire_backend().await?;
    backend.close_page(target).await?;
    Ok(format!("Page '{}' closed.", target))
}

// ── navigate ─────────────────────────────────────────────────────────────

async fn action_navigate(args: &Value) -> Result<String> {
    let op = get_str(args, "op").unwrap_or("go");
    let backend = acquire_backend().await?;
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
    if result.is_ok() {
        browser::frame::emit_frame_async();
    }
    result
}

// ── snapshot ─────────────────────────────────────────────────────────────

async fn action_snapshot(args: &Value, session_id: Option<&str>) -> Result<String> {
    let format = get_str(args, "format").unwrap_or("role");
    let backend = acquire_backend().await?;

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
        let mut line = format!("{}[ref={}] {}", indent, el.ref_id, el.role);
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
    let bytes = backend
        .take_screenshot(ScreenshotParams {
            format,
            full_page,
            quality: None,
            ref_id: None,
        })
        .await?;
    let mime = format.mime();
    let ext = format.extension();
    let display_filename = format!("browser_screenshot.{ext}");
    let caption = format!(
        "Screenshot captured (format: {}{})",
        ext,
        if full_page { ", full page" } else { "" }
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

async fn snapshot_pdf(args: &Value, backend: &dyn BrowserBackend) -> Result<String> {
    let bytes = backend
        .save_pdf(PdfParams {
            paper_format: get_str(args, "paper_format").map(String::from),
            landscape: get_bool(args, "landscape"),
            print_background: get_bool(args, "print_background"),
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

async fn action_act(args: &Value) -> Result<String> {
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
    let backend = acquire_backend().await?;
    let result = backend.act(kind, params).await;
    // Always emit a frame after an act attempt — even on failure the page
    // state may have changed (partial fill, click that did nothing, etc.).
    browser::frame::emit_frame_async();
    result
}

// ── observe ──────────────────────────────────────────────────────────────

async fn action_observe(args: &Value) -> Result<String> {
    let kind_str = get_str(args, "kind").unwrap_or("console");
    let kind = match kind_str {
        "console" => ObserveKind::Console,
        "network" => ObserveKind::Network,
        "page_errors" | "errors" => ObserveKind::PageErrors,
        other => {
            return Err(anyhow!(
                "Unknown observe.kind: '{}'. Valid: console / network / page_errors",
                other
            ))
        }
    };
    let since = get_i64(args, "since");
    let backend = acquire_backend().await?;
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
        anyhow!("control requires 'op' (resize / scroll / wait_for / handle_dialog / evaluate)")
    })?;
    let backend = acquire_backend().await?;
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
            confirm_evaluate(script, session_id).await?;
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
            "Unknown control.op: '{}'. Valid: resize / scroll / wait_for / handle_dialog / evaluate",
            other
        )),
    }
}

/// Gate `control.evaluate` behind an explicit user confirmation. Arbitrary
/// JS execution is the agent's most dangerous outbound surface (the SSRF
/// regex scan above is best-effort and won't catch dynamic URL
/// construction or `Function(...)` indirection). Bypassed for global YOLO
/// users, who have already accepted the trade-off.
const EVALUATE_AFFIRMATIVE_LABEL: &str = "Run it";

async fn confirm_evaluate(script: &str, session_id: Option<&str>) -> Result<()> {
    if crate::security::dangerous::is_dangerous_skip_active() {
        return Ok(());
    }
    let Some(sid) = session_id else {
        // Without a session_id we can't drive `ask_user_question`; deny by
        // default rather than silently running.
        return Err(anyhow!(
            "control.evaluate refused: no active session to confirm against. \
             Enable global YOLO mode if this call is from a non-interactive context."
        ));
    };
    // Truncate the script for the prompt — long bundles aren't useful in
    // a confirmation modal, but a non-empty head helps the user judge.
    let preview = {
        let s = script.trim();
        if s.chars().count() <= 280 {
            s.to_string()
        } else {
            let head: String = s.chars().take(277).collect();
            format!("{head}...")
        }
    };
    let ask_args = serde_json::json!({
        "context": "Browser control.evaluate is about to run arbitrary JavaScript in the active tab. \
                    Approve only if you trust the script.",
        "questions": [{
            "question_id": "confirm_browser_evaluate",
            "text": format!("Run this JavaScript in the browser?\n\n{preview}"),
            "header": "Browser evaluate",
            "options": [
                {"value": "confirm", "label": EVALUATE_AFFIRMATIVE_LABEL, "recommended": false},
                {"value": "cancel", "label": "Cancel", "recommended": true},
            ],
            "multi_select": false,
            "default_values": ["cancel"]
        }]
    });
    let raw = crate::tools::ask_user_question::execute(&ask_args, Some(sid)).await;
    if crate::ask_user::was_affirmative(&raw, &[EVALUATE_AFFIRMATIVE_LABEL]) {
        Ok(())
    } else {
        Err(anyhow!(
            "control.evaluate cancelled by user (or no response). \
             If this is a trusted automation, enable YOLO mode."
        ))
    }
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
        let script = r#"fetch("https://example.com/")"#;
        let res = evaluate_with_ssrf_scan(script).await;
        assert!(res.is_ok(), "public URL must not be blocked: {res:?}");
    }

    #[tokio::test]
    async fn evaluate_ssrf_scan_skips_payloads_without_http() {
        let script = "document.title";
        assert!(evaluate_with_ssrf_scan(script).await.is_ok());
    }

    // Affirmative-label parsing is covered by `crate::ask_user::was_affirmative`'s
    // own test suite. We don't re-test it here.

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
}
