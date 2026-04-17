use anyhow::Result;
use serde_json::json;
use std::sync::Arc;

use super::events::emit_team_event;
use super::types::*;
use crate::session::SessionDB;

/// Return user-managed team templates from the DB.
///
/// Built-in templates were removed in favor of user-configured presets via the
/// Settings → Teams panel (see AGENTS.md). An empty vector means the user has
/// not configured any preset — callers should fall back to inline `members=[...]`.
pub fn all_templates(db: &Arc<SessionDB>) -> Vec<TeamTemplate> {
    db.list_team_templates().unwrap_or_default()
}

/// Persist a template and broadcast `template_saved` on the team EventBus.
///
/// The returned `TeamTemplate` carries the stored `created_at` / `updated_at`
/// so the caller can hand it straight back to the UI without a second query.
pub fn save_template(db: &SessionDB, template: TeamTemplate) -> Result<TeamTemplate> {
    let saved = db.insert_team_template(&template)?;
    emit_team_event("template_saved", &saved);
    Ok(saved)
}

/// Delete a template and broadcast `template_deleted` on the team EventBus.
pub fn delete_template(db: &SessionDB, template_id: &str) -> Result<()> {
    db.delete_team_template(template_id)?;
    emit_team_event("template_deleted", &json!({ "templateId": template_id }));
    Ok(())
}
