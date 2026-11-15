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
