use crate::commands::CmdError;
use crate::AppState;
use ha_core::local_embedding::OllamaEmbeddingModel;
use ha_core::local_llm::{ModelCandidate, OllamaPullRequest};
use ha_core::local_model_jobs::{self, LocalModelJobLogEntry, LocalModelJobSnapshot};
use tauri::State;

#[tauri::command]
pub async fn local_model_job_start_chat_model(
    model: ModelCandidate,
    state: State<'_, AppState>,
) -> Result<LocalModelJobSnapshot, CmdError> {
    let hook = local_model_jobs::rebuild_active_agent_hook(state.agent.clone());
    local_model_jobs::start_chat_model_job(model, Some(hook)).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_start_embedding(
    model: OllamaEmbeddingModel,
) -> Result<LocalModelJobSnapshot, CmdError> {
    local_model_jobs::start_embedding_job(model).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_start_ollama_install() -> Result<LocalModelJobSnapshot, CmdError> {
    local_model_jobs::start_ollama_install_job().map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_start_ollama_pull(
    request: OllamaPullRequest,
) -> Result<LocalModelJobSnapshot, CmdError> {
    local_model_jobs::start_ollama_pull_job(request).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_start_ollama_preload(
    model_id: String,
    display_name: Option<String>,
) -> Result<LocalModelJobSnapshot, CmdError> {
    local_model_jobs::start_ollama_preload_job(model_id, display_name).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_list() -> Result<Vec<LocalModelJobSnapshot>, CmdError> {
    local_model_jobs::list_jobs().map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_get(
    job_id: String,
) -> Result<Option<LocalModelJobSnapshot>, CmdError> {
    local_model_jobs::get_job(&job_id).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_logs(
    job_id: String,
    after_seq: Option<i64>,
) -> Result<Vec<LocalModelJobLogEntry>, CmdError> {
    local_model_jobs::get_logs(&job_id, after_seq).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_cancel(job_id: String) -> Result<LocalModelJobSnapshot, CmdError> {
    local_model_jobs::cancel_job(&job_id).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_pause(job_id: String) -> Result<LocalModelJobSnapshot, CmdError> {
    local_model_jobs::pause_job(&job_id).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_retry(
    job_id: String,
    state: State<'_, AppState>,
) -> Result<LocalModelJobSnapshot, CmdError> {
    let hook = local_model_jobs::rebuild_active_agent_hook(state.agent.clone());
    local_model_jobs::retry_job(&job_id, Some(hook)).map_err(Into::into)
}

#[tauri::command]
pub async fn local_model_job_clear(job_id: String) -> Result<(), CmdError> {
    local_model_jobs::clear_job(&job_id).map_err(Into::into)
}
