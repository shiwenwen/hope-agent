use axum::extract::{Path, Query};
use axum::Json;
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextRetrievalQuery {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub template_id: Option<String>,
    #[serde(default)]
    pub template_version: Option<String>,
}

pub async fn get_context_retrieval(
    Path(session_id): Path<String>,
    Query(query): Query<ContextRetrievalQuery>,
) -> Result<Json<ha_core::context_retrieval::ContextRetrievalSnapshot>, AppError> {
    ha_core::context_retrieval::context_retrieval_for_session(
        session_db()?.clone(),
        session_id,
        ha_core::context_retrieval::ContextRetrievalInput {
            query: query.query,
            limit: query.limit,
            ide_context: None,
            domain: query.domain,
            template_id: query.template_id,
            template_version: query.template_version,
        },
    )
    .await
    .map(Json)
    .map_err(|e| AppError::bad_request(e.to_string()))
}
