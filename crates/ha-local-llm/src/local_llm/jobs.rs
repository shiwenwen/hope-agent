//! 本地模型任务的**执行器**——Ollama 安装 / 拉取 / 预载 / 嵌入模型下载的
//! 实际跑批逻辑与进度处理，以及按 kind 分派的 retry。
//!
//! 与 [`ha_core::local_model_jobs`]（任务簿记面：DB / 快照类型 / spawn /
//! finish / 进度写入 / 取消暂停）分家的理由（crate-split 破环）：簿记面被
//! kernel 的 `memory::reembed_job` 与未来的 ha-knowledge 共用——它是通用的
//! 后台任务台账，不是 Ollama 专属；执行器才是 ha-local-llm 的本体。合在
//! 一起会让 knowledge 为了记账而依赖 local-llm，正是 7-环里最后一条边。

use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use tokio_util::sync::CancellationToken;

use crate::local_embedding::{self, OllamaEmbeddingModel};
use crate::local_llm::{
    self, install_ollama_via_script_cancellable, start_ollama, InstallScriptKind,
    InstallScriptProgress, ModelCandidate, OllamaPhase, OllamaPullRequest, PullProgress,
};
use ha_core::local_model_jobs::{
    append_log, emit_snapshot, finish_job, require_db, spawn_job, update_job,
    update_job_with_bytes, ChatCompletionHook, LocalModelJobKind, LocalModelJobSnapshot,
    LocalModelJobStatus, ProgressThrottle, EVENT_LOCAL_MODEL_JOB_UPDATED,
};

const PRELOAD_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
const PRELOAD_MAX_LOADING_PERCENT: u8 = 90;

pub fn start_chat_model_job(
    model: ModelCandidate,
    on_complete: Option<ChatCompletionHook>,
) -> Result<LocalModelJobSnapshot> {
    let model_id = model.id.clone();
    let display_name = model.display_name.clone();
    spawn_job(
        LocalModelJobKind::ChatModel,
        model_id,
        display_name,
        move |job_id, token| run_chat_model_job(job_id, model, token, on_complete),
    )
}

pub fn start_embedding_job(model: OllamaEmbeddingModel) -> Result<LocalModelJobSnapshot> {
    let model_id = model.id.clone();
    let display_name = model.display_name.clone();
    spawn_job(
        LocalModelJobKind::EmbeddingModel,
        model_id,
        display_name,
        move |job_id, token| run_embedding_job(job_id, model, token),
    )
}

pub fn start_ollama_install_job() -> Result<LocalModelJobSnapshot> {
    spawn_job(
        LocalModelJobKind::OllamaInstall,
        "ollama".into(),
        "Ollama".into(),
        run_ollama_install_job,
    )
}

pub fn start_ollama_pull_job(request: OllamaPullRequest) -> Result<LocalModelJobSnapshot> {
    let model_id = request.model_id.clone();
    let display_name = request
        .display_name
        .clone()
        .unwrap_or_else(|| request.model_id.clone());
    spawn_job(
        LocalModelJobKind::OllamaPull,
        model_id,
        display_name,
        move |job_id, token| run_ollama_pull_job(job_id, request, token),
    )
}

pub fn start_ollama_preload_job(
    model_id: String,
    display_name: Option<String>,
) -> Result<LocalModelJobSnapshot> {
    let display_name = display_name.unwrap_or_else(|| model_id.clone());
    spawn_job(
        LocalModelJobKind::OllamaPreload,
        model_id.clone(),
        display_name,
        move |job_id, token| run_ollama_preload_job(job_id, model_id, token),
    )
}

pub fn retry_job(
    job_id: &str,
    on_chat_complete: Option<ChatCompletionHook>,
) -> Result<LocalModelJobSnapshot> {
    let db = require_db()?.clone();
    let job = db
        .load(job_id)?
        .ok_or_else(|| anyhow!("Local model job not found: {job_id}"))?;
    if !job.status.is_terminal() {
        return Err(anyhow!("Only terminal jobs can be retried"));
    }
    let hide_original_after_retry = matches!(
        job.status,
        LocalModelJobStatus::Paused
            | LocalModelJobStatus::Failed
            | LocalModelJobStatus::Interrupted
    );
    let next_job = match job.kind {
        LocalModelJobKind::ChatModel => {
            let model = local_llm::model_catalog()
                .into_iter()
                .find(|model| model.id == job.model_id)
                .ok_or_else(|| anyhow!("Unsupported Ollama model: {}", job.model_id))?;
            start_chat_model_job(model, on_chat_complete)
        }
        LocalModelJobKind::EmbeddingModel => {
            let model = local_embedding::resolve_catalog_model(&job.model_id)?;
            start_embedding_job(model)
        }
        LocalModelJobKind::OllamaInstall => start_ollama_install_job(),
        LocalModelJobKind::OllamaPull => {
            let _ = on_chat_complete;
            start_ollama_pull_job(OllamaPullRequest {
                model_id: job.model_id,
                display_name: Some(job.display_name),
            })
        }
        LocalModelJobKind::OllamaPreload => {
            let _ = on_chat_complete;
            start_ollama_preload_job(job.model_id, Some(job.display_name))
        }
        LocalModelJobKind::MemoryReembed => {
            // Retry always uses KeepExisting: a partially-failed DeleteAll
            // already cleared the rows, so KeepExisting reembeds the same
            // empty vectors. The chat-completion hook is irrelevant here.
            let _ = on_chat_complete;
            ha_core::memory::reembed_job::start_memory_reembed_job(
                &job.model_id,
                ha_core::memory::reembed_job::ReembedMode::KeepExisting,
                // Retry 路径里没有可跟踪的发起者任务（用户从历史任务卡片重启
                // 一次失败的 reembed），故不传 successor 链路。
                None,
            )
        }
        LocalModelJobKind::KnowledgeReembed => {
            // Retry re-runs the same scope the failed job had (`None` = every
            // KB, `Some(ids)` = the specific KB(s) it targeted) — a single-KB
            // bind-scan failure must retry just that KB, not escalate into a
            // full-app rebuild. The chat-completion hook is irrelevant here.
            let _ = on_chat_complete;
            ha_knowledge::knowledge::reembed::start_knowledge_reembed_job(
                job.target_kb_ids.clone(),
                "retry",
            )
        }
    }?;

    if hide_original_after_retry {
        match db.mark_cancelled(job_id) {
            Ok(Some(cancelled)) => emit_snapshot(EVENT_LOCAL_MODEL_JOB_UPDATED, &cancelled),
            Ok(None) => app_warn!(
                "local_model_jobs",
                "retry",
                "Retried local model job but original job was not found: {}",
                job_id
            ),
            Err(e) => app_warn!(
                "local_model_jobs",
                "retry",
                "Retried local model job but failed to hide original job {}: {}",
                job_id,
                e
            ),
        }
    }

    Ok(next_job)
}

async fn run_chat_model_job(
    job_id: String,
    model: ModelCandidate,
    cancel_token: CancellationToken,
    on_complete: Option<ChatCompletionHook>,
) {
    let final_result = match run_common_setup(&job_id, &cancel_token).await {
        Ok(()) => {
            let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
            let job_id_for_progress = job_id.clone();
            match local_llm::pull_and_activate_cancellable(
                model,
                move |progress| handle_pull_progress(&job_id_for_progress, progress, &throttle),
                cancel_token.clone(),
            )
            .await
            {
                Ok((provider_id, model_id)) => {
                    if let Some(hook) = on_complete {
                        hook(provider_id.clone(), model_id.clone());
                    }
                    Ok(json!({ "providerId": provider_id, "modelId": model_id }))
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    finish_job(&job_id, final_result, &cancel_token);
}

async fn run_embedding_job(
    job_id: String,
    model: OllamaEmbeddingModel,
    cancel_token: CancellationToken,
) {
    let final_result = match run_common_setup(&job_id, &cancel_token).await {
        Ok(()) => {
            let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
            let job_id_for_progress = job_id.clone();
            local_embedding::pull_and_activate_cancellable(
                model,
                move |progress| handle_pull_progress(&job_id_for_progress, progress, &throttle),
                cancel_token.clone(),
                Some(job_id.clone()),
            )
            .await
            .map(|config| json!(config))
        }
        Err(e) => Err(e),
    };

    finish_job(&job_id, final_result, &cancel_token);
}

async fn run_ollama_install_job(job_id: String, cancel_token: CancellationToken) {
    let final_result = install_ollama_only(&job_id, &cancel_token).await;
    finish_job(&job_id, final_result, &cancel_token);
}

async fn run_ollama_pull_job(
    job_id: String,
    request: OllamaPullRequest,
    cancel_token: CancellationToken,
) {
    let final_result = match run_common_setup(&job_id, &cancel_token).await {
        Ok(()) => {
            let throttle = Arc::new(Mutex::new(ProgressThrottle::default()));
            let job_id_for_progress = job_id.clone();
            let model_id = request.model_id.clone();
            match local_llm::pull_model_cancellable(
                &model_id,
                move |progress| handle_pull_progress(&job_id_for_progress, progress, &throttle),
                cancel_token.clone(),
            )
            .await
            {
                Ok(()) => {
                    update_job(
                        &job_id,
                        LocalModelJobStatus::Running,
                        "done",
                        Some(100),
                        None,
                        None,
                    );
                    Ok(json!({
                        "modelId": model_id,
                        "downloaded": true
                    }))
                }
                Err(e) => Err(e),
            }
        }
        Err(e) => Err(e),
    };
    finish_job(&job_id, final_result, &cancel_token);
}

async fn run_ollama_preload_job(job_id: String, model_id: String, cancel_token: CancellationToken) {
    let final_result = match run_common_setup(&job_id, &cancel_token).await {
        Ok(()) => preload_ollama_model_for_job(&job_id, &model_id, &cancel_token).await,
        Err(e) => Err(e),
    };
    finish_job(&job_id, final_result, &cancel_token);
}

async fn preload_ollama_model_for_job(
    job_id: &str,
    model_id: &str,
    cancel_token: &CancellationToken,
) -> Result<Value> {
    if local_llm::is_ollama_model_running(model_id).await? {
        append_log(job_id, "step", "Model is already loaded");
        update_job(
            job_id,
            LocalModelJobStatus::Running,
            "done",
            Some(100),
            None,
            None,
        );
        return Ok(json!({
            "modelId": model_id,
            "loaded": true,
            "alreadyRunning": true
        }));
    }

    append_log(job_id, "step", &format!("Load model {model_id}"));
    update_job(
        job_id,
        LocalModelJobStatus::Running,
        "loading-model",
        Some(10),
        None,
        None,
    );

    let mut preload = Box::pin(local_llm::preload_ollama_model(model_id));
    let mut poll_count = 0u8;
    let mut observed_running = false;
    let mut last_progress: (&'static str, u8) = ("loading-model", 10);
    loop {
        tokio::select! {
            result = &mut preload => {
                result?;
                emit_preload_progress(job_id, "verifying-load", 95, &mut last_progress);
                if !local_llm::is_ollama_model_running(model_id).await? {
                    return Err(anyhow!(
                        "Ollama finished the model load request, but {model_id} is not listed by /api/ps"
                    ));
                }
                append_log(job_id, "step", "Model loaded");
                emit_preload_progress(job_id, "done", 100, &mut last_progress);
                return Ok(json!({
                    "modelId": model_id,
                    "loaded": true,
                    "alreadyRunning": false
                }));
            }
            _ = cancel_token.cancelled() => {
                unload_after_preload_cancel(job_id, model_id, observed_running).await;
                return Err(anyhow!("Local model job was cancelled"));
            }
            _ = tokio::time::sleep(PRELOAD_POLL_INTERVAL) => {
                if local_llm::is_ollama_model_running(model_id).await.unwrap_or(false) {
                    if !observed_running {
                        observed_running = true;
                        append_log(job_id, "step", "Model is loaded; waiting for Ollama warmup to finish");
                    }
                    emit_preload_progress(job_id, "loaded-waiting", 95, &mut last_progress);
                } else {
                    poll_count = poll_count.saturating_add(1);
                    let percent = 10u8.saturating_add(poll_count).min(PRELOAD_MAX_LOADING_PERCENT);
                    emit_preload_progress(job_id, "loading-model", percent, &mut last_progress);
                }
            }
        }
    }
}

async fn unload_after_preload_cancel(job_id: &str, model_id: &str, observed_running: bool) {
    let should_unload = observed_running
        || local_llm::is_ollama_model_running(model_id)
            .await
            .unwrap_or(false);
    if !should_unload {
        return;
    }
    append_log(
        job_id,
        "step",
        "Cancellation observed; unloading model from Ollama",
    );
    if let Err(e) = local_llm::stop_ollama_model(model_id).await {
        app_warn!(
            "local_model_jobs",
            "preload_cancel",
            "Failed to unload model {} after preload cancellation: {}",
            model_id,
            e
        );
    }
}

fn emit_preload_progress(
    job_id: &str,
    phase: &'static str,
    percent: u8,
    last: &mut (&'static str, u8),
) {
    if *last == (phase, percent) {
        return;
    }
    *last = (phase, percent);
    update_job(
        job_id,
        LocalModelJobStatus::Running,
        phase,
        Some(percent),
        None,
        None,
    );
}

async fn install_ollama_only(job_id: &str, cancel_token: &CancellationToken) -> Result<Value> {
    update_job(
        job_id,
        LocalModelJobStatus::Running,
        "checking-ollama",
        Some(0),
        None,
        None,
    );
    let mut status = local_llm::detect_ollama().await;
    if cancel_token.is_cancelled() {
        return Err(anyhow!("Local model job was cancelled"));
    }

    if status.phase == OllamaPhase::NotInstalled {
        append_log(job_id, "step", "Install Ollama");
        update_job(
            job_id,
            LocalModelJobStatus::Running,
            "install-ollama",
            Some(0),
            None,
            None,
        );
        let job_id_for_progress = job_id.to_string();
        install_ollama_via_script_cancellable(
            move |progress| handle_install_progress(&job_id_for_progress, progress),
            cancel_token.clone(),
        )
        .await?;
        status = local_llm::detect_ollama().await;
    }

    if status.phase == OllamaPhase::NotInstalled {
        return Err(anyhow!(
            "Ollama installation finished but Ollama was not detected"
        ));
    }

    if status.phase != OllamaPhase::Running {
        append_log(job_id, "step", "Start Ollama");
        update_job(
            job_id,
            LocalModelJobStatus::Running,
            "start-ollama",
            Some(80),
            None,
            None,
        );
        tokio::select! {
            result = start_ollama() => result?,
            _ = cancel_token.cancelled() => return Err(anyhow!("Local model job was cancelled")),
        }
        status = local_llm::detect_ollama().await;
    }

    update_job(
        job_id,
        LocalModelJobStatus::Running,
        "done",
        Some(100),
        None,
        None,
    );
    serde_json::to_value(status).map_err(Into::into)
}

async fn run_common_setup(job_id: &str, cancel_token: &CancellationToken) -> Result<()> {
    update_job(
        job_id,
        LocalModelJobStatus::Running,
        "checking-ollama",
        Some(0),
        None,
        None,
    );
    let mut status = local_llm::detect_ollama().await;
    if cancel_token.is_cancelled() {
        return Err(anyhow!("Local model job was cancelled"));
    }

    if status.phase == OllamaPhase::NotInstalled {
        append_log(job_id, "step", "Install Ollama");
        update_job(
            job_id,
            LocalModelJobStatus::Running,
            "install-ollama",
            Some(0),
            None,
            None,
        );
        let job_id_for_progress = job_id.to_string();
        install_ollama_via_script_cancellable(
            move |progress| handle_install_progress(&job_id_for_progress, progress),
            cancel_token.clone(),
        )
        .await?;
        status = local_llm::detect_ollama().await;
    }

    if status.phase != OllamaPhase::Running {
        append_log(job_id, "step", "Start Ollama");
        update_job(
            job_id,
            LocalModelJobStatus::Running,
            "start-ollama",
            Some(5),
            None,
            None,
        );
        tokio::select! {
            result = start_ollama() => result?,
            _ = cancel_token.cancelled() => return Err(anyhow!("Local model job was cancelled")),
        }
    }

    Ok(())
}

fn handle_install_progress(job_id: &str, progress: &InstallScriptProgress) {
    match progress.kind {
        InstallScriptKind::Step => {
            update_job(
                job_id,
                LocalModelJobStatus::Running,
                &progress.message,
                None,
                None,
                None,
            );
            append_log(job_id, "step", &progress.message);
        }
        InstallScriptKind::Log => append_log(job_id, "log", &progress.message),
        InstallScriptKind::Error => {
            append_log(job_id, "error", &progress.message);
            update_job(
                job_id,
                LocalModelJobStatus::Running,
                "install-ollama",
                None,
                Some(progress.message.clone()),
                None,
            );
        }
    }
}

fn handle_pull_progress(
    job_id: &str,
    progress: &PullProgress,
    throttle: &Arc<Mutex<ProgressThrottle>>,
) {
    {
        let mut guard = throttle.lock().unwrap_or_else(|p| p.into_inner());
        if !guard.should_emit(&progress.phase, progress.percent, progress.bytes_completed) {
            return;
        }
    }
    update_job_with_bytes(
        job_id,
        LocalModelJobStatus::Running,
        &progress.phase,
        progress.percent,
        progress.bytes_completed,
        progress.bytes_total,
        None,
        None,
    );
    let suffix = progress
        .percent
        .map(|p| format!(" {p}%"))
        .unwrap_or_default();
    append_log(job_id, "log", &format!("{}{}", progress.phase, suffix));
}
