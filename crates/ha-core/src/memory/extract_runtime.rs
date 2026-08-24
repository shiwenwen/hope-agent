//! Kernel-owned contract for the optional memory extraction machine.
//!
//! The implementation is registered by `ha-memory`; this module deliberately
//! contains no extraction prompts, provider construction or background
//! scheduling policy. Unwired extraction is an explicit, observable no-op so
//! a reduced kernel build can still run without silently pretending to persist
//! memories.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use serde_json::Value;

use crate::provider::ActiveModel;
use crate::session::SessionDB;

pub type MemoryExtractFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
pub type TrackedExtractionFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

#[derive(Clone, Copy)]
pub struct MemoryExtractRuntime {
    pub run_extraction: for<'a> fn(
        &'a [Value],
        &'a str,
        &'a str,
        &'a ActiveModel,
        Option<Arc<SessionDB>>,
    ) -> MemoryExtractFuture<'a, ()>,
    pub flush_before_compact: for<'a> fn(
        &'a [Value],
        &'a str,
        &'a str,
        &'a ActiveModel,
        Option<Arc<SessionDB>>,
    ) -> MemoryExtractFuture<'a, Result<usize>>,
    pub spawn_tracked_extraction: fn(String, TrackedExtractionFuture),
    pub cancel_active_extractions: fn(&str) -> usize,
    pub cancel_idle_extraction: fn(&str) -> bool,
    pub schedule_idle_extraction: fn(String, String, String, u64),
    pub flush_all_idle_extractions: fn(),
}

static RUNTIME: OnceLock<MemoryExtractRuntime> = OnceLock::new();
static WARNED_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register(runtime: MemoryExtractRuntime) -> std::result::Result<(), &'static str> {
    RUNTIME
        .set(runtime)
        .map_err(|_| "memory extraction runtime already registered")
}

fn runtime() -> Option<&'static MemoryExtractRuntime> {
    let runtime = RUNTIME.get();
    if runtime.is_none() && !WARNED_UNAVAILABLE.swap(true, Ordering::Relaxed) {
        app_warn!(
            "memory",
            "extract_runtime_unavailable",
            "Memory extraction runtime is not wired; optional extraction is disabled"
        );
    }
    runtime
}

pub async fn run_extraction(
    messages: &[Value],
    agent_id: &str,
    session_id: &str,
    model: &ActiveModel,
    session_db: Option<Arc<SessionDB>>,
) {
    let Some(runtime) = runtime() else {
        return;
    };
    (runtime.run_extraction)(messages, agent_id, session_id, model, session_db).await;
}

pub async fn flush_before_compact(
    messages_to_discard: &[Value],
    agent_id: &str,
    session_id: &str,
    model: &ActiveModel,
    session_db: Option<Arc<SessionDB>>,
) -> Result<usize> {
    let Some(runtime) = runtime() else {
        return Ok(0);
    };
    (runtime.flush_before_compact)(messages_to_discard, agent_id, session_id, model, session_db)
        .await
}

pub fn spawn_tracked_extraction<F>(session_id: String, future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    if let Some(runtime) = runtime() {
        (runtime.spawn_tracked_extraction)(session_id, Box::pin(future));
    }
}

pub fn cancel_active_extractions(session_id: &str) -> usize {
    runtime().map_or(0, |runtime| (runtime.cancel_active_extractions)(session_id))
}

pub fn cancel_idle_extraction(session_id: &str) -> bool {
    if let Some(runtime) = runtime() {
        return (runtime.cancel_idle_extraction)(session_id);
    }
    false
}

pub fn schedule_idle_extraction(
    agent_id: String,
    session_id: String,
    updated_at: String,
    idle_timeout_secs: u64,
) {
    if let Some(runtime) = runtime() {
        (runtime.schedule_idle_extraction)(agent_id, session_id, updated_at, idle_timeout_secs);
    }
}

pub fn flush_all_idle_extractions() {
    if let Some(runtime) = runtime() {
        (runtime.flush_all_idle_extractions)();
    }
}
