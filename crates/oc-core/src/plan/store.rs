use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use super::file_io::{load_plan_file, plan_file_path};
use super::parser::parse_plan_steps;
use super::types::{PlanMeta, PlanModeState, PlanStep, PlanStepStatus};

// ── Global Per-Session Store ────────────────────────────────────

static PLAN_STORE: OnceLock<Arc<RwLock<HashMap<String, PlanMeta>>>> = OnceLock::new();

pub fn store() -> &'static Arc<RwLock<HashMap<String, PlanMeta>>> {
    PLAN_STORE.get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
}

pub async fn get_plan_state(session_id: &str) -> PlanModeState {
    let map = store().read().await;
    map.get(session_id)
        .map(|m| m.state.clone())
        .unwrap_or(PlanModeState::Off)
}

pub async fn set_plan_state(session_id: &str, state: PlanModeState) {
    let mut map = store().write().await;
    if state == PlanModeState::Off {
        map.remove(session_id);
    } else if let Some(meta) = map.get_mut(session_id) {
        // Record paused_at_step when transitioning to Paused
        if state == PlanModeState::Paused {
            // Find the first in_progress step, or the first pending step
            let paused_at = meta
                .steps
                .iter()
                .position(|s| s.status == PlanStepStatus::InProgress)
                .or_else(|| {
                    meta.steps
                        .iter()
                        .position(|s| s.status == PlanStepStatus::Pending)
                });
            meta.paused_at_step = paused_at;
        } else if state == PlanModeState::Executing {
            // Clear paused_at_step when resuming
            meta.paused_at_step = None;
        }
        meta.state = state;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
    } else {
        // Create a new PlanMeta entry
        let file_path = plan_file_path(session_id)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        map.insert(
            session_id.to_string(),
            PlanMeta {
                session_id: session_id.to_string(),
                title: None,
                file_path,
                state,
                steps: Vec::new(),
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                paused_at_step: None,
                version: 1,
                checkpoint_ref: None,
            },
        );
    }
}

pub async fn get_plan_meta(session_id: &str) -> Option<PlanMeta> {
    let map = store().read().await;
    map.get(session_id).cloned()
}

pub async fn update_plan_steps(session_id: &str, steps: Vec<PlanStep>) {
    let mut map = store().write().await;
    if let Some(meta) = map.get_mut(session_id) {
        meta.steps = steps.clone();
        meta.updated_at = chrono::Utc::now().to_rfc3339();
    }
    drop(map);
    // Persist to DB for crash recovery
    persist_steps_to_db(session_id, &steps);
}

pub async fn update_step_status(
    session_id: &str,
    step_index: usize,
    status: PlanStepStatus,
    duration_ms: Option<u64>,
) {
    let steps_snapshot;
    {
        let mut map = store().write().await;
        if let Some(meta) = map.get_mut(session_id) {
            if let Some(step) = meta.steps.get_mut(step_index) {
                step.status = status;
                if duration_ms.is_some() {
                    step.duration_ms = duration_ms;
                }
                meta.updated_at = chrono::Utc::now().to_rfc3339();
            }
            steps_snapshot = Some(meta.steps.clone());
        } else {
            steps_snapshot = None;
        }
    }
    // Persist step statuses to DB for crash recovery
    if let Some(steps) = steps_snapshot {
        persist_steps_to_db(session_id, &steps);
    }
}

/// Persist plan steps to DB as JSON (fire-and-forget, non-blocking).
fn persist_steps_to_db(session_id: &str, steps: &[PlanStep]) {
    if let Ok(json) = serde_json::to_string(steps) {
        if let Some(db) = crate::get_session_db() {
            let _ = db.save_plan_steps(session_id, &json);
        }
    }
}

/// Restore plan state from DB on session load.
/// First tries to load persisted step statuses from DB (crash-safe),
/// then falls back to re-parsing the plan markdown file.
pub async fn restore_from_db(session_id: &str, plan_mode_str: &str) {
    let state = PlanModeState::from_str(plan_mode_str);
    if state == PlanModeState::Off {
        return;
    }

    // Try loading persisted step statuses from DB first (crash recovery)
    let steps = if let Some(db) = crate::get_session_db() {
        if let Ok(Some(json)) = db.load_plan_steps(session_id) {
            serde_json::from_str::<Vec<PlanStep>>(&json).unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };

    // Fallback: if DB had no steps, re-parse from plan file
    let steps = if steps.is_empty() {
        match load_plan_file(session_id) {
            Ok(Some(content)) => parse_plan_steps(&content),
            _ => Vec::new(),
        }
    } else {
        steps
    };

    let file_path = plan_file_path(session_id)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut map = store().write().await;
    map.insert(
        session_id.to_string(),
        PlanMeta {
            session_id: session_id.to_string(),
            title: None,
            file_path,
            state,
            steps,
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            paused_at_step: None,
            version: 1,
            checkpoint_ref: None,
        },
    );
}
