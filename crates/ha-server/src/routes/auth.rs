//! Codex OAuth routes.
//!
//! The OAuth flow runs a local HTTP callback server (see
//! `ha_core::oauth::start_oauth_flow`) and stores the resulting `TokenData`
//! in a shared mutex. Two distinct requests from the frontend — "start" and
//! "finalize" — access the same mutex; we hold it in a process-wide
//! `OnceLock` so it outlives individual request handlers.

use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as TokioMutex;

use ha_core::agent;
use ha_core::oauth::{self, AuthStatus, TokenData};

use crate::error::AppError;

type AuthResult = Arc<TokioMutex<Option<anyhow::Result<TokenData>>>>;

fn auth_result_slot() -> AuthResult {
    static SLOT: OnceLock<AuthResult> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(TokioMutex::new(None))).clone()
}

/// `POST /api/auth/codex/start` — kick off the Codex OAuth flow.
///
/// Spawns a local callback server + opens the auth URL in the user's
/// browser. On desktop this blocks the caller until the user completes the
/// flow; in headless server mode the callback page is delivered to whatever
/// browser the operator is pointing at the server. Use
/// `POST /api/auth/codex/finalize` afterwards to convert the landed token
/// into an active provider.
pub async fn start_codex_auth() -> Result<Json<Value>, AppError> {
    let slot = auth_result_slot();
    {
        let mut lock = slot.lock().await;
        *lock = None;
    }
    oauth::start_oauth_flow(slot)
        .await
        .map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/auth/codex/finalize` — read the token produced by
/// `start_codex_auth`, register the Codex provider, and persist it.
pub async fn finalize_codex_auth() -> Result<Json<Value>, AppError> {
    let slot = auth_result_slot();
    let token = {
        let mut lock = slot.lock().await;
        match lock.take() {
            Some(Ok(token)) => token,
            Some(Err(e)) => return Err(AppError::internal(e.to_string())),
            None => return Err(AppError::bad_request("Auth not complete yet")),
        }
    };

    let account_id = token
        .account_id
        .clone()
        .or_else(|| oauth::extract_account_id(&token.access_token))
        .ok_or_else(|| {
            AppError::internal("Failed to extract account ID from Codex token".to_string())
        })?;

    let mut store = ha_core::config::load_config()?;
    let codex_provider_id = ha_core::provider::ensure_codex_provider(&mut store);
    store.active_model = Some(ha_core::provider::ActiveModel {
        provider_id: codex_provider_id,
        model_id: "gpt-5.4".to_string(),
    });
    ha_core::config::save_config(&store)?;

    // Persist token for subsequent sessions.
    oauth::save_token(&token).map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(json!({
        "ok": true,
        "account_id": account_id,
    })))
}

/// `GET /api/auth/codex/status` — is the Codex OAuth token configured and
/// valid? Mirrors the Tauri `check_auth_status` command. Checks the
/// in-process result slot first (so a just-finished OAuth flow reports
/// correctly before disk-write races), falls back to on-disk token.
pub async fn check_auth_status() -> Result<Json<AuthStatus>, AppError> {
    // In-process slot (populated by `start_codex_auth` / `finalize_codex_auth`).
    {
        let slot = auth_result_slot();
        let lock = slot.lock().await;
        match lock.as_ref() {
            Some(Ok(_)) => {
                return Ok(Json(AuthStatus {
                    authenticated: true,
                    error: None,
                }))
            }
            Some(Err(e)) => {
                return Ok(Json(AuthStatus {
                    authenticated: false,
                    error: Some(e.to_string()),
                }))
            }
            None => {}
        }
    }
    // Fallback: persisted token on disk.
    match oauth::load_token() {
        Ok(Some(token)) if !oauth::is_token_expired(&token) => Ok(Json(AuthStatus {
            authenticated: true,
            error: None,
        })),
        _ => Ok(Json(AuthStatus {
            authenticated: false,
            error: None,
        })),
    }
}

/// `POST /api/auth/codex/logout` — clear the on-disk token, drop the
/// in-process result, and remove the Codex provider row from app config.
pub async fn logout_codex() -> Result<Json<Value>, AppError> {
    {
        let slot = auth_result_slot();
        let mut lock = slot.lock().await;
        *lock = None;
    }

    // Remove Codex provider from store, same as Tauri `logout_codex`.
    {
        let mut store = ha_core::config::load_config()?;
        store
            .providers
            .retain(|p| p.api_type != ha_core::provider::ApiType::Codex);
        if let Some(ref active) = store.active_model {
            if !store.providers.iter().any(|p| p.id == active.provider_id) {
                store.active_model = None;
            }
        }
        ha_core::config::save_config(&store)?;
    }

    oauth::clear_token().map_err(|e| AppError::internal(e.to_string()))?;
    Ok(Json(json!({ "ok": true })))
}

/// `POST /api/auth/session/restore` — attempt to re-hydrate a prior Codex
/// session from the saved token. Returns `{ restored: bool }`.
///
/// HTTP-mode scope: this only validates/refreshes the token and ensures the
/// Codex provider row exists. The agent cache is built per-request, so no
/// in-memory agent rehydration is needed here (in contrast to the Tauri
/// command which keeps `AppState.agent` populated).
pub async fn try_restore_session() -> Result<Json<Value>, AppError> {
    let token = match oauth::load_token() {
        Ok(Some(t)) => t,
        _ => return Ok(Json(json!({ "restored": false }))),
    };

    let token = if oauth::is_token_expired(&token) {
        let refresh = match &token.refresh_token {
            Some(rt) => rt.clone(),
            None => {
                let _ = oauth::clear_token();
                return Ok(Json(json!({ "restored": false })));
            }
        };
        match oauth::refresh_access_token(&refresh).await {
            Ok(new_token) => {
                oauth::save_token(&new_token).map_err(|e| AppError::internal(e.to_string()))?;
                new_token
            }
            Err(_) => {
                let _ = oauth::clear_token();
                return Ok(Json(json!({ "restored": false })));
            }
        }
    } else {
        token
    };

    let _account_id = token
        .account_id
        .clone()
        .or_else(|| oauth::extract_account_id(&token.access_token));

    // Ensure the Codex provider row exists so subsequent `chat` calls can
    // find it. Avoid a disk write + autosave snapshot when nothing actually
    // changed — this handler fires on every server-mode startup.
    {
        let mut store = ha_core::config::load_config()?;
        let provider_count_before = store.providers.len();
        let codex_provider_id = ha_core::provider::ensure_codex_provider(&mut store);
        let mut dirty = store.providers.len() != provider_count_before;
        if store.active_model.is_none() {
            store.active_model = Some(ha_core::provider::ActiveModel {
                provider_id: codex_provider_id,
                model_id: "gpt-5.4".to_string(),
            });
            dirty = true;
        }
        if dirty {
            ha_core::config::save_config(&store)?;
        }
    }

    Ok(Json(json!({ "restored": true })))
}

/// `GET /api/auth/codex/models` — list the Codex OAuth models the UI may
/// switch between. Mirrors the Tauri `get_codex_models` command.
pub async fn get_codex_models() -> Result<Json<Vec<agent::CodexModel>>, AppError> {
    Ok(Json(agent::get_codex_models()))
}

#[derive(Debug, Deserialize)]
pub struct SetCodexModelBody {
    pub model: String,
}

/// `POST /api/auth/codex/models` — switch the active Codex model. Mirrors
/// the Tauri `set_codex_model` command. In HTTP mode there is no in-memory
/// agent to rebuild; we only persist the selection to config so subsequent
/// `POST /api/chat` calls pick it up.
pub async fn set_codex_model(Json(body): Json<SetCodexModelBody>) -> Result<Json<Value>, AppError> {
    let valid = agent::get_codex_models().iter().any(|m| m.id == body.model);
    if !valid {
        return Err(AppError::bad_request(format!(
            "Unknown model: {}",
            body.model
        )));
    }

    let model = body.model;
    ha_core::config::mutate_config(("active_model", "set-codex-model"), |store| {
        if let Some(ref mut active) = store.active_model {
            active.model_id = model.clone();
        }
        Ok(())
    })?;

    Ok(Json(json!({ "ok": true })))
}
