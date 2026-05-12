//! `app_restart` tool — restart the running Hope Agent process across
//! desktop / installed-service / foreground-server formfactors.
//!
//! Routing happens inside [`crate::lifecycle::route`]:
//!
//! | mode                                    | route       |
//! | --------------------------------------- | ----------- |
//! | desktop (Tauri GUI)                     | exit(42) + guardian respawn |
//! | server, `service_install::is_service_installed()` | OS supervisor restart |
//! | server, foreground (no system service)  | detach respawn + self-exit |
//! | acp / unknown                           | refuse — IDE owns lifetime |
//!
//! Confirmation lives in the tool, not the permission engine:
//!   1. Optional **pre-flight**: if `collect_inflight` reports any chat turns
//!      / async tool jobs / running cron jobs, ask the user to acknowledge
//!      the interruption first.
//!   2. **Standard Yes/No**: a second `ask_user_question` carries the route
//!      label so the user sees exactly what restart strategy will run.
//!
//! Both confirmations are mandatory — neither Plan nor YOLO can suppress
//! them. Returns immediately after handing off; the process may die before
//! the model sees the result.

use anyhow::Result;
use serde_json::{json, Value};

use crate::lifecycle::{self, InflightSummary, Route};

use super::ToolExecContext;

pub async fn tool_app_restart(args: &Value, ctx: &ToolExecContext) -> Result<String> {
    // `action` is reserved for future verbs (stop / start). Today we only
    // accept "restart" or omit it entirely.
    if let Some(action) = args.get("action").and_then(|v| v.as_str()) {
        if action != "restart" {
            return Ok(json!({
                "status": "unsupported_action",
                "note": format!("'{action}' is not supported — only 'restart' (or omit the field) is recognized today"),
            })
            .to_string());
        }
    }

    let session_id = ctx
        .session_id
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("app_restart requires a session context"))?;

    let route = lifecycle::route();
    if route == Route::Unsupported {
        return Ok(json!({
            "status": "unsupported_mode",
            "runtime_role": crate::app_init::runtime_role().unwrap_or("unknown"),
            "note": "Restart is not available for this runtime (ACP / unknown). The IDE / launcher owns the process lifetime here.",
        })
        .to_string());
    }

    let inflight = lifecycle::collect_inflight();

    // ── 1. Pre-flight: only if there's something to warn about ─────
    if !inflight.is_empty() && !ask_inflight_ack(session_id, &inflight).await? {
        return Ok(json!({
            "status": "cancelled_by_user",
            "phase": "preflight",
            "inflight_items": inflight.items,
            "note": "User declined to proceed after seeing in-flight work that would be interrupted.",
        })
        .to_string());
    }

    // ── 2. Standard Yes/No ─────────────────────────────────────────
    if !ask_restart_confirmation(session_id, route, &inflight).await? {
        return Ok(json!({
            "status": "cancelled_by_user",
            "phase": "confirmation",
            "note": "User declined the restart confirmation dialog.",
        })
        .to_string());
    }

    app_info!(
        "lifecycle",
        "restart",
        "user-confirmed restart (route={}, inflight_items={})",
        route.as_str(),
        inflight.len()
    );

    // Hand off. `lifecycle::restart` returns once the OS-level supervisor
    // has been told to swap us out (or our own self-exit has been
    // scheduled). Don't block past this point — by the time the model
    // receives this string, the process may already be on its way out.
    match lifecycle::restart() {
        Ok(outcome) => Ok(json!({
            "status": "restart_initiated",
            "route": outcome.route.as_str(),
            "detail": outcome.detail,
            "pid_at_handoff": std::process::id(),
        })
        .to_string()),
        Err(e) => Ok(json!({
            "status": "failed",
            "route": route.as_str(),
            "error": e.to_string(),
        })
        .to_string()),
    }
}

async fn ask_inflight_ack(session_id: &str, inflight: &InflightSummary) -> Result<bool> {
    let bullet_list: String = inflight
        .items
        .iter()
        .take(8) // truncate for prompt sanity; user can still see full list via the tool result
        .map(|it| format!("• {} — {}", it.kind, it.label))
        .collect::<Vec<_>>()
        .join("\n");
    let overflow = if inflight.len() > 8 {
        format!("\n… and {} more", inflight.len() - 8)
    } else {
        String::new()
    };

    let ask_args = json!({
        "context": format!(
            "Restarting now will interrupt {} in-flight item(s):\n\n{}{}",
            inflight.len(),
            bullet_list,
            overflow,
        ),
        "questions": [{
            "question_id": "preflight_ack",
            "text": "These items will be interrupted. Continue anyway?",
            "header": "Hope Agent restart — in-flight work",
            "options": [
                {"value": "continue", "label": "Continue anyway", "recommended": false},
                {"value": "cancel", "label": "Cancel restart", "recommended": true}
            ],
            "multi_select": false,
            "default_values": ["cancel"]
        }]
    });
    let raw = super::ask_user_question::execute(&ask_args, Some(session_id)).await;
    Ok(super::ask_user_question::answer_matches_any(
        &raw,
        &["continue anyway"],
    ))
}

async fn ask_restart_confirmation(
    session_id: &str,
    route: Route,
    inflight: &InflightSummary,
) -> Result<bool> {
    let inflight_line = if inflight.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n{} item(s) will be interrupted (you acknowledged this).",
            inflight.len()
        )
    };
    let context = format!(
        "Restart route: {}{}\n\nThe process will terminate after handoff and the new instance will reconnect on its own.",
        lifecycle::route_label(route),
        inflight_line,
    );

    let ask_args = json!({
        "context": context,
        "questions": [{
            "question_id": "confirm_restart",
            "text": "Restart Hope Agent now?",
            "header": "Hope Agent restart",
            "options": [
                {"value": "confirm", "label": "Restart now", "recommended": false},
                {"value": "cancel", "label": "Cancel", "recommended": true}
            ],
            "multi_select": false,
            "default_values": ["cancel"]
        }]
    });
    let raw = super::ask_user_question::execute(&ask_args, Some(session_id)).await;
    Ok(super::ask_user_question::answer_matches_any(
        &raw,
        &["restart now"],
    ))
}
