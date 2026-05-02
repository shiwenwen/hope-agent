use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

use super::file_io::plan_file_path;
use super::types::{PlanMeta, PlanModeState};

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

pub async fn set_plan_state(session_id: &str, state: PlanModeState) -> bool {
    let mut map = store().write().await;
    if state == PlanModeState::Off {
        map.remove(session_id);
        true
    } else if let Some(meta) = map.get_mut(session_id) {
        // Reject illegal transitions to keep concurrent writers from skipping
        // the review checkpoint.
        if !meta.state.is_valid_transition(&state) {
            app_warn!(
                "plan",
                "state",
                "Rejecting illegal plan transition for session {}: {} -> {}",
                session_id,
                meta.state.as_str(),
                state.as_str()
            );
            return false;
        }
        meta.state = state;
        meta.updated_at = chrono::Utc::now().to_rfc3339();
        true
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
                created_at: chrono::Utc::now().to_rfc3339(),
                updated_at: chrono::Utc::now().to_rfc3339(),
                version: 1,
                checkpoint_ref: None,
                executing_started_at: None,
            },
        );
        true
    }
}

pub fn should_create_execution_checkpoint(
    requested_state: &PlanModeState,
    previous_state: &PlanModeState,
    persisted_plan_mode: Option<PlanModeState>,
    checkpoint_exists: bool,
) -> bool {
    if requested_state != &PlanModeState::Executing || checkpoint_exists {
        return false;
    }
    if matches!(previous_state, PlanModeState::Executing) {
        return false;
    }
    !matches!(persisted_plan_mode, Some(PlanModeState::Executing))
}

pub async fn get_plan_meta(session_id: &str) -> Option<PlanMeta> {
    let map = store().read().await;
    map.get(session_id).cloned()
}

/// Restore plan state from DB on session load.
pub async fn restore_from_db(session_id: &str, state: PlanModeState) {
    if state == PlanModeState::Off {
        return;
    }
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
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            version: 1,
            checkpoint_ref: None,
            executing_started_at: None,
        },
    );
}
