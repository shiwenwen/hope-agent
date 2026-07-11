use axum::extract::Path;
use axum::Json;
use ha_core::session::SessionIdeContext;
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::helpers::session_db;

pub async fn get_session_ide_context(
    Path(session_id): Path<String>,
) -> Result<Json<Option<ha_core::session::SessionIdeContextSnapshot>>, AppError> {
    let db = session_db()?;
    Ok(Json(
        db.run(move |db| db.get_session_ide_context(&session_id))
            .await?,
    ))
}

#[derive(Debug, Deserialize)]
pub struct SaveSessionIdeContextBody {
    pub context: SessionIdeContext,
}

pub async fn save_session_ide_context(
    Path(session_id): Path<String>,
    Json(body): Json<SaveSessionIdeContextBody>,
) -> Result<Json<ha_core::session::SessionIdeContextSnapshot>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.save_session_ide_context(&session_id, body.context))
        .await
        .map(Json)
        .map_err(|e| AppError::bad_request(e.to_string()))
}

pub async fn clear_session_ide_context(
    Path(session_id): Path<String>,
) -> Result<Json<()>, AppError> {
    let db = session_db()?;
    db.run(move |db| db.clear_session_ide_context(&session_id))
        .await?;
    Ok(Json(()))
}
