//! Stable trigger contract for the feature-owned Dreaming scheduler.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use super::config::DreamingConfig;
use super::types::DreamReport;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DreamTrigger {
    Idle,
    Cron,
    Manual,
}

impl DreamTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Cron => "cron",
            Self::Manual => "manual",
        }
    }
}

pub type DreamingTriggerFuture = Pin<Box<dyn Future<Output = DreamReport> + Send + 'static>>;

#[derive(Clone, Copy)]
pub struct DreamingTriggerRuntime {
    pub dreaming_running: fn() -> bool,
    pub last_activity_epoch_secs: fn() -> i64,
    pub touch_activity: fn(),
    pub check_idle_trigger: fn(&DreamingConfig) -> bool,
    pub manual_run: fn(DreamTrigger) -> DreamingTriggerFuture,
    pub spawn_cron_loop: fn(),
}

static TRIGGER_RUNTIME: OnceLock<DreamingTriggerRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_dreaming_trigger_runtime(
    runtime: DreamingTriggerRuntime,
) -> Result<(), &'static str> {
    TRIGGER_RUNTIME
        .set(runtime)
        .map_err(|_| "Dreaming trigger runtime already registered")
}

fn runtime() -> Option<&'static DreamingTriggerRuntime> {
    let runtime = TRIGGER_RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "dreaming_trigger_runtime_unavailable",
            "Dreaming trigger runtime is not wired; background cycles are disabled"
        );
    }
    runtime
}

pub fn dreaming_running() -> bool {
    runtime().is_some_and(|runtime| (runtime.dreaming_running)())
}

pub fn last_activity_epoch_secs() -> i64 {
    runtime()
        .map(|runtime| (runtime.last_activity_epoch_secs)())
        .unwrap_or(0)
}

pub fn touch_activity() {
    if let Some(runtime) = runtime() {
        (runtime.touch_activity)();
    }
}

pub fn check_idle_trigger(cfg: &DreamingConfig) -> bool {
    runtime().is_some_and(|runtime| (runtime.check_idle_trigger)(cfg))
}

pub async fn manual_run(trigger: DreamTrigger) -> DreamReport {
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
            note: Some("Dreaming trigger runtime is not wired".to_string()),
        };
    };
    (runtime.manual_run)(trigger).await
}

pub fn spawn_dreaming_cron_loop() {
    if let Some(runtime) = runtime() {
        (runtime.spawn_cron_loop)();
    }
}
