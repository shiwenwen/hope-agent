use crate::commands::CmdError;
use crate::plan::{self, PlanIndexEntry, PlanIndexFilter, PlanMentionResolution};

#[tauri::command]
pub async fn list_plans(filter: Option<PlanIndexFilter>) -> Result<Vec<PlanIndexEntry>, CmdError> {
    let filter = filter.unwrap_or_default();
    plan::list_all_plans(&filter).map_err(Into::into)
}

#[tauri::command]
pub async fn resolve_plan_mention(
    short_id: String,
    version: Option<u32>,
) -> Result<PlanMentionResolution, CmdError> {
    plan::resolve_plan_mention(&short_id, version.unwrap_or(0)).map_err(Into::into)
}
