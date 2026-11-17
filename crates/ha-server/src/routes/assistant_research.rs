//! Authenticated owner surface for source-scoped proactive work.
use crate::{error::AppError, middleware::AuthState, routes::helpers::session_db};
use axum::{extract::Path, Extension, Json};
use ha_core::personal_assistant::{
    policy::{AssistantPolicySettings, AssistantPolicyStatus},
    research::{
        self, AssistantFinding, AssistantSource, AssistantSourceInput, ResearchActivity,
        ResearchQueryReceipt,
    },
};

fn owner(auth: &AuthState) -> Result<(), AppError> {
    if !auth.auth_required() {
        return Err(AppError::forbidden(
            "Personal assistant access requires a Server Owner Token.",
        ));
    }
    Ok(())
}
fn available() -> Result<(), AppError> {
    if !research::coordinator_registered() {
        return Err(AppError::forbidden(
            "The assistant coordinator is unavailable.",
        ));
    }
    Ok(())
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PolicyInput {
    pub expected_revision: i64,
    pub settings: AssistantPolicySettings,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceInput {
    pub source: AssistantSourceInput,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequestInput {
    pub request_key: String,
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewInput {
    pub revision: i64,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuestionInput {
    pub revision: i64,
    pub question: String,
    pub request_key: String,
}

pub async fn get_personal_assistant_policy(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<AssistantPolicyStatus>, AppError> {
    owner(&auth)?;
    Ok(Json(
        session_db()?.run(|db| db.assistant_policy_status()).await?,
    ))
}
pub async fn set_personal_assistant_policy(
    Extension(auth): Extension<AuthState>,
    Json(input): Json<PolicyInput>,
) -> Result<Json<AssistantPolicyStatus>, AppError> {
    owner(&auth)?;
    available()?;
    Ok(Json(
        session_db()?
            .run(move |db| db.request_assistant_policy(input.expected_revision, input.settings))
            .await?,
    ))
}
pub async fn list_personal_assistant_sources(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<Vec<AssistantSource>>, AppError> {
    owner(&auth)?;
    Ok(Json(
        session_db()?.run(|db| db.list_assistant_sources()).await?,
    ))
}
pub async fn add_personal_assistant_source(
    Extension(auth): Extension<AuthState>,
    Json(input): Json<SourceInput>,
) -> Result<Json<AssistantSource>, AppError> {
    owner(&auth)?;
    available()?;
    Ok(Json(
        research::create_source(session_db()?.clone(), input.source).await?,
    ))
}
pub async fn revoke_personal_assistant_source(
    Extension(auth): Extension<AuthState>,
    Path(id): Path<String>,
) -> Result<Json<()>, AppError> {
    owner(&auth)?;
    session_db()?
        .run(move |db| db.revoke_assistant_source(&id, chrono::Utc::now().timestamp()))
        .await?;
    Ok(Json(()))
}
pub async fn check_personal_assistant_source(
    Extension(auth): Extension<AuthState>,
    Path(id): Path<String>,
    Json(input): Json<RequestInput>,
) -> Result<Json<ResearchQueryReceipt>, AppError> {
    owner(&auth)?;
    available()?;
    let db = session_db()?.clone();
    research::refresh_owner_sources(db.clone()).await?;
    Ok(Json(
        db.run(move |db| {
            db.request_assistant_source_review(
                &id,
                &input.request_key,
                chrono::Utc::now().timestamp(),
            )
        })
        .await?,
    ))
}
pub async fn list_personal_assistant_findings(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<Vec<AssistantFinding>>, AppError> {
    owner(&auth)?;
    let db = session_db()?.clone();
    research::refresh_owner_sources(db.clone()).await?;
    Ok(Json(
        db.run(|db| db.list_assistant_findings(chrono::Utc::now().timestamp()))
            .await?,
    ))
}
pub async fn view_personal_assistant_finding(
    Extension(auth): Extension<AuthState>,
    Path(id): Path<String>,
    Json(input): Json<ViewInput>,
) -> Result<Json<()>, AppError> {
    owner(&auth)?;
    let db = session_db()?.clone();
    research::refresh_owner_sources(db.clone()).await?;
    db.run(move |db| {
        db.view_assistant_finding(&id, input.revision, chrono::Utc::now().timestamp())
    })
    .await?;
    Ok(Json(()))
}
pub async fn ask_personal_assistant_finding(
    Extension(auth): Extension<AuthState>,
    Path(id): Path<String>,
    Json(input): Json<QuestionInput>,
) -> Result<Json<ResearchQueryReceipt>, AppError> {
    owner(&auth)?;
    available()?;
    let db = session_db()?.clone();
    research::refresh_owner_sources(db.clone()).await?;
    Ok(Json(
        db.run(move |db| {
            db.request_assistant_research_question(
                &id,
                input.revision,
                &input.question,
                &input.request_key,
                chrono::Utc::now().timestamp(),
            )
        })
        .await?,
    ))
}
pub async fn personal_assistant_research_activity(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<Vec<ResearchActivity>>, AppError> {
    owner(&auth)?;
    Ok(Json(
        session_db()?
            .run(|db| db.list_assistant_research_activity())
            .await?,
    ))
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PresenceInput {
    pub client_id: String,
    pub active: bool,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoticeClaimInput {
    pub client_id: String,
    pub claim_key: String,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoticeFinishInput {
    pub client_id: String,
    pub claim_key: String,
    pub outcome: ha_core::personal_assistant::attention::AttentionOutcome,
}
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DisclosureInput {
    pub revision: i64,
    pub enabled: bool,
    pub expected_disclosure_revision: i64,
    pub expected_connection_id: String,
    pub expected_generation: i64,
}

pub async fn get_personal_assistant_attention(
    Extension(auth): Extension<AuthState>,
) -> Result<Json<ha_core::personal_assistant::attention::AssistantAttentionStatus>, AppError> {
    owner(&auth)?;
    Ok(Json(
        session_db()?
            .run(|db| db.assistant_attention_status(chrono::Utc::now().timestamp()))
            .await?,
    ))
}
pub async fn set_personal_assistant_presence(
    Extension(auth): Extension<AuthState>,
    Json(input): Json<PresenceInput>,
) -> Result<Json<()>, AppError> {
    owner(&auth)?;
    session_db()?
        .run(move |db| {
            db.update_assistant_presence(
                &input.client_id,
                input.active,
                chrono::Utc::now().timestamp(),
            )
        })
        .await?;
    Ok(Json(()))
}
pub async fn claim_personal_assistant_desktop_notice(
    Extension(auth): Extension<AuthState>,
    Json(input): Json<NoticeClaimInput>,
) -> Result<Json<Option<ha_core::personal_assistant::attention::AssistantDesktopNotice>>, AppError>
{
    owner(&auth)?;
    Ok(Json(
        session_db()?
            .clone()
            .claim_assistant_desktop_notice(input.client_id, input.claim_key)
            .await?,
    ))
}
pub async fn finish_personal_assistant_desktop_notice(
    Extension(auth): Extension<AuthState>,
    Path(id): Path<String>,
    Json(input): Json<NoticeFinishInput>,
) -> Result<Json<()>, AppError> {
    owner(&auth)?;
    session_db()?
        .run(move |db| {
            db.finish_assistant_desktop_notice(
                &id,
                &input.client_id,
                &input.claim_key,
                input.outcome,
                chrono::Utc::now().timestamp(),
            )
        })
        .await?;
    Ok(Json(()))
}
pub async fn set_personal_assistant_source_im_disclosure(
    Extension(auth): Extension<AuthState>,
    Path(id): Path<String>,
    Json(input): Json<DisclosureInput>,
) -> Result<Json<()>, AppError> {
    owner(&auth)?;
    session_db()?
        .run(move |db| {
            db.set_assistant_source_im_disclosure(
                &id,
                input.revision,
                input.enabled,
                input.expected_disclosure_revision,
                &input.expected_connection_id,
                input.expected_generation,
                chrono::Utc::now().timestamp(),
            )
        })
        .await?;
    Ok(Json(()))
}

#[cfg(test)]
mod attention_owner_tests {
    use super::*;
    #[tokio::test]
    async fn attention_operations_reject_no_auth_before_accessing_private_state() {
        let auth = AuthState::new(None, None, false);
        assert_eq!(
            get_personal_assistant_attention(Extension(auth.clone()))
                .await
                .unwrap_err()
                .status,
            axum::http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            set_personal_assistant_presence(
                Extension(auth.clone()),
                Json(PresenceInput {
                    client_id: uuid::Uuid::new_v4().to_string(),
                    active: true
                })
            )
            .await
            .unwrap_err()
            .status,
            axum::http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            claim_personal_assistant_desktop_notice(
                Extension(auth.clone()),
                Json(NoticeClaimInput {
                    client_id: uuid::Uuid::new_v4().to_string(),
                    claim_key: uuid::Uuid::new_v4().to_string()
                })
            )
            .await
            .unwrap_err()
            .status,
            axum::http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            finish_personal_assistant_desktop_notice(
                Extension(auth.clone()),
                Path("notice".into()),
                Json(NoticeFinishInput {
                    client_id: uuid::Uuid::new_v4().to_string(),
                    claim_key: uuid::Uuid::new_v4().to_string(),
                    outcome: ha_core::personal_assistant::attention::AttentionOutcome::Accepted
                })
            )
            .await
            .unwrap_err()
            .status,
            axum::http::StatusCode::FORBIDDEN
        );
        assert_eq!(
            set_personal_assistant_source_im_disclosure(
                Extension(auth),
                Path("source".into()),
                Json(DisclosureInput {
                    revision: 1,
                    enabled: true,
                    expected_disclosure_revision: 0,
                    expected_connection_id: "connection".into(),
                    expected_generation: 1
                })
            )
            .await
            .unwrap_err()
            .status,
            axum::http::StatusCode::FORBIDDEN
        );
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionNotificationInput {
    pub execution_id: String,
}
/// Uses the enclosing ordinary authentication boundary. Returns only a host
/// classification, so a no-auth server without private data keeps normal alerts.
pub async fn get_execution_notification_ownership(
    Json(input): Json<ExecutionNotificationInput>,
) -> Result<Json<ha_core::personal_assistant::attention::NotificationOwnership>, AppError> {
    if input.execution_id.is_empty() || input.execution_id.len() > 256 {
        return Ok(Json(
            ha_core::personal_assistant::attention::NotificationOwnership::Unknown,
        ));
    }
    Ok(Json(
        session_db()?
            .run(move |db| db.assistant_notification_execution_ownership(&input.execution_id))
            .await?,
    ))
}
