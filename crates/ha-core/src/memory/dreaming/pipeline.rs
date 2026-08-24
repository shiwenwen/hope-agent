//! Kernel contract for the feature-owned Dreaming cycle machine.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use super::config::DreamingConfig;
use super::triggers::DreamTrigger;
use super::types::DreamReport;

pub type DreamingPipelineFuture = Pin<Box<dyn Future<Output = DreamReport> + Send + 'static>>;

#[derive(Clone, Copy)]
pub struct DreamingPipelineRuntime {
    pub last_report_snapshot: fn() -> Option<DreamReport>,
    pub run_cycle: fn(DreamTrigger) -> DreamingPipelineFuture,
}

static PIPELINE_RUNTIME: OnceLock<DreamingPipelineRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_dreaming_pipeline_runtime(
    runtime: DreamingPipelineRuntime,
) -> Result<(), &'static str> {
    PIPELINE_RUNTIME
        .set(runtime)
        .map_err(|_| "Dreaming pipeline runtime already registered")
}

fn runtime() -> Option<&'static DreamingPipelineRuntime> {
    let runtime = PIPELINE_RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "dreaming_pipeline_runtime_unavailable",
            "Dreaming pipeline runtime is not wired; cycles are unavailable"
        );
    }
    runtime
}

pub fn last_report_snapshot() -> Option<DreamReport> {
    runtime().and_then(|runtime| (runtime.last_report_snapshot)())
}

pub async fn run_cycle(trigger: DreamTrigger) -> DreamReport {
    let started = Instant::now();
    let Some(runtime) = runtime() else {
        return DreamReport {
            run_id: None,
            trigger,
            candidates_scanned: 0,
            candidates_nominated: 0,
            promoted: Vec::new(),
            diary_path: None,
            duration_ms: started.elapsed().as_millis() as u64,
            note: Some("Dreaming pipeline runtime is not wired".to_string()),
        };
    };
    (runtime.run_cycle)(trigger).await
}

/// Resolve the shared automation chain from stable configuration contracts.
/// This policy remains in the kernel so Light Dreaming and Deep Resolver use
/// one model selection rule without depending on each other's implementation.
#[doc(hidden)]
pub fn resolve_dreaming_chain(cfg: &DreamingConfig) -> Vec<crate::provider::ActiveModel> {
    let app_cfg = crate::config::cached_config();
    let override_chain = cfg
        .model_override
        .clone()
        .or_else(|| crate::automation::parse_legacy_model_string(cfg.narrative_model.as_deref()?));
    crate::automation::effective_chain(&app_cfg, override_chain)
}
