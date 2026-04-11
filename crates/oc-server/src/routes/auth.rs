//! Codex OAuth routes.
//!
//! The OAuth flow runs a local HTTP callback server (see
//! `oc_core::oauth::start_oauth_flow`) and stores the resulting `TokenData`
//! in a shared mutex. Two distinct requests from the frontend — "start" and
//! "finalize" — access the same mutex; we hold it in a process-wide
//! `OnceLock` so it outlives individual request handlers.

use axum::Json;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as TokioMutex;

use oc_core::oauth::{self, TokenData};

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

    let mut store = oc_core::config::load_config()?;
    let codex_provider_id = oc_core::provider::ensure_codex_provider(&mut store);
    store.active_model = Some(oc_core::provider::ActiveModel {
        provider_id: codex_provider_id,
        model_id: "gpt-5.4".to_string(),
    });
    oc_core::config::save_config(&store)?;

    // Persist token for subsequent sessions.
    oauth::save_token(&token).map_err(|e| AppError::internal(e.to_string()))?;

    Ok(Json(json!({
        "ok": true,
        "account_id": account_id,
    })))
}
