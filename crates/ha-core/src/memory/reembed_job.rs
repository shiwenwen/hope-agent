//! Kernel contract for the feature-owned memory re-embedding machine.

use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::local_model_jobs::LocalModelJobSnapshot;

pub const PHASE_REEMBED_KEEP: &str = "reembed-keep";
pub const PHASE_REEMBED_FRESH: &str = "reembed-fresh";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReembedMode {
    #[default]
    KeepExisting,
    DeleteAll,
}

#[derive(Clone, Copy)]
pub struct MemoryReembedRuntime {
    pub cancel_active: fn(),
    pub start: fn(&str, ReembedMode, Option<&str>) -> Result<LocalModelJobSnapshot>,
}

static RUNTIME: OnceLock<MemoryReembedRuntime> = OnceLock::new();

pub fn register_reembed_runtime(
    runtime: MemoryReembedRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("memory reembed runtime"))
}

pub fn cancel_active_memory_reembed_jobs() {
    if let Some(runtime) = RUNTIME.get() {
        (runtime.cancel_active)();
    }
}

pub fn start_memory_reembed_job(
    model_config_id: &str,
    mode: ReembedMode,
    parent_job_id: Option<&str>,
) -> Result<LocalModelJobSnapshot> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| anyhow!("memory reembed runtime is not wired"))?;
    (runtime.start)(model_config_id, mode, parent_job_id)
}
