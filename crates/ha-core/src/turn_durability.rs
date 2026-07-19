//! Runtime-neutral durability contract for conversation-producing agent turns.
//!
//! This module intentionally sits above both `agent` and `chat_engine`: the
//! agent tool loop can require persistence barriers without depending on a
//! shell-specific engine implementation.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlushReason {
    Timer,
    SizeThreshold,
    RoleSwitch,
    ToolBoundary,
    ToolResultBoundary,
    RoundEnd,
    Stop,
    Failure,
    FinalEnd,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamSnapshot {
    pub session_id: String,
    pub stream_id: Option<String>,
    pub turn_id: Option<String>,
    pub persistence_run_id: String,
    pub accepted_seq: u64,
    pub durable_seq: u64,
    pub committed_seq: u64,
    pub status: String,
    pub events: Vec<crate::session::JournalEvent>,
}

#[async_trait]
pub trait TurnDurabilitySink: Send + Sync + 'static {
    /// Accept one raw provider/agent event. Implementations must only parse and
    /// enqueue under a short lock; durable IO belongs to the background writer.
    fn accept_event(&self, raw_event: &str) -> Result<u64>;

    /// Wait until every event accepted before this call is durable.
    async fn flush(&self, reason: FlushReason) -> Result<u64>;

    /// Persist provider-native context at a semantic boundary with revision
    /// compare-and-swap. Returns the new authoritative revision.
    async fn checkpoint_context(
        &self,
        history: &[serde_json::Value],
        expected_revision: i64,
    ) -> Result<i64>;

    /// Mark a failed failover attempt without deleting its journal.
    async fn supersede_attempt(&self, error: Option<&str>) -> Result<()>;

    /// Switch to a new provider/profile attempt within the same run.
    async fn begin_attempt(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
        provider_shape: Option<&str>,
    ) -> Result<u32>;

    fn persistence_run_id(&self) -> &str;
    fn current_attempt_no(&self) -> u32;
    fn context_revision(&self) -> i64;
    fn snapshot(&self) -> StreamSnapshot;
}
