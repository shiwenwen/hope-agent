//! Kernel contract and stable result type for memory profile synthesis.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use serde::Serialize;

use super::triggers::DreamTrigger;

/// Terminal summary of a profile-synthesis cycle.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub scanned: usize,
    pub scopes: usize,
    pub snapshots_written: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ProfileReport {
    #[doc(hidden)]
    pub fn skipped(note: &str, started: Instant) -> Self {
        Self {
            run_id: None,
            scanned: 0,
            scopes: 0,
            snapshots_written: 0,
            duration_ms: started.elapsed().as_millis() as u64,
            note: Some(note.to_string()),
        }
    }
}

pub type ProfileRuntimeFuture = Pin<Box<dyn Future<Output = ProfileReport> + Send + 'static>>;

#[derive(Clone, Copy)]
pub struct DreamingProfileRuntime {
    pub run_cycle: fn(DreamTrigger) -> ProfileRuntimeFuture,
}

static PROFILE_RUNTIME: OnceLock<DreamingProfileRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_dreaming_profile_runtime(
    runtime: DreamingProfileRuntime,
) -> Result<(), &'static str> {
    PROFILE_RUNTIME
        .set(runtime)
        .map_err(|_| "Dreaming profile runtime already registered")
}

fn runtime() -> Option<&'static DreamingProfileRuntime> {
    let runtime = PROFILE_RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "dreaming_profile_runtime_unavailable",
            "Dreaming profile runtime is not wired; profile synthesis is unavailable"
        );
    }
    runtime
}

pub async fn run_profile_synthesis_cycle(trigger: DreamTrigger) -> ProfileReport {
    let started = Instant::now();
    let Some(runtime) = runtime() else {
        return ProfileReport::skipped("Dreaming profile runtime is not wired", started);
    };
    (runtime.run_cycle)(trigger).await
}
