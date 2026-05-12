//! `app_update` tool — Agent-driven self-upgrade across all formfactors.
//!
//! Action matrix:
//!
//! | action     | side effects | gating                                |
//! | ---------- | ------------ | ------------------------------------- |
//! | `check`    | none         | none                                  |
//! | `install`  | binary swap, service restart | `ask_user_question` Yes/No |
//! | `status`   | none         | none                                  |
//! | `rollback` | binary swap, service restart | `ask_user_question` Yes/No |
//!
//! Confirmation lives inside the tool (not the permission engine) so the
//! Yes/No dialog can carry the full upgrade plan (current → target, route,
//! release notes summary) — generic `AskReason::EditTool` couldn't.
//!
//! Long-running `install` is detached onto an OS thread. The tool returns
//! a synthetic `{job_id, status: "started", ...}` immediately; the model
//! polls progress via `app_update(action="status", job_id=...)` and the
//! UI mirrors `app_update:progress` EventBus frames.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{json, Value};

use crate::updater::{self, RecommendedPath};

use super::ToolExecContext;

#[derive(Debug, Clone, Serialize)]
struct InstallJobState {
    job_id: String,
    started_at: i64,
    phase: String,
    target_version: Option<String>,
    path: String,
    completed_at: Option<i64>,
    outcome: Option<Value>,
    error: Option<String>,
}

fn tracker() -> &'static Mutex<HashMap<String, InstallJobState>> {
    static T: OnceLock<Mutex<HashMap<String, InstallJobState>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(HashMap::new()))
}

pub async fn tool_app_update(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let action = args.get("action").and_then(|v| v.as_str()).ok_or_else(|| {
        anyhow::anyhow!("`action` is required (check | install | status | rollback)")
    })?;

    match action {
        "check" => action_check().await,
        "install" => action_install(args, ctx).await,
        "status" => action_status(args).await,
        "rollback" => action_rollback(args, ctx).await,
        other => Err(anyhow::anyhow!(
            "unknown action '{other}' — expected check | install | status | rollback"
        )),
    }
}

async fn action_check() -> Result<String> {
    let outcome = updater::check_update().await?;
    Ok(serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".into()))
}

async fn action_status(args: &Value) -> Result<String> {
    let job_id = args
        .get("job_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("`job_id` is required for status"))?;
    let state = {
        let g = tracker().lock().unwrap_or_else(|p| p.into_inner());
        g.get(job_id).cloned()
    };
    match state {
        Some(s) => Ok(serde_json::to_string_pretty(&s).unwrap_or_else(|_| "{}".into())),
        None => Ok(json!({
            "job_id": job_id,
            "phase": "unknown",
            "note": "No tracked install job with this id. Either it completed and was pruned, the process restarted (in-memory tracker only), or the id is wrong."
        }).to_string()),
    }
}

async fn action_install(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let session_id = ctx.session_id.as_deref().ok_or_else(|| {
        anyhow::anyhow!("install requires a session context (cannot run from cron / one-shot CLI)")
    })?;

    let target_version = args
        .get("target_version")
        .and_then(|v| v.as_str())
        .map(|s| s.trim_start_matches('v').to_string());
    let prefer_path = args
        .get("prefer_path")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");

    let (snapshot, manifest) = updater::check_update_full().await?;
    if !snapshot.has_update && target_version.is_none() {
        return Ok(json!({
            "status": "no_update_available",
            "current_version": snapshot.current_version,
            "latest_version": snapshot.latest_version,
            "note": "Already on the latest version. Pass `target_version` to force-install a specific build."
        })
        .to_string());
    }

    // The release manifest only describes the latest tag. Allowing a
    // `target_version` that differs from `manifest.version` would silently
    // install the latest while reporting the pinned version — a worse
    // failure than refusing. Reject up-front and tell the user how to
    // get an arbitrary tag installed.
    if let Some(pinned) = target_version.as_deref() {
        let normalized_pin = pinned.trim_start_matches('v');
        let normalized_latest = snapshot.latest_version.trim_start_matches('v');
        if normalized_pin != normalized_latest {
            return Ok(json!({
                "status": "target_version_not_in_manifest",
                "current_version": snapshot.current_version,
                "latest_version": snapshot.latest_version,
                "requested_target": pinned,
                "note": format!(
                    "Pinned install to a specific tag is not yet wired up — the release manifest only describes the latest tag ({}). Either omit `target_version` to install the latest, or download the {} installer manually from https://github.com/shiwenwen/hope-agent/releases/tag/v{}.",
                    snapshot.latest_version, pinned, pinned
                )
            })
            .to_string());
        }
    }

    let chosen_path = resolve_path_override(prefer_path, snapshot.recommended_path)?;
    let to_version = target_version
        .clone()
        .unwrap_or_else(|| snapshot.latest_version.clone());

    if matches!(chosen_path, RecommendedPath::ManualPrompt) {
        return prompt_manual_install(session_id, &snapshot, &to_version).await;
    }
    if matches!(chosen_path, RecommendedPath::Tauri) && updater::get_updater_bridge().is_none() {
        // Desktop classified by source_detector but no bridge registered
        // (running e.g. `hope-agent server start` against the bundled app).
        // Fall back to the self-contained route — it shares the Minisign
        // root of trust and works headlessly.
        app_info!(
            "self_update",
            "install",
            "Tauri path recommended but no updater bridge registered — falling back to self_contained"
        );
        let confirm = ask_install_confirmation(
            session_id,
            &snapshot,
            &to_version,
            RecommendedPath::SelfContained,
        )
        .await?;
        if !confirm {
            return Ok(json!({
                "status": "cancelled_by_user",
                "note": "User declined the upgrade confirmation dialog (fallback path)."
            })
            .to_string());
        }
        return spawn_and_synthetic(
            to_version,
            RecommendedPath::SelfContained,
            snapshot.current_version,
            Some(manifest),
        );
    }

    let confirm = ask_install_confirmation(session_id, &snapshot, &to_version, chosen_path).await?;
    if !confirm {
        return Ok(json!({
            "status": "cancelled_by_user",
            "note": "User declined the upgrade confirmation dialog."
        })
        .to_string());
    }

    spawn_and_synthetic(
        to_version,
        chosen_path,
        snapshot.current_version,
        Some(manifest),
    )
}

fn spawn_and_synthetic(
    to_version: String,
    chosen_path: RecommendedPath,
    current_version: String,
    manifest: Option<updater::manifest::Manifest>,
) -> Result<String> {
    let job_id = format!("update_{}", uuid::Uuid::new_v4().simple());
    let now = now_secs();
    {
        let mut g = tracker().lock().unwrap_or_else(|p| p.into_inner());
        g.insert(
            job_id.clone(),
            InstallJobState {
                job_id: job_id.clone(),
                started_at: now,
                phase: "starting".into(),
                target_version: Some(to_version.clone()),
                path: path_label(chosen_path).into(),
                completed_at: None,
                outcome: None,
                error: None,
            },
        );
    }

    spawn_install_thread(job_id.clone(), to_version.clone(), chosen_path, manifest);

    Ok(json!({
        "job_id": job_id,
        "status": "started",
        "current_version": current_version,
        "target_version": to_version,
        "path": path_label(chosen_path),
        "hint": "Track progress via `app_update(action=\"status\", job_id=...)` or subscribe to the `app_update:progress` EventBus topic from the UI.",
    })
    .to_string())
}

async fn action_rollback(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("rollback requires a session context"))?;
    let _ = args; // no parameters beyond the action keyword today.

    if updater::backup::most_recent().is_none() {
        return Ok(json!({
            "status": "no_backup",
            "note": "No prior binary in ~/.hope-agent/updater/backup/ to roll back to."
        })
        .to_string());
    }

    let ask_args = json!({
        "context": "Hope Agent rollback — restore the previous binary from ~/.hope-agent/updater/backup/ and restart the service.",
        "questions": [{
            "question_id": "confirm_rollback",
            "text": "Roll back to the previously-installed Hope Agent binary? The service will restart immediately after the swap.",
            "header": "Hope Agent rollback",
            "options": [
                {"value": "confirm", "label": "Roll back now", "recommended": false},
                {"value": "cancel", "label": "Cancel", "recommended": true}
            ],
            "multi_select": false,
            "default_values": ["cancel"]
        }]
    });
    let raw_answer = super::ask_user_question::execute(&ask_args, Some(session_id)).await;
    if !is_confirm(&raw_answer) {
        return Ok(json!({
            "status": "cancelled_by_user",
            "note": "User declined the rollback confirmation dialog."
        })
        .to_string());
    }

    let job_id = format!("rollback_{}", uuid::Uuid::new_v4().simple());
    let outcome = updater::self_contained::rollback(&job_id)?;
    Ok(serde_json::to_string_pretty(&outcome).unwrap_or_else(|_| "{}".into()))
}

fn resolve_path_override(prefer: &str, recommended: RecommendedPath) -> Result<RecommendedPath> {
    Ok(match prefer {
        "auto" => recommended,
        "package_manager" => RecommendedPath::PackageManager,
        "self_contained" => RecommendedPath::SelfContained,
        other => anyhow::bail!(
            "invalid prefer_path '{other}' — expected auto | package_manager | self_contained"
        ),
    })
}

fn path_label(path: RecommendedPath) -> &'static str {
    match path {
        RecommendedPath::Tauri => "tauri",
        RecommendedPath::PackageManager => "package_manager",
        RecommendedPath::SelfContained => "self_contained",
        RecommendedPath::ManualPrompt => "manual_prompt",
    }
}

async fn ask_install_confirmation(
    session_id: &str,
    snapshot: &updater::CheckOutcome,
    to_version: &str,
    path: RecommendedPath,
) -> Result<bool> {
    let route_desc = match path {
        RecommendedPath::PackageManager => format!(
            "Route: package manager ({})",
            snapshot.install_source.label()
        ),
        RecommendedPath::SelfContained => {
            "Route: download bare binary, verify Minisign signature, atomically swap.".into()
        }
        RecommendedPath::Tauri => "Route: Tauri bundled updater (signed installer).".into(),
        RecommendedPath::ManualPrompt => {
            "Route: manual download (no automated path matches).".into()
        }
    };
    let notes_line = snapshot
        .notes
        .as_deref()
        .map(|n| {
            format!(
                "\n\nRelease notes (excerpt):\n{}",
                crate::truncate_utf8(n, 512)
            )
        })
        .unwrap_or_default();
    let text = format!(
        "Upgrade Hope Agent {} → {}?\n{}{}\n\nThe user-level service will restart immediately after the binary swap (typically 1-2 seconds of downtime). Any in-flight chat turn, cron job, or IM stream will be cancelled — pause non-trivial work first.",
        snapshot.current_version, to_version, route_desc, notes_line
    );
    let ask_args = json!({
        "context": format!(
            "Hope Agent self-update confirmation. Install source: {}. Recommended path: {}.",
            snapshot.install_source.label(),
            path_label(path),
        ),
        "questions": [{
            "question_id": "confirm_install",
            "text": text,
            "header": format!("Hope Agent {} → {}", snapshot.current_version, to_version),
            "options": [
                {"value": "confirm", "label": "Upgrade now", "recommended": false},
                {"value": "cancel", "label": "Not now", "recommended": true}
            ],
            "multi_select": false,
            "default_values": ["cancel"]
        }]
    });
    let raw = super::ask_user_question::execute(&ask_args, Some(session_id)).await;
    Ok(is_confirm(&raw))
}

async fn prompt_manual_install(
    session_id: &str,
    snapshot: &updater::CheckOutcome,
    to_version: &str,
) -> Result<String> {
    // No automated path applies — surface the gap to the user via a
    // structured prompt so they can pick a recovery option (point us at a
    // package manager, download manually, abort). The model never tries
    // to recover this itself — wrong move could break the install.
    let ask_args = json!({
        "context": "Hope Agent cannot pick an automated upgrade path for this install. Decide how to proceed.",
        "questions": [{
            "question_id": "manual_install_route",
            "text": format!(
                "No automated upgrade path is available for Hope Agent {} → {}. Install source detected as: {}. Pick a recovery:",
                snapshot.current_version, to_version, snapshot.install_source.label()
            ),
            "header": "Manual upgrade required",
            "options": [
                {"value": "open_releases", "label": "Open release page in browser (manual download)", "recommended": true},
                {"value": "force_self_contained", "label": "Try the self-contained bare-binary swap anyway"},
                {"value": "abort", "label": "Abort upgrade"}
            ],
            "multi_select": false,
            "default_values": ["open_releases"]
        }]
    });
    let raw = super::ask_user_question::execute(&ask_args, Some(session_id)).await;
    Ok(json!({
        "status": "manual_prompt_emitted",
        "user_response": raw,
        "next_step_hint": "If user picked `force_self_contained`, re-invoke with `prefer_path: \"self_contained\"`. If they picked `open_releases`, do nothing — the user will install manually."
    })
    .to_string())
}

fn spawn_install_thread(
    job_id: String,
    to_version: String,
    path: RecommendedPath,
    manifest: Option<updater::manifest::Manifest>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                finalize_failed(&job_id, format!("runtime build failed: {e}"));
                return;
            }
        };
        rt.block_on(async move {
            update_phase(&job_id, "running");
            let result = match path {
                RecommendedPath::SelfContained => {
                    run_self_contained(&job_id, &to_version, manifest).await
                }
                RecommendedPath::PackageManager => run_package_manager(&job_id).await,
                RecommendedPath::Tauri => run_tauri_bridge(&job_id).await,
                _ => Err(anyhow::anyhow!(
                    "internal: spawn_install_thread reached with non-spawnable path {:?}",
                    path
                )),
            };
            match result {
                Ok(v) => finalize_ok(&job_id, v),
                Err(e) => finalize_failed(&job_id, e.to_string()),
            }
        });
    });
}

async fn run_self_contained(
    job_id: &str,
    to_version: &str,
    manifest: Option<updater::manifest::Manifest>,
) -> Result<Value> {
    let outcome = updater::self_contained::install(job_id, Some(to_version), manifest).await?;
    serde_json::to_value(&outcome).context("serialize install outcome")
}

async fn run_tauri_bridge(job_id: &str) -> Result<Value> {
    let bridge = updater::get_updater_bridge().ok_or_else(|| {
        anyhow::anyhow!(
            "Tauri path requested but no updater bridge registered — call `updater::set_updater_bridge` from src-tauri startup"
        )
    })?;
    updater::self_contained::emit_phase(job_id, updater::self_contained::Phase::Downloading);
    let summary = bridge.install_and_restart(job_id).await?;
    updater::self_contained::emit_phase(job_id, updater::self_contained::Phase::Done);
    Ok(json!({"path": "tauri", "summary": summary}))
}

async fn run_package_manager(job_id: &str) -> Result<Value> {
    updater::self_contained::emit_phase(job_id, updater::self_contained::Phase::Downloading);
    let source = updater::source_detector::detect_install_source();
    let outcome = updater::package_manager::upgrade(&source)?;
    if !outcome.success {
        anyhow::bail!(
            "package manager upgrade failed: {}\nstderr: {}",
            outcome.command,
            crate::truncate_utf8(&outcome.stderr, 1024)
        );
    }
    updater::self_contained::emit_phase(job_id, updater::self_contained::Phase::Restarting);
    let restart = updater::service_control::restart_service().ok();
    updater::self_contained::emit_phase(job_id, updater::self_contained::Phase::Done);
    Ok(json!({
        "path": "package_manager",
        "command": outcome.command,
        "stdout": crate::truncate_utf8(&outcome.stdout, 4096),
        "stderr": crate::truncate_utf8(&outcome.stderr, 4096),
        "service_restart": restart,
    }))
}

fn update_phase(job_id: &str, phase: &str) {
    let mut g = tracker().lock().unwrap_or_else(|p| p.into_inner());
    if let Some(s) = g.get_mut(job_id) {
        s.phase = phase.into();
    }
}

fn finalize_ok(job_id: &str, outcome: Value) {
    let now = now_secs();
    {
        let mut g = tracker().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.get_mut(job_id) {
            s.phase = "done".into();
            s.completed_at = Some(now);
            s.outcome = Some(outcome.clone());
        }
        prune_completed_locked(&mut g, now);
    }
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "app_update:completed",
            json!({"job_id": job_id, "status": "done", "outcome": outcome}),
        );
    }
}

fn finalize_failed(job_id: &str, err: String) {
    let now = now_secs();
    {
        let mut g = tracker().lock().unwrap_or_else(|p| p.into_inner());
        if let Some(s) = g.get_mut(job_id) {
            s.phase = "failed".into();
            s.completed_at = Some(now);
            s.error = Some(err.clone());
        }
        prune_completed_locked(&mut g, now);
    }
    app_warn!("self_update", "install", "job {} failed: {}", job_id, err);
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "app_update:completed",
            json!({"job_id": job_id, "status": "failed", "error": err}),
        );
    }
}

/// Drop completed/failed entries older than 24 h so a long-running daemon
/// that's been upgraded many times doesn't accumulate stale tracker rows.
/// Called under the tracker mutex (`_locked` suffix) so we don't double-lock.
fn prune_completed_locked(map: &mut HashMap<String, InstallJobState>, now: i64) {
    const TTL_SECS: i64 = 24 * 3600;
    map.retain(|_, s| match s.completed_at {
        Some(t) => now.saturating_sub(t) < TTL_SECS,
        None => true,
    });
}

/// Exact match against the two affirmative labels declared in the
/// `confirm_install` / `confirm_rollback` schemas in this file. Both ends
/// of the contract are controlled here, so this is intentionally rigid:
/// any future label edit must also touch this list.
const AFFIRMATIVE_LABELS: &[&str] = &["upgrade now", "roll back now"];

fn is_confirm(raw_answer: &str) -> bool {
    super::ask_user_question::answer_matches_any(raw_answer, AFFIRMATIVE_LABELS)
}

fn now_secs() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_confirm_picks_up_upgrade_now_label() {
        let raw = r#"{"answers":[{"question":"Upgrade?","selected":["Upgrade now"],"customInput":null}]}"#;
        assert!(is_confirm(raw));
    }

    #[test]
    fn is_confirm_rejects_not_now() {
        let raw =
            r#"{"answers":[{"question":"Upgrade?","selected":["Not now"],"customInput":null}]}"#;
        assert!(!is_confirm(raw));
    }

    #[test]
    fn is_confirm_rejects_cancellation_message() {
        // ask_user returns a plain string on user-cancel — must not be
        // mistaken for confirmation.
        assert!(!is_confirm(
            "The user cancelled the questions without answering."
        ));
    }

    #[test]
    fn is_confirm_picks_up_rollback_label() {
        let raw = r#"{"answers":[{"question":"Rollback?","selected":["Roll back now"],"customInput":null}]}"#;
        assert!(is_confirm(raw));
    }

    #[test]
    fn resolve_path_override_accepts_force_self_contained() {
        let p = resolve_path_override("self_contained", RecommendedPath::PackageManager).unwrap();
        assert_eq!(p, RecommendedPath::SelfContained);
    }

    #[test]
    fn resolve_path_override_auto_keeps_recommendation() {
        let p = resolve_path_override("auto", RecommendedPath::SelfContained).unwrap();
        assert_eq!(p, RecommendedPath::SelfContained);
    }

    #[test]
    fn resolve_path_override_rejects_unknown() {
        assert!(resolve_path_override("yolo", RecommendedPath::SelfContained).is_err());
    }
}
