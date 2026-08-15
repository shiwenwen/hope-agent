//! Runtime-neutral durability contract for conversation-producing agent turns.
//!
//! This module intentionally sits above both `agent` and `chat_engine`: the
//! agent tool loop can require persistence barriers without depending on a
//! shell-specific engine implementation.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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

/// Provider request role. Auxiliary model calls must never mutate the main
/// conversation's projection head or terminal assistant checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableRequestRole {
    MainContinuation,
    Tier3SummaryInput,
    SideQuery,
}

/// Body-free description of one request-only history degradation. The source
/// guard and replacement fingerprint are installation-keyed identifiers; they
/// are locators/integrity evidence, never capabilities or recoverable bodies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DurableProjectionItem {
    pub projection_item_key: String,
    pub result_id: Option<String>,
    pub stable_ordinal: u64,
    pub action: String,
    pub source_guard: String,
    pub replacement_fingerprint: String,
}

/// Content-free identity of the exact provider body that is about to be
/// dispatched. The exact bytes are supplied separately to the durability sink
/// so implementations cannot accidentally serialize them into diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepareRequestPlan {
    pub request_plan_id: String,
    pub role: DurableRequestRole,
    pub provider_id: String,
    pub provider_profile_id: Option<String>,
    pub model_id: String,
    pub endpoint_kind: String,
    pub request_shape: String,
    pub content_type: String,
    pub cache_identity_hash: String,
    pub body_keyed_fingerprint: String,
    pub body_len: u64,
    pub round: u32,
    pub final_capacity_count_json: String,
    pub projection: Vec<DurableProjectionItem>,
    /// Set only when this exact request needed deterministic old-history
    /// projection to fit. The durability sink publishes the follow-up marker
    /// with the request plan's `ContextCommitted` transition.
    pub tier3_followup_after_capacity_projection: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchClaim {
    pub request_attempt_id: String,
    pub provider_idempotency_key: Option<String>,
    /// The independently prepared body identity observed at the adapter's
    /// final pre-send boundary. The durable claim CASes all four fields
    /// against the plan, preventing a rebuilt or rerouted body from borrowing
    /// an older plan's send authorization.
    pub body_keyed_fingerprint: String,
    pub body_len: u64,
    pub endpoint_kind: String,
    pub content_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseStarted {
    pub provider_attempt: u32,
    pub status: u16,
    pub provider_request_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestTerminalOutcome {
    Success,
    ProviderRejected,
    CancelledBeforeSend,
    CancelledAfterResponse,
    ResponseIncomplete,
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

    /// Persist Tier 4 history and its required Tier 3 follow-up at the same
    /// durable boundary.
    async fn checkpoint_emergency_context(
        &self,
        history: &[serde_json::Value],
        expected_revision: i64,
    ) -> Result<i64>;

    /// Publish a winning Tier 3 summary and clear any Tier 4 follow-up marker
    /// in the same durable checkpoint transaction.
    async fn checkpoint_summarized_context(
        &self,
        history: &[serde_json::Value],
        expected_revision: i64,
    ) -> Result<i64>;

    /// Stage an exact main/auxiliary request and publish its body-free plan at
    /// the current canonical revision. Persistent implementations may retain
    /// `exact_body` only through the kernel-private encrypted payload store;
    /// when that capability is unavailable they must persist an explicit
    /// non-recoverable plan rather than plaintext or a filesystem path.
    async fn prepare_request_plan(
        &self,
        input: &PrepareRequestPlan,
        exact_body: Arc<[u8]>,
    ) -> Result<()>;

    /// Durable write-ahead claim. This method must return successfully before
    /// the Provider adapter is allowed to poll network I/O.
    async fn claim_request_dispatch(
        &self,
        request_plan_id: &str,
        claim: &DispatchClaim,
    ) -> Result<()>;

    /// Record response headers before the error/SSE body is consumed.
    async fn mark_request_response_started(
        &self,
        request_plan_id: &str,
        response: &ResponseStarted,
    ) -> Result<()>;

    /// The body may have crossed the process boundary but no authoritative
    /// response proof exists. Such a plan is terminal for automatic replay.
    async fn mark_request_send_unknown(
        &self,
        request_plan_id: &str,
        diagnostic: Option<&str>,
    ) -> Result<()>;

    /// Close a request whose Provider outcome is known. Response/SSE journal
    /// durability remains a separate barrier owned by the caller.
    async fn mark_request_terminal(
        &self,
        request_plan_id: &str,
        outcome: RequestTerminalOutcome,
    ) -> Result<()>;

    /// Abandon a plan only while it is provably pre-dispatch. Implementations
    /// must reject attempts to supersede dispatching/response/send-unknown
    /// plans.
    async fn supersede_request_plan(&self, request_plan_id: &str) -> Result<()>;

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
    /// True once any attempt in this turn has crossed any tool boundary.
    /// Same-model/profile retry and whole-chain restart use this to preserve
    /// completed tool context instead of superseding it with a fresh attempt.
    fn had_tool_activity(&self) -> bool;
    /// True once any attempt has crossed a non-replayable tool boundary.
    /// Forward-only model fallback uses this as the side-effect barrier:
    /// mutating or otherwise unsafe work must not be repeated on the next
    /// configured model. Events without explicit replay-safety metadata remain
    /// conservative and set the barrier.
    fn had_non_replayable_tool_activity(&self) -> bool;
    fn snapshot(&self) -> StreamSnapshot;
}
