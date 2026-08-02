//! Codex OAuth routes.
//!
//! The OAuth flow runs a local HTTP callback server (see
//! `ha_core::oauth::start_oauth_flow`) and stores the resulting `TokenData`
//! in a shared mutex. Two distinct requests from the frontend — "start" and
//! "finalize" — access the same mutex; we hold it in a process-wide
//! `OnceLock` so it outlives individual request handlers.

use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::{extract::ConnectInfo, Extension, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as TokioMutex;
use tower::ServiceExt;

use ha_core::agent;
use ha_core::oauth::{self, AuthStatus, TokenData};
use ha_core::provider::{ActiveModelUpdate, ApiType};

use crate::error::AppError;
use crate::middleware::{AuthState, BROWSER_SESSION_COOKIE};

const ACCESS_TICKET_TTL_SECS: u64 = 15 * 60;

#[derive(Clone)]
pub struct ScopedResourceService(pub axum::Router);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerAuthStatus {
    auth_required: bool,
    authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_fingerprint: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBrowserSessionBody {
    token: String,
    #[serde(default)]
    remember: bool,
}

/// Public bootstrap probe for the browser Auth Gate. The fingerprint is only
/// returned to an already-authenticated caller; exposing even a truncated hash
/// before login would provide an offline oracle for operator-chosen weak tokens.
pub async fn server_auth_status(
    Extension(auth): Extension<AuthState>,
    headers: HeaderMap,
) -> Response {
    no_store_json(StatusCode::OK, &server_auth_status_payload(&auth, &headers))
}

fn server_auth_status_payload(auth: &AuthState, headers: &HeaderMap) -> ServerAuthStatus {
    let authenticated = auth.headers_are_owner_authenticated(&headers);
    ServerAuthStatus {
        auth_required: auth.auth_required(),
        authenticated,
        token_fingerprint: authenticated.then(|| auth.owner_fingerprint()).flatten(),
    }
}

/// Exchange the long-lived owner token for an HttpOnly signed session. The
/// root token is never returned and need not remain in browser storage.
pub async fn create_browser_session(
    Extension(auth): Extension<AuthState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<CreateBrowserSessionBody>,
) -> Response {
    if !same_origin_or_non_browser(&headers) {
        return no_store_error(StatusCode::FORBIDDEN, "Cross-origin login is not allowed");
    }
    let peer = peer.ip();
    if !auth.login_allowed(peer) {
        return no_store_error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many failed attempts; try again shortly",
        );
    }
    if !auth.check_owner_token(body.token.as_bytes()) {
        auth.record_login_failure(peer);
        return no_store_error(StatusCode::UNAUTHORIZED, "Invalid owner token");
    }
    auth.clear_login_failures(peer);
    let max_age = if body.remember {
        30 * 24 * 60 * 60
    } else {
        12 * 60 * 60
    };
    let session = match auth.create_browser_session(max_age) {
        Ok(session) => session,
        Err(error) => {
            ha_core::app_error!(
                "security",
                "browser_session_create_failed",
                "Failed to create browser session: {}",
                error
            );
            return no_store_error(StatusCode::INTERNAL_SERVER_ERROR, "Login is unavailable");
        }
    };
    let secure = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|origin| origin.starts_with("https://"));
    let cookie = format!(
        "{BROWSER_SESSION_COOKIE}={session}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
        if secure { "; Secure" } else { "" }
    );
    let mut response = no_store_json(StatusCode::OK, &serde_json::json!({ "ok": true }));
    if let Ok(cookie) = HeaderValue::from_str(&cookie) {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

pub async fn clear_browser_session(headers: HeaderMap) -> Response {
    if !same_origin_or_non_browser(&headers) {
        return no_store_error(StatusCode::FORBIDDEN, "Cross-origin logout is not allowed");
    }
    let mut response = no_store_json(StatusCode::OK, &serde_json::json!({ "ok": true }));
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("ha_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
    );
    response
}

/// Mint transport-only capabilities for browser APIs that cannot attach an
/// Authorization header. The event ticket is carried as a WebSocket
/// subprotocol; the resource ticket is accepted only by the dedicated
/// read-only resource dispatcher.
pub async fn create_transport_access_tickets(Extension(auth): Extension<AuthState>) -> Response {
    if !auth.auth_required() {
        return no_store_json(
            StatusCode::OK,
            &serde_json::json!({
                "authRequired": false,
                "resourceTicket": null,
                "eventTicket": null,
                "expiresInSecs": null,
            }),
        );
    }
    match auth.create_transport_access_tickets(ACCESS_TICKET_TTL_SECS) {
        Ok((resource_ticket, event_ticket)) => no_store_json(
            StatusCode::OK,
            &serde_json::json!({
                "authRequired": true,
                "resourceTicket": resource_ticket,
                "eventTicket": event_ticket,
                "expiresInSecs": ACCESS_TICKET_TTL_SECS,
            }),
        ),
        Err(error) => {
            ha_core::app_error!(
                "security",
                "transport_ticket_mint_failed",
                "Transport access ticket minting failed: {}",
                error
            );
            no_store_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Transport access is unavailable",
            )
        }
    }
}

/// Validate a short-lived read-only ticket, then internally dispatch to the
/// small resource-only router. Rewriting the URI preserves the ticket prefix
/// in the browser, so relative iframe CSS/JS/image URLs inherit it without
/// exposing the Owner Token.
pub async fn serve_scoped_resource(
    Extension(auth): Extension<AuthState>,
    Extension(service): Extension<ScopedResourceService>,
    request: Request,
) -> Response {
    const PREFIX: &str = "/api/resource/";
    let raw_path = request.uri().path();
    let Some(ticket_and_path) = raw_path.strip_prefix(PREFIX) else {
        return no_store_error(StatusCode::NOT_FOUND, "Resource not found");
    };
    let Some((ticket, resource_path)) = ticket_and_path.split_once('/') else {
        return no_store_error(StatusCode::NOT_FOUND, "Resource not found");
    };
    if !auth.check_access_ticket(ticket, "resources") {
        return no_store_error(
            StatusCode::UNAUTHORIZED,
            "Invalid or expired resource ticket",
        );
    }

    let rewritten = match request.uri().query() {
        Some(query) => format!("/{resource_path}?{query}"),
        None => format!("/{resource_path}"),
    };
    let Ok(uri) = rewritten.parse::<Uri>() else {
        return no_store_error(StatusCode::BAD_REQUEST, "Invalid resource path");
    };
    let (mut parts, body) = request.into_parts();
    parts.uri = uri;
    // The outer capability route already installed its `{ticket}` / `{path}`
    // captures. Do not forward those private Axum extensions into the inner
    // router or its `Path<T>` extractors would see both capture sets.
    parts.extensions = Default::default();
    let request = Request::from_parts(parts, body);
    match service.0.clone().oneshot(request).await {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

/// Rotate the single owner root token. The new value is returned exactly once;
/// all existing browser sessions and Bearer clients become invalid immediately.
pub async fn rotate_server_owner_token(Extension(auth): Extension<AuthState>) -> Response {
    if auth.externally_managed() {
        return no_store_error(
            StatusCode::CONFLICT,
            "The owner token is externally managed; rotate it at its source",
        );
    }
    let rotated = match ha_core::blocking::run_blocking(|| {
        ha_core::server_auth::rotate_managed_token("http-token-rotate")
    })
    .await
    {
        Ok(rotated) => rotated,
        Err(error) => {
            ha_core::app_error!(
                "security",
                "server_token_rotate_failed",
                "Failed to rotate server owner token: {}",
                error
            );
            return no_store_error(StatusCode::INTERNAL_SERVER_ERROR, "Token rotation failed");
        }
    };
    if let Err(error) = auth.replace_owner_token(Some(rotated.token.clone())) {
        ha_core::app_error!(
            "security",
            "server_token_activate_failed",
            "New server owner token was stored but could not be activated: {}",
            error
        );
        return no_store_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Token was stored but requires a server restart",
        );
    }
    no_store_json(
        StatusCode::OK,
        &serde_json::json!({
            "token": rotated.token,
            "fingerprint": ha_core::server_auth::token_fingerprint(&rotated.token),
        }),
    )
}

fn same_origin_or_non_browser(headers: &HeaderMap) -> bool {
    let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    else {
        return true;
    };
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    origin
        .parse::<axum::http::Uri>()
        .ok()
        .and_then(|uri| {
            uri.authority()
                .map(|authority| authority.as_str().to_string())
        })
        .is_some_and(|authority| authority.eq_ignore_ascii_case(host))
}

fn no_store_json<T: Serialize>(status: StatusCode, body: &T) -> Response {
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, max-age=0"),
    );
    response
}

fn no_store_error(status: StatusCode, message: &'static str) -> Response {
    no_store_json(status, &serde_json::json!({ "error": message }))
}

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

    ha_core::blocking::run_blocking(|| {
        ha_core::provider::ensure_codex_provider_persisted(
            ActiveModelUpdate::Always(ha_core::agent::DEFAULT_CODEX_MODEL_ID.to_string()),
            "oauth-finalize-http",
        )
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;

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

    ha_core::blocking::run_blocking(|| {
        ha_core::provider::delete_providers_by_api_type(ApiType::Codex, "http")
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;

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
    ha_core::blocking::run_blocking(|| {
        ha_core::provider::ensure_codex_provider_persisted(
            ActiveModelUpdate::IfMissing(ha_core::agent::DEFAULT_CODEX_MODEL_ID.to_string()),
            "session-restore-http",
        )
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?;

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
    if !agent::is_valid_codex_model(&body.model) {
        return Err(AppError::bad_request(format!(
            "Unknown model: {}",
            body.model
        )));
    }

    let model = body.model;
    ha_core::config::mutate_config_async(("active_model", "set-codex-model"), move |store| {
        if let Some(ref mut active) = store.active_model {
            active.model_id = model;
        }
        Ok(())
    })
    .await?;

    Ok(Json(json!({ "ok": true })))
}

#[cfg(test)]
mod server_token_tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::Path;
    use axum::http::Request as HttpRequest;
    use axum::routing::get;
    use axum::Router;

    #[test]
    fn public_auth_probe_does_not_expose_a_token_fingerprint_or_oracle() {
        let auth = AuthState::new(Some("weak-operator-token".to_string()), None, false);
        let status = server_auth_status_payload(&auth, &HeaderMap::new());
        assert!(status.auth_required);
        assert!(!status.authenticated);
        assert!(status.token_fingerprint.is_none());
        let json = serde_json::to_value(status).unwrap();
        assert!(json.get("tokenFingerprint").is_none());
    }

    #[test]
    fn authenticated_auth_probe_can_return_the_operator_fingerprint() {
        let auth = AuthState::new(Some("owner-token".to_string()), None, false);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer owner-token"),
        );
        let status = server_auth_status_payload(&auth, &headers);
        assert!(status.authenticated);
        assert_eq!(
            status.token_fingerprint,
            Some(ha_core::server_auth::token_fingerprint("owner-token"))
        );
    }

    #[tokio::test]
    async fn scoped_resource_ticket_reaches_only_the_read_only_dispatcher() {
        let auth = AuthState::new(Some("owner-token".to_string()), None, false);
        let ticket = auth.create_access_ticket("resources", 900).unwrap();
        let resources = Router::new().route(
            "/avatars/{filename}",
            get(|Path(filename): Path<String>| async move {
                assert_eq!(filename, "a.png");
                StatusCode::NO_CONTENT
            }),
        );
        let app = Router::new()
            .route("/api/resource/{ticket}/{*path}", get(serve_scoped_resource))
            .layer(Extension(auth))
            .layer(Extension(ScopedResourceService(resources)));

        let valid = app
            .clone()
            .oneshot(
                HttpRequest::builder()
                    .uri(format!("/api/resource/{ticket}/avatars/a.png"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(valid.status(), StatusCode::NO_CONTENT);

        let invalid = app
            .oneshot(
                HttpRequest::builder()
                    .uri("/api/resource/not-a-ticket/avatars/a.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
    }
}
