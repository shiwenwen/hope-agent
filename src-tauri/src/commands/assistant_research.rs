use crate::commands::CmdError;
use ha_core::personal_assistant::{
    policy::{AssistantPolicySettings, AssistantPolicyStatus},
    research::{
        self, AssistantFinding, AssistantSource, AssistantSourceInput, ResearchActivity,
        ResearchQueryReceipt,
    },
};
fn available() -> Result<(), CmdError> {
    if !research::coordinator_registered() {
        return Err(anyhow::anyhow!("The assistant coordinator is unavailable.").into());
    }
    Ok(())
}
#[tauri::command]
pub async fn get_personal_assistant_policy(
    state: tauri::State<'_, crate::AppState>,
) -> Result<AssistantPolicyStatus, CmdError> {
    state
        .session_db
        .run(|db| db.assistant_policy_status())
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn set_personal_assistant_policy(
    expected_revision: i64,
    settings: AssistantPolicySettings,
    state: tauri::State<'_, crate::AppState>,
) -> Result<AssistantPolicyStatus, CmdError> {
    available()?;
    state
        .session_db
        .run(move |db| db.request_assistant_policy(expected_revision, settings))
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn list_personal_assistant_sources(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<AssistantSource>, CmdError> {
    state
        .session_db
        .run(|db| db.list_assistant_sources())
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn add_personal_assistant_source(
    source: AssistantSourceInput,
    state: tauri::State<'_, crate::AppState>,
) -> Result<AssistantSource, CmdError> {
    available()?;
    research::create_source(state.session_db.clone(), source)
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn revoke_personal_assistant_source(
    id: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    state
        .session_db
        .run(move |db| db.revoke_assistant_source(&id, chrono::Utc::now().timestamp()))
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn check_personal_assistant_source(
    id: String,
    request_key: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<ResearchQueryReceipt, CmdError> {
    available()?;
    research::refresh_owner_sources(state.session_db.clone()).await?;
    state
        .session_db
        .run(move |db| {
            db.request_assistant_source_review(&id, &request_key, chrono::Utc::now().timestamp())
        })
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn list_personal_assistant_findings(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<AssistantFinding>, CmdError> {
    research::refresh_owner_sources(state.session_db.clone()).await?;
    state
        .session_db
        .run(|db| db.list_assistant_findings(chrono::Utc::now().timestamp()))
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn view_personal_assistant_finding(
    id: String,
    revision: i64,
    state: tauri::State<'_, crate::AppState>,
) -> Result<(), CmdError> {
    research::refresh_owner_sources(state.session_db.clone()).await?;
    state
        .session_db
        .run(move |db| db.view_assistant_finding(&id, revision, chrono::Utc::now().timestamp()))
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn ask_personal_assistant_finding(
    id: String,
    revision: i64,
    question: String,
    request_key: String,
    state: tauri::State<'_, crate::AppState>,
) -> Result<ResearchQueryReceipt, CmdError> {
    available()?;
    research::refresh_owner_sources(state.session_db.clone()).await?;
    state
        .session_db
        .run(move |db| {
            db.request_assistant_research_question(
                &id,
                revision,
                &question,
                &request_key,
                chrono::Utc::now().timestamp(),
            )
        })
        .await
        .map_err(Into::into)
}
#[tauri::command]
pub async fn personal_assistant_research_activity(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<ResearchActivity>, CmdError> {
    state
        .session_db
        .run(|db| db.list_assistant_research_activity())
        .await
        .map_err(Into::into)
}
