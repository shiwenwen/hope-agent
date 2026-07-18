//! REST route for the panel action timeline (in-memory `tool_actions` ring).

use axum::extract::Query;
use axum::Json;
use ha_core::tool_actions::{self, ToolActionRecord, ToolActionSource};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentActionsQuery {
    pub source: Option<String>,
    pub session_id: Option<String>,
    pub limit: Option<usize>,
}

/// `GET /api/tool-actions`
pub async fn recent(Query(query): Query<RecentActionsQuery>) -> Json<Vec<ToolActionRecord>> {
    let source = query.source.as_deref().and_then(ToolActionSource::parse);
    Json(tool_actions::recent(
        source,
        query.session_id.as_deref(),
        query.limit.unwrap_or(200),
    ))
}
