use axum::extract::Path;
use axum::http::HeaderMap;
use axum::{Extension, Json};
use ha_channel::personal_assistant_im as service;
use ha_core::personal_assistant_im::{AssistantImError, ConnectionView, PairingCode, SessionGrant};
use serde::Deserialize;

use crate::error::AppError;
use crate::middleware::AuthState;
use crate::routes::helpers::session_db;

fn require_owner(auth: &AuthState, headers: &HeaderMap) -> Result<(), AppError> {
    if !auth.auth_required() || !auth.headers_are_owner_authenticated(headers) {
        return Err(AppError::forbidden(
            "Personal assistant IM requires authenticated owner access.",
        ));
    }
    Ok(())
}

fn api_error(error: anyhow::Error) -> AppError {
    match error.downcast_ref::<AssistantImError>() {
        Some(AssistantImError::IdentityInUse) => {
            AppError::conflict_with_code("assistant_im_identity_in_use", error.to_string())
        }
        Some(AssistantImError::StaleGeneration) => {
            AppError::conflict_with_code("assistant_im_stale_generation", error.to_string())
        }
        Some(AssistantImError::PermissionDenied) => AppError::forbidden(error.to_string()),
        Some(AssistantImError::InvalidInput) => AppError::bad_request(error.to_string()),
        _ => AppError::internal(error.to_string()),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigureInput {
    token: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmInput {
    generation: i64,
    user_id: i64,
    chat_id: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GrantInput {
    input: SessionGrant,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PauseInput {
    paused: bool,
}

pub async fn personal_assistant_im_status(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
) -> Result<Json<service::AssistantImStatus>, AppError> {
    require_owner(&auth, &headers)?;
    service::status(session_db()?.clone())
        .await
        .map(Json)
        .map_err(api_error)
}

pub async fn personal_assistant_im_configure(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Json(input): Json<ConfigureInput>,
) -> Result<Json<ConnectionView>, AppError> {
    require_owner(&auth, &headers)?;
    service::configure(session_db()?.clone(), input.token)
        .await
        .map(Json)
        .map_err(api_error)
}

pub async fn personal_assistant_im_begin_pairing(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
) -> Result<Json<PairingCode>, AppError> {
    require_owner(&auth, &headers)?;
    service::begin_pairing(session_db()?.clone())
        .await
        .map(Json)
        .map_err(api_error)
}

pub async fn personal_assistant_im_confirm_pairing(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Json(input): Json<ConfirmInput>,
) -> Result<Json<()>, AppError> {
    require_owner(&auth, &headers)?;
    service::confirm_pairing(
        session_db()?.clone(),
        input.generation,
        input.user_id,
        input.chat_id,
    )
    .await
    .map(Json)
    .map_err(api_error)
}

pub async fn personal_assistant_im_set_grant(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Json(input): Json<GrantInput>,
) -> Result<Json<SessionGrant>, AppError> {
    require_owner(&auth, &headers)?;
    service::set_grant(session_db()?.clone(), input.input)
        .await
        .map(Json)
        .map_err(api_error)
}

pub async fn personal_assistant_im_revoke_grant(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Path(session_id): Path<String>,
) -> Result<Json<()>, AppError> {
    require_owner(&auth, &headers)?;
    service::revoke_grant(session_db()?.clone(), session_id)
        .await
        .map(Json)
        .map_err(api_error)
}

pub async fn personal_assistant_im_revoke_pairing(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
) -> Result<Json<()>, AppError> {
    require_owner(&auth, &headers)?;
    service::revoke_pairing(session_db()?.clone())
        .await
        .map(Json)
        .map_err(api_error)
}

pub async fn personal_assistant_im_remove_connection(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
) -> Result<Json<()>, AppError> {
    require_owner(&auth, &headers)?;
    service::remove_connection(session_db()?.clone())
        .await
        .map(Json)
        .map_err(api_error)
}

pub async fn personal_assistant_im_set_notifications_paused(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
    Json(input): Json<PauseInput>,
) -> Result<Json<()>, AppError> {
    require_owner(&auth, &headers)?;
    service::set_notifications_paused(session_db()?.clone(), input.paused)
        .await
        .map(Json)
        .map_err(api_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_im_owner_guard_rejects_no_auth_and_scoped_ticket_credentials() {
        let empty = HeaderMap::new();
        assert!(require_owner(&AuthState::new(None, None, false), &empty).is_err());
        let auth = AuthState::new(Some("test-owner".to_owned()), None, false);
        assert!(require_owner(&auth, &empty).is_err());
        let mut headers = HeaderMap::new();
        headers.insert("authorization", "Bearer test-owner".parse().unwrap());
        assert!(require_owner(&auth, &headers).is_ok());
        let ticket = auth.create_access_ticket("owner_events", 60).unwrap();
        headers.insert("authorization", format!("Bearer {ticket}").parse().unwrap());
        assert!(require_owner(&auth, &headers).is_err());
    }
}
