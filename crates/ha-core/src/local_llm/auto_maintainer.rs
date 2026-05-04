//! Background watchdog that keeps default chat / embedding Ollama models
//! preloaded and surfaces missing-file alerts to the user.
//!
//! Three guarantees:
//! 1. **Self-heal**: every [`SWEEP_INTERVAL`] tick, if the default chat or
//!    embedding model is `enabled`, has a real Ollama backend, exists in
//!    `/api/tags` but isn't `running` in `/api/ps`, and the user did not
//!    explicitly stop it via the UI, we call [`preload_ollama_model`] to
//!    pin it back into runtime memory (`keep_alive=-1`).
//! 2. **Respect user intent**: a model in
//!    `AppConfig.local_llm.user_stopped_models` is never auto-preloaded.
//!    [`super::management::stop_ollama_model`] adds tags to that list,
//!    [`preload_ollama_model`] removes them when the user starts a model
//!    manually.
//! 3. **Surface missing files**: when the default model's tag is no longer
//!    in `/api/tags` (likely external `ollama rm`), emit a
//!    `local_model:missing_alert` EventBus event so the frontend can show
//!    a top-level dialog with redownload / switch / disable options. Each
//!    tag is rate-limited by a 5-minute in-memory cooldown plus a process-
//!    lifetime "silence" set so the user can postpone or dismiss without
//!    being spammed.
//!
//! Public API:
//! - [`spawn_loop`]: idempotent — call once from `app_init`.
//! - [`trigger`]: nudge the loop to run immediately (after model swap /
//!   redownload completion).
//! - [`dismiss_alert_temporary`] / [`silence_for_session`]: dismiss-button
//!   handlers from the frontend.
//! - [`disable_auto_maintenance`]: kill switch (also exposed via the
//!   regular settings panel toggle).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use once_cell::sync::Lazy;
use serde::Serialize;
use tokio::sync::{Mutex, Notify};

use super::management::{list_local_ollama_models, preload_ollama_model, LocalOllamaModel};
use super::types::model_catalog;
use crate::config::{cached_config, AppConfig};
use crate::local_embedding::embedding_model_catalog;
use crate::provider::provider_matches_known_local_backend;

/// Watchdog sweep cadence. Per-cycle work is one `/api/tags` + `/api/ps`
/// fan-out (already concurrent) plus at most two `/api/embed|generate`
/// preload calls — light enough to run every minute without measurable
/// load on Ollama.
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// "Dismiss for 5 min" cooldown. After the user clicks the temporary
/// dismiss button (or the watchdog emits the very first alert for a tag),
/// we suppress further alerts for the same tag until this window elapses.
const DISMISS_COOLDOWN: Duration = Duration::from_secs(300);

static TRIGGER: Lazy<Arc<Notify>> = Lazy::new(|| Arc::new(Notify::new()));
static ALERT_COOLDOWN: Lazy<Mutex<HashMap<String, Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static SESSION_SILENCED: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static LOOP_SPAWNED: AtomicBool = AtomicBool::new(false);

// ── Public types (mirrored on the TS side) ─────────────────────────

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ModelKind {
    Chat,
    Embedding,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelMissingAlert {
    pub kind: ModelKind,
    pub missing_model_id: String,
    pub missing_display_name: String,
    pub alternatives: Vec<MissingAlertAlternative>,
    /// Whether the missing tag is in our built-in catalog so the
    /// "Redownload" action can spawn an install job. Externally-imported
    /// (catalog-less) Ollama models cannot be auto-redownloaded.
    pub can_redownload: bool,
    /// Only `true` for the embedding kind; surfaces the
    /// "Disable vector search" escape hatch.
    pub can_disable_embedding: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingAlertAlternative {
    pub model_id: String,
    pub display_name: String,
    /// Set for chat alternatives — the provider that hosts this model in
    /// the current `AppConfig.providers` list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// Set for embedding alternatives — the `embedding_models[*].id` to
    /// pass into [`crate::memory::set_memory_embedding_default`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_config_id: Option<String>,
}

// ── Public API ─────────────────────────────────────────────────────

pub fn spawn_loop() {
    if LOOP_SPAWNED.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        crate::app_info!(
            "local_llm",
            "auto_maintainer",
            "Auto-maintenance watchdog started (sweep every {}s)",
            SWEEP_INTERVAL.as_secs(),
        );
        loop {
            tokio::select! {
                _ = tokio::time::sleep(SWEEP_INTERVAL) => {},
                _ = TRIGGER.notified() => {},
            }
            run_one_pass().await;
        }
    });
}

/// Wake the watchdog now (instead of waiting for the next 60s tick).
/// Called by the user-driven model-swap / redownload paths so the
/// preload kicks in immediately rather than after a minute of "off".
pub fn trigger() {
    TRIGGER.notify_one();
}

pub async fn dismiss_alert_temporary(model_id: &str) {
    let mut cooldown = ALERT_COOLDOWN.lock().await;
    cooldown.insert(model_id.to_string(), Instant::now());
    crate::app_info!(
        "local_llm",
        "auto_maintainer",
        "Missing alert cooldown bumped: tag={} duration={}s",
        model_id,
        DISMISS_COOLDOWN.as_secs()
    );
}

pub async fn silence_for_session(model_id: &str) {
    let mut silenced = SESSION_SILENCED.lock().await;
    silenced.insert(model_id.to_string());
    crate::app_info!(
        "local_llm",
        "auto_maintainer",
        "Missing alert silenced for this session: tag={}",
        model_id
    );
}

pub fn get_auto_maintenance_enabled() -> bool {
    cached_config().local_llm.auto_maintenance.enabled
}

pub fn set_auto_maintenance_enabled(enabled: bool) -> Result<()> {
    write_auto_maintenance_enabled(enabled, "settings_panel")
}

/// Same as `set_auto_maintenance_enabled(false)` but tags the autosave
/// snapshot as coming from the missing-model alert dialog so audits can
/// distinguish "user flipped settings toggle" from "user clicked Turn off
/// in alert dialog".
pub fn disable_via_alert_dialog() -> Result<()> {
    write_auto_maintenance_enabled(false, "missing_alert_dialog")
}

fn write_auto_maintenance_enabled(enabled: bool, source: &'static str) -> Result<()> {
    crate::config::mutate_config(("local_llm.auto_maintenance", source), |cfg| {
        cfg.local_llm.auto_maintenance.enabled = enabled;
        Ok(())
    })
    .map(|_| ())
}

// ── Loop body ──────────────────────────────────────────────────────

async fn run_one_pass() {
    let cfg = cached_config();
    if !cfg.local_llm.auto_maintenance.enabled {
        return;
    }
    // If the daemon is down, try to bring it up. start_ollama() is idempotent
    // (returns Ok early when ping succeeds) and will attempt to spawn ollama
    // serve when it doesn't. Failing here means Ollama isn't installed or the
    // binary is unusable — the user has to fix that themselves; nothing the
    // watchdog can do.
    if let Err(e) = super::start_ollama().await {
        crate::app_warn!(
            "local_llm",
            "auto_maintainer",
            "Skipping pass — Ollama is unreachable and could not be auto-started: {:#}",
            e
        );
        return;
    }
    let installed = match list_local_ollama_models().await {
        Ok(v) => v,
        Err(e) => {
            crate::app_warn!(
                "local_llm",
                "auto_maintainer",
                "list_local_ollama_models failed: {:#}",
                e
            );
            return;
        }
    };
    // Both checks only read shared state and either emit an alert or
    // call preload — independent paths. tokio::join! lets a slow chat
    // preload (network hang) not push the embedding check past its 60s
    // sweep window.
    tokio::join!(
        run_check(ModelKind::Chat, &cfg, &installed),
        run_check(ModelKind::Embedding, &cfg, &installed),
    );
}

/// Resolve the default tag for a model kind from `AppConfig`. Returns the
/// Ollama tag and a display name, or `None` when there is no default to
/// maintain (no active model, non-Ollama provider, embedding disabled).
fn resolve_default_tag(kind: ModelKind, cfg: &AppConfig) -> Option<(String, String)> {
    match kind {
        ModelKind::Chat => {
            let active = cfg.active_model.as_ref()?;
            let provider = cfg.providers.iter().find(|p| p.id == active.provider_id)?;
            if !provider_matches_known_local_backend(provider, "ollama") {
                return None;
            }
            let tag = active.model_id.clone();
            let display_name = provider
                .models
                .iter()
                .find(|m| m.id == tag)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| tag.clone());
            Some((tag, display_name))
        }
        ModelKind::Embedding => {
            if !cfg.memory_embedding.enabled {
                return None;
            }
            let model_cfg_id = cfg.memory_embedding.model_config_id.as_deref()?;
            let em = cfg.embedding_models.iter().find(|m| m.id == model_cfg_id)?;
            if em.source.as_deref() != Some("ollama") {
                return None;
            }
            let tag = em.api_model.as_deref()?.to_string();
            Some((tag, em.name.clone()))
        }
    }
}

async fn run_check(kind: ModelKind, cfg: &AppConfig, installed: &[LocalOllamaModel]) {
    let Some((tag, display_name)) = resolve_default_tag(kind, cfg) else {
        return;
    };
    match installed.iter().find(|m| m.id == tag) {
        None => {
            maybe_emit_missing_alert(cfg, installed, kind, &tag, &display_name).await;
        }
        Some(m) if m.running => {
            // Already running, nothing to do.
        }
        Some(_) => {
            if cfg.local_llm.user_stopped_models.iter().any(|m| m == &tag) {
                return;
            }
            match preload_ollama_model(&tag).await {
                Ok(_) => crate::app_info!(
                    "local_llm",
                    "auto_maintainer",
                    "Auto-preloaded default {} model: {}",
                    kind.as_log_label(),
                    tag
                ),
                Err(e) => crate::app_warn!(
                    "local_llm",
                    "auto_maintainer",
                    "Auto-preload {} model failed: tag={} error={:#}",
                    kind.as_log_label(),
                    tag,
                    e
                ),
            }
        }
    }
}

impl ModelKind {
    fn as_log_label(self) -> &'static str {
        match self {
            ModelKind::Chat => "chat",
            ModelKind::Embedding => "embedding",
        }
    }
}

// ── Missing alert emission ─────────────────────────────────────────

async fn maybe_emit_missing_alert(
    cfg: &AppConfig,
    installed: &[LocalOllamaModel],
    kind: ModelKind,
    missing_tag: &str,
    missing_display_name: &str,
) {
    // Process-lifetime silence wins over any cooldown / config check.
    {
        let silenced = SESSION_SILENCED.lock().await;
        if silenced.contains(missing_tag) {
            return;
        }
    }
    // 5 min cooldown gate. Bump the timestamp so subsequent ticks within
    // the window stay quiet — same effect as the user clicking "Dismiss
    // for 5 min" on the very first emission.
    {
        let mut cooldown = ALERT_COOLDOWN.lock().await;
        if let Some(last) = cooldown.get(missing_tag) {
            if last.elapsed() < DISMISS_COOLDOWN {
                return;
            }
        }
        cooldown.insert(missing_tag.to_string(), Instant::now());
    }

    let alternatives = match kind {
        ModelKind::Chat => collect_chat_alternatives(cfg, installed, missing_tag),
        ModelKind::Embedding => collect_embedding_alternatives(cfg, installed, missing_tag),
    };
    let can_redownload = match kind {
        ModelKind::Chat => model_catalog().iter().any(|m| m.id == missing_tag),
        ModelKind::Embedding => embedding_model_catalog().iter().any(|m| m.id == missing_tag),
    };

    let alert = LocalModelMissingAlert {
        kind,
        missing_model_id: missing_tag.to_string(),
        missing_display_name: missing_display_name.to_string(),
        alternatives,
        can_redownload,
        can_disable_embedding: matches!(kind, ModelKind::Embedding),
    };

    crate::app_info!(
        "local_llm",
        "auto_maintainer",
        "Emitting missing_alert: kind={} tag={} alternatives={} canRedownload={}",
        alert.kind.as_log_label(),
        alert.missing_model_id,
        alert.alternatives.len(),
        alert.can_redownload
    );

    if let Some(bus) = crate::get_event_bus() {
        let payload = serde_json::to_value(&alert).unwrap_or(serde_json::Value::Null);
        bus.emit("local_model:missing_alert", payload);
    }
}

fn collect_chat_alternatives(
    cfg: &AppConfig,
    installed: &[LocalOllamaModel],
    missing_tag: &str,
) -> Vec<MissingAlertAlternative> {
    let mut out = Vec::new();
    for provider in &cfg.providers {
        if !provider.enabled {
            continue;
        }
        if !provider_matches_known_local_backend(provider, "ollama") {
            continue;
        }
        for model in &provider.models {
            if model.id == missing_tag {
                continue;
            }
            if !installed.iter().any(|m| m.id == model.id) {
                continue;
            }
            out.push(MissingAlertAlternative {
                model_id: model.id.clone(),
                display_name: model.name.clone(),
                provider_id: Some(provider.id.clone()),
                embedding_config_id: None,
            });
        }
    }
    out
}

fn collect_embedding_alternatives(
    cfg: &AppConfig,
    installed: &[LocalOllamaModel],
    missing_tag: &str,
) -> Vec<MissingAlertAlternative> {
    let mut out = Vec::new();
    for em in &cfg.embedding_models {
        if em.source.as_deref() != Some("ollama") {
            continue;
        }
        let Some(api_model) = em.api_model.as_deref() else {
            continue;
        };
        if api_model == missing_tag {
            continue;
        }
        if !installed.iter().any(|m| m.id == api_model) {
            continue;
        }
        out.push(MissingAlertAlternative {
            model_id: api_model.to_string(),
            display_name: em.name.clone(),
            provider_id: None,
            embedding_config_id: Some(em.id.clone()),
        });
    }
    out
}
