//! Explicit-owner UI operations for the private assistant.
use axum::extract::Path;
use axum::{Extension, Json};
use ha_core::personal_assistant::{AssistantRequest, AssistantWatch};
use ha_core::session::SessionMeta;

use crate::error::AppError;
use crate::middleware::AuthState;
use crate::routes::helpers::session_db;

fn require_owner(auth: &AuthState) -> Result<(), AppError> {
    // The enclosing router authenticates the owner. A no-auth server does not
    // establish a private owner even though its regular API remains available.
    if !auth.auth_required() {
        return Err(AppError::forbidden(
            "Personal assistant access requires a Server Owner Token.",
        ));
    }
    Ok(())
}

pub async fn ensure_personal_assistant_home(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<SessionMeta>, AppError> {
    require_owner(&auth)?;
    Ok(Json(
        session_db()?
            .run(|db| db.ensure_personal_assistant_home())
            .await?,
    ))
}

pub async fn list_personal_assistant_requests(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<Vec<AssistantRequest>>, AppError> {
    require_owner(&auth)?;
    let db = session_db()?;
    let mut requests = db.run(|db| db.list_personal_assistant_requests()).await?;
    ha_core::personal_assistant::enrich_personal_assistant_requests(&mut requests, &db).await?;
    Ok(Json(requests))
}

pub async fn cancel_personal_assistant_request(
    Extension(auth): Extension<AuthState>,
    Path(id): Path<String>,
) -> Result<Json<AssistantRequest>, AppError> {
    require_owner(&auth)?;
    Ok(Json(
        ha_core::tools::cancel_personal_assistant_dispatch(session_db()?, id).await?,
    ))
}

pub async fn personal_assistant_inbox(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<Vec<AssistantWatch>>, AppError> {
    require_owner(&auth)?;
    Ok(Json(
        session_db()?
            .run(|db| db.personal_assistant_inbox())
            .await?,
    ))
}

pub async fn watch_personal_goal(
    Extension(auth): Extension<AuthState>,
    Path((session_id, goal_id)): Path<(String, String)>,
) -> Result<Json<AssistantWatch>, AppError> {
    require_owner(&auth)?;
    Ok(Json(
        session_db()?
            .run(move |db| db.watch_personal_goal(&session_id, &goal_id))
            .await?,
    ))
}

pub async fn unwatch_personal_goal(
    Extension(auth): Extension<AuthState>,
    Path((session_id, goal_id)): Path<(String, String)>,
) -> Result<Json<()>, AppError> {
    require_owner(&auth)?;
    session_db()?
        .run(move |db| db.unwatch_personal_goal(&session_id, &goal_id))
        .await?;
    Ok(Json(()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;

    #[tokio::test]
    async fn every_assistant_route_rejects_a_no_auth_server_before_database_access() {
        let auth = AuthState::new(None, None, false);
        assert_eq!(
            ensure_personal_assistant_home(Extension(auth.clone()))
                .await
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            list_personal_assistant_requests(Extension(auth.clone()))
                .await
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            personal_assistant_inbox(Extension(auth.clone()))
                .await
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            cancel_personal_assistant_request(Extension(auth.clone()), Path("request".into()))
                .await
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            watch_personal_goal(
                Extension(auth.clone()),
                Path(("session".into(), "goal".into()))
            )
            .await
            .unwrap_err()
            .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            unwatch_personal_goal(Extension(auth), Path(("session".into(), "goal".into())))
                .await
                .unwrap_err()
                .status,
            StatusCode::FORBIDDEN
        );
    }
}
