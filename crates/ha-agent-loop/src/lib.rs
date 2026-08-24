//! Provider-neutral mechanics for a multi-round agent loop.
//!
//! This crate intentionally has zero dependencies on Hope crates. Product
//! policy, provider payloads, tool execution, persistence, and event delivery
//! are supplied by the caller. Keeping the controller pure makes retry and
//! round-limit behavior deterministic and cheap to test.

use std::future::Future;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::future::join_all;

/// Configured round bound. A zero product setting maps to [`Self::Unlimited`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundLimit {
    /// No configured product limit. The internal counter still saturates.
    Unlimited,
    /// A non-zero configured limit.
    Limited(NonZeroU32),
}

impl RoundLimit {
    /// Convert the product's `0 = unlimited` representation.
    pub fn from_configured_max(value: u32) -> Self {
        NonZeroU32::new(value).map_or(Self::Unlimited, Self::Limited)
    }

    /// Return the configured value, or `None` when unlimited.
    pub fn configured_max(self) -> Option<NonZeroU32> {
        match self {
            Self::Unlimited => None,
            Self::Limited(value) => Some(value),
        }
    }
}

/// Immutable description of the round currently being attempted.
///
/// A provider retry reuses the same ticket. Only [`LoopController::advance`]
/// moves to the next round.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoundTicket {
    index: u32,
    final_round: bool,
}

impl RoundTicket {
    /// Zero-based round index.
    pub fn index(self) -> u32 {
        self.index
    }

    /// Whether this request is the last currently allowed round.
    pub fn is_final(self) -> bool {
        self.final_round
    }
}

/// Deterministic summary used by the outer runtime to select terminal copy and
/// persistence behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopTerminal {
    /// Number of distinct round indices that started. Retrying the same index
    /// does not increase this value.
    pub rounds_started: u32,
    /// Whether cancellation was observed by the product runtime.
    pub cancelled: bool,
    /// Whether the effective configured bound was reached.
    pub hit_round_limit: bool,
    /// Whether the bound, rather than a natural provider exit, ended the loop.
    pub rounds_exhausted: bool,
    /// Effective bound after one-time activation grace and queued-message
    /// continuation extensions.
    pub effective_max_rounds: u32,
}

/// Stable stages in the provider/tool state machine.
///
/// The names describe commit boundaries, not Hope implementation details.
/// Callers may persist a checkpoint, append provider-native history, or emit a
/// trace before acknowledging the corresponding transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopPhase {
    Ready,
    Streaming,
    ModelCommitted,
    ToolRunning,
    ToolCommitted,
    Completed,
    Cancelled,
    Failed,
}

/// Typed failure classes. Product runtimes may attach their own diagnostics,
/// but retry/finalization policy must not inspect error strings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopErrorKind {
    Cancelled,
    Provider,
    Tool,
    Checkpoint,
    Protocol,
    RoundLimit,
}

/// Provider-neutral loop failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoopError {
    kind: LoopErrorKind,
    phase: LoopPhase,
    message: &'static str,
}

impl LoopError {
    pub fn new(kind: LoopErrorKind, phase: LoopPhase, message: &'static str) -> Self {
        Self {
            kind,
            phase,
            message,
        }
    }

    pub fn kind(&self) -> LoopErrorKind {
        self.kind
    }

    pub fn phase(&self) -> LoopPhase {
        self.phase
    }
}

impl std::fmt::Display for LoopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message)
    }
}

impl std::error::Error for LoopError {}

/// Result of committing one complete model response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelRoundDecision {
    Complete,
    ExecuteTools,
}

/// Product-independent cancellation shared by every loop port.
#[derive(Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Complete provider result for one round. Native history remains opaque and
/// is passed back only to its provider adapter checkpoint.
pub struct ModelRound<N, C> {
    pub native: N,
    pub tool_calls: Vec<C>,
    /// Provider-native terminal signal. A terminal round never executes tool
    /// calls even if a malformed payload included them.
    pub terminal: bool,
}

/// Complete, input-ordered results of one tool batch.
pub struct ExecutedToolBatch<R> {
    pub results: Vec<R>,
}

/// Provider preparation and streaming port.
#[async_trait]
pub trait RoundDriver: Send {
    type PreparedRound: Send;
    type NativeRound: Send + Sync;
    type ToolCall: Send + Sync;

    async fn prepare_round(
        &mut self,
        ticket: RoundTicket,
    ) -> Result<Self::PreparedRound, LoopError>;

    async fn stream_round(
        &mut self,
        ticket: RoundTicket,
        prepared: Self::PreparedRound,
    ) -> Result<ModelRound<Self::NativeRound, Self::ToolCall>, LoopError>;
}

/// Product tool planner/executor port. The implementation, not this crate,
/// decides which calls may run concurrently and must return input order.
#[async_trait]
pub trait ToolBatchDriver<C>: Send
where
    C: Send + Sync,
{
    type ToolResult: Send + Sync;

    async fn execute_tool_batch(
        &mut self,
        ticket: RoundTicket,
        calls: Vec<C>,
        cancellation: &CancellationToken,
    ) -> Result<ExecutedToolBatch<Self::ToolResult>, LoopError>;
}

/// Durable/history checkpoint port. Model history is committed before tools;
/// tool results are committed before steering or another provider request.
#[async_trait]
pub trait LoopCheckpoint<N, R>: Send
where
    N: Send + Sync,
    R: Send + Sync,
{
    async fn commit_model_round(
        &mut self,
        ticket: RoundTicket,
        native: &N,
    ) -> Result<(), LoopError>;

    async fn commit_tool_batch(
        &mut self,
        ticket: RoundTicket,
        results: &[R],
    ) -> Result<(), LoopError>;
}

/// Durable steering/follow-up mailbox port.
#[async_trait]
pub trait SteeringDriver: Send {
    /// Returns the number of newly committed messages.
    async fn commit_steering(&mut self, ticket: RoundTicket) -> Result<usize, LoopError>;
}

/// Terminal returned by the generic port driver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DriveOutcome {
    pub terminal: LoopTerminal,
    pub phase: LoopPhase,
}

/// Drive the full provider → model checkpoint → tool → tool checkpoint →
/// steering loop using only narrow ports.
pub async fn drive_loop<D>(
    configured_max: u32,
    cancellation: &CancellationToken,
    driver: &mut D,
) -> Result<DriveOutcome, LoopError>
where
    D: RoundDriver
        + ToolBatchDriver<<D as RoundDriver>::ToolCall>
        + LoopCheckpoint<
            <D as RoundDriver>::NativeRound,
            <D as ToolBatchDriver<<D as RoundDriver>::ToolCall>>::ToolResult,
        > + SteeringDriver,
{
    let mut machine = LoopStateMachine::from_configured_max(configured_max);
    loop {
        if cancellation.is_cancelled() {
            let terminal = machine.cancel();
            return Ok(DriveOutcome {
                terminal,
                phase: machine.phase(),
            });
        }
        let Some(ticket) = machine.begin_round()? else {
            return Ok(DriveOutcome {
                terminal: machine.finish(false),
                phase: machine.phase(),
            });
        };

        let prepared = driver.prepare_round(ticket).await?;
        if cancellation.is_cancelled() {
            let terminal = machine.cancel();
            return Ok(DriveOutcome {
                terminal,
                phase: machine.phase(),
            });
        }
        let round = driver.stream_round(ticket, prepared).await?;
        if cancellation.is_cancelled() {
            let terminal = machine.cancel();
            return Ok(DriveOutcome {
                terminal,
                phase: machine.phase(),
            });
        }

        driver.commit_model_round(ticket, &round.native).await?;
        if cancellation.is_cancelled() {
            let terminal = machine.cancel();
            return Ok(DriveOutcome {
                terminal,
                phase: machine.phase(),
            });
        }
        let should_execute_tools = !round.terminal && !round.tool_calls.is_empty();
        match machine.commit_model_round(ticket, should_execute_tools)? {
            ModelRoundDecision::Complete => {
                return Ok(DriveOutcome {
                    terminal: machine.finish(false),
                    phase: machine.phase(),
                });
            }
            ModelRoundDecision::ExecuteTools => {}
        }

        machine.begin_tool_batch(ticket)?;
        let batch = driver
            .execute_tool_batch(ticket, round.tool_calls, cancellation)
            .await?;
        if cancellation.is_cancelled() {
            // The tool port still returns a complete batch for every planned
            // call. Commit it before publishing the cancelled terminal.
            driver.commit_tool_batch(ticket, &batch.results).await?;
            machine.commit_tool_batch(ticket)?;
            let terminal = machine.cancel();
            return Ok(DriveOutcome {
                terminal,
                phase: machine.phase(),
            });
        }

        driver.commit_tool_batch(ticket, &batch.results).await?;
        machine.commit_tool_batch(ticket)?;
        if cancellation.is_cancelled() {
            let terminal = machine.cancel();
            return Ok(DriveOutcome {
                terminal,
                phase: machine.phase(),
            });
        }
        let inserted = driver.commit_steering(ticket).await?;
        if cancellation.is_cancelled() {
            let terminal = machine.cancel();
            return Ok(DriveOutcome {
                terminal,
                phase: machine.phase(),
            });
        }
        machine.commit_steering(ticket, inserted)?;
    }
}

/// Pure round-state controller.
///
/// It deliberately does not inspect provider output or tool names. The outer
/// runtime decides when an activation or queued-message extension is valid and
/// reports only that fact here.
#[derive(Debug, Clone)]
pub struct LoopController {
    limit: RoundLimit,
    current_round: u32,
    effective_max_rounds: u32,
    rounds_started: u32,
    natural_exit: bool,
    activation_grace_used: bool,
}

impl LoopController {
    /// Build a controller from the product's `0 = unlimited` setting.
    pub fn from_configured_max(configured_max: u32) -> Self {
        let limit = RoundLimit::from_configured_max(configured_max);
        let effective_max_rounds = limit.configured_max().map_or(u32::MAX, NonZeroU32::get);
        Self {
            limit,
            current_round: 0,
            effective_max_rounds,
            rounds_started: 0,
            natural_exit: false,
            activation_grace_used: false,
        }
    }

    /// Whether the product configured a finite round limit.
    pub fn is_limited(&self) -> bool {
        matches!(self.limit, RoundLimit::Limited(_))
    }

    /// Original configured maximum, using `u32::MAX` for the legacy unlimited
    /// representation expected by the compatibility adapter.
    pub fn configured_max_or_unlimited(&self) -> u32 {
        self.limit
            .configured_max()
            .map_or(u32::MAX, NonZeroU32::get)
    }

    /// Effective bound after extensions.
    pub fn effective_max_rounds(&self) -> u32 {
        self.effective_max_rounds
    }

    /// Return the current round ticket, or `None` when the bound is exhausted.
    pub fn current_ticket(&self) -> Option<RoundTicket> {
        (self.current_round < self.effective_max_rounds).then(|| RoundTicket {
            index: self.current_round,
            final_round: self.is_final_round_index(self.current_round),
        })
    }

    /// Record that a provider attempt for this round began. Repeating this for
    /// a retry of the same ticket is idempotent.
    pub fn mark_started(&mut self, ticket: RoundTicket) {
        debug_assert_eq!(ticket.index, self.current_round);
        self.rounds_started = self.rounds_started.max(ticket.index.saturating_add(1));
    }

    /// Advance after the model round, tool batch, and all checkpoints settle.
    pub fn advance(&mut self, ticket: RoundTicket) {
        debug_assert_eq!(ticket.index, self.current_round);
        self.current_round = self.current_round.saturating_add(1);
    }

    /// Mark a provider-native natural exit. A later terminal report will not
    /// misclassify a final allowed text round as exhaustion.
    pub fn mark_natural_exit(&mut self) {
        self.natural_exit = true;
    }

    /// Grant the single bounded extension used after a newly discovered tool
    /// schema becomes available. Returns whether the bound changed.
    pub fn grant_activation_grace(&mut self) -> bool {
        if !self.is_limited() || self.activation_grace_used {
            return false;
        }
        self.effective_max_rounds = self.effective_max_rounds.saturating_add(1);
        self.activation_grace_used = true;
        true
    }

    /// Ensure messages accepted at the last tool boundary receive a real
    /// follow-up model round. Returns whether the bound changed.
    pub fn ensure_followup_after_insertion(
        &mut self,
        ticket: RoundTicket,
        inserted_count: usize,
    ) -> bool {
        if inserted_count == 0 || ticket.index.saturating_add(1) < self.effective_max_rounds {
            return false;
        }
        self.effective_max_rounds = self.effective_max_rounds.saturating_add(1);
        true
    }

    /// Whether `index` is the final round under the current effective bound.
    pub fn is_final_round_index(&self, index: u32) -> bool {
        index.saturating_add(1) == self.effective_max_rounds
    }

    /// Build the terminal classification after the outer runtime observes its
    /// cancellation source.
    pub fn finish(&self, cancelled: bool) -> LoopTerminal {
        let hit_round_limit =
            self.is_limited() && !cancelled && self.rounds_started >= self.effective_max_rounds;
        LoopTerminal {
            rounds_started: self.rounds_started,
            cancelled,
            hit_round_limit,
            rounds_exhausted: hit_round_limit && !self.natural_exit,
            effective_max_rounds: self.effective_max_rounds,
        }
    }
}

/// Authoritative phase machine layered over [`LoopController`].
///
/// A provider retry is the only transition that returns from `Streaming` to
/// `Ready` without advancing the round. A tool batch cannot start until the
/// model checkpoint is acknowledged, and the next model round cannot start
/// until both tool and steering checkpoints are acknowledged.
#[derive(Debug, Clone)]
pub struct LoopStateMachine {
    controller: LoopController,
    phase: LoopPhase,
    active_ticket: Option<RoundTicket>,
}

impl LoopStateMachine {
    pub fn from_configured_max(configured_max: u32) -> Self {
        Self {
            controller: LoopController::from_configured_max(configured_max),
            phase: LoopPhase::Ready,
            active_ticket: None,
        }
    }

    pub fn phase(&self) -> LoopPhase {
        self.phase
    }

    pub fn is_limited(&self) -> bool {
        self.controller.is_limited()
    }

    pub fn configured_max_or_unlimited(&self) -> u32 {
        self.controller.configured_max_or_unlimited()
    }

    pub fn effective_max_rounds(&self) -> u32 {
        self.controller.effective_max_rounds()
    }

    pub fn is_final_round_index(&self, index: u32) -> bool {
        self.controller.is_final_round_index(index)
    }

    /// Start the next provider round. `Ok(None)` is the deterministic round
    /// limit terminal and never starts a provider attempt.
    pub fn begin_round(&mut self) -> Result<Option<RoundTicket>, LoopError> {
        if self.phase != LoopPhase::Ready {
            return Err(self.protocol("round started outside the ready checkpoint"));
        }
        let Some(ticket) = self.controller.current_ticket() else {
            self.phase = LoopPhase::Completed;
            return Ok(None);
        };
        self.controller.mark_started(ticket);
        self.active_ticket = Some(ticket);
        self.phase = LoopPhase::Streaming;
        Ok(Some(ticket))
    }

    /// Retry the same provider round without consuming another round.
    pub fn retry_provider(&mut self, ticket: RoundTicket) -> Result<(), LoopError> {
        self.require_active(ticket, LoopPhase::Streaming)?;
        self.phase = LoopPhase::Ready;
        self.active_ticket = None;
        Ok(())
    }

    /// Acknowledge that provider-native assistant/tool-call history has been
    /// committed. Tool calls on a final tools-disabled round are a protocol
    /// failure and are never authorized for execution.
    pub fn commit_model_round(
        &mut self,
        ticket: RoundTicket,
        has_tool_calls: bool,
    ) -> Result<ModelRoundDecision, LoopError> {
        self.require_active(ticket, LoopPhase::Streaming)?;
        if !has_tool_calls {
            self.controller.mark_natural_exit();
            self.phase = LoopPhase::Completed;
            self.active_ticket = None;
            return Ok(ModelRoundDecision::Complete);
        }
        if ticket.is_final() {
            self.phase = LoopPhase::Failed;
            return Err(LoopError::new(
                LoopErrorKind::Protocol,
                LoopPhase::Streaming,
                "provider requested tools on the final tools-disabled round",
            ));
        }
        self.phase = LoopPhase::ModelCommitted;
        Ok(ModelRoundDecision::ExecuteTools)
    }

    pub fn begin_tool_batch(&mut self, ticket: RoundTicket) -> Result<(), LoopError> {
        self.require_active(ticket, LoopPhase::ModelCommitted)?;
        self.phase = LoopPhase::ToolRunning;
        Ok(())
    }

    /// Acknowledge that the complete, input-ordered tool-result history has
    /// been committed.
    pub fn commit_tool_batch(&mut self, ticket: RoundTicket) -> Result<(), LoopError> {
        self.require_active(ticket, LoopPhase::ToolRunning)?;
        self.phase = LoopPhase::ToolCommitted;
        Ok(())
    }

    /// Steering/follow-up input is absorbed only after the tool checkpoint.
    pub fn commit_steering(
        &mut self,
        ticket: RoundTicket,
        inserted_count: usize,
    ) -> Result<(), LoopError> {
        self.require_active(ticket, LoopPhase::ToolCommitted)?;
        self.controller
            .ensure_followup_after_insertion(ticket, inserted_count);
        self.controller.advance(ticket);
        self.phase = LoopPhase::Ready;
        self.active_ticket = None;
        Ok(())
    }

    /// End after a committed tool batch without admitting another provider
    /// round (for example, an outer observation hook requested a clean stop).
    pub fn complete_after_tool_batch(&mut self, ticket: RoundTicket) -> Result<(), LoopError> {
        self.require_active(ticket, LoopPhase::ToolCommitted)?;
        self.phase = LoopPhase::Completed;
        self.active_ticket = None;
        Ok(())
    }

    pub fn grant_activation_grace(&mut self) -> bool {
        self.controller.grant_activation_grace()
    }

    /// Cancellation is observable at every phase and wins over the round
    /// limit classification.
    pub fn cancel(&mut self) -> LoopTerminal {
        self.phase = LoopPhase::Cancelled;
        self.active_ticket = None;
        self.controller.finish(true)
    }

    pub fn fail(&mut self, kind: LoopErrorKind, message: &'static str) -> LoopError {
        let phase = self.phase;
        self.phase = LoopPhase::Failed;
        LoopError::new(kind, phase, message)
    }

    pub fn finish(&self, cancelled: bool) -> LoopTerminal {
        self.controller.finish(cancelled)
    }

    fn require_active(&self, ticket: RoundTicket, expected: LoopPhase) -> Result<(), LoopError> {
        if self.phase != expected || self.active_ticket != Some(ticket) {
            return Err(self.protocol("loop checkpoint transition is out of order"));
        }
        Ok(())
    }

    fn protocol(&self, message: &'static str) -> LoopError {
        LoopError::new(LoopErrorKind::Protocol, self.phase, message)
    }
}

/// Run futures concurrently with a bounded in-flight count and return results
/// in input order.
///
/// The product runtime decides which operations are safe to put in this batch;
/// this mechanism never infers safety from tool names. `max = 0` is clamped to
/// one so a bad caller cannot deadlock on a zero-permit semaphore.
pub async fn run_bounded_in_order<T, Fut>(max: usize, futures: Vec<Fut>) -> Vec<T>
where
    Fut: Future<Output = T>,
{
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max.max(1)));
    let bounded = futures.into_iter().map(|future| {
        let semaphore = semaphore.clone();
        async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("agent-loop semaphore is never closed");
            future.await
        }
    });
    join_all(bounded).await
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct ScriptRound {
        native: &'static str,
        calls: Vec<usize>,
        terminal: bool,
    }

    struct ScriptedDriver {
        rounds: VecDeque<ScriptRound>,
        steering: VecDeque<usize>,
        trace: Vec<String>,
        fail_model_checkpoint: bool,
        cancel_during_tools: Option<CancellationToken>,
    }

    impl ScriptedDriver {
        fn new(rounds: Vec<ScriptRound>) -> Self {
            Self {
                rounds: rounds.into(),
                steering: VecDeque::new(),
                trace: Vec::new(),
                fail_model_checkpoint: false,
                cancel_during_tools: None,
            }
        }
    }

    #[async_trait]
    impl RoundDriver for ScriptedDriver {
        type PreparedRound = ();
        type NativeRound = &'static str;
        type ToolCall = usize;

        async fn prepare_round(
            &mut self,
            ticket: RoundTicket,
        ) -> Result<Self::PreparedRound, LoopError> {
            self.trace.push(format!("prepare:{}", ticket.index()));
            Ok(())
        }

        async fn stream_round(
            &mut self,
            ticket: RoundTicket,
            _prepared: Self::PreparedRound,
        ) -> Result<ModelRound<Self::NativeRound, Self::ToolCall>, LoopError> {
            self.trace.push(format!("stream:{}", ticket.index()));
            let round = self.rounds.pop_front().ok_or_else(|| {
                LoopError::new(
                    LoopErrorKind::Provider,
                    LoopPhase::Streaming,
                    "script exhausted",
                )
            })?;
            Ok(ModelRound {
                native: round.native,
                tool_calls: round.calls,
                terminal: round.terminal,
            })
        }
    }

    #[async_trait]
    impl ToolBatchDriver<usize> for ScriptedDriver {
        type ToolResult = usize;

        async fn execute_tool_batch(
            &mut self,
            ticket: RoundTicket,
            calls: Vec<usize>,
            _cancellation: &CancellationToken,
        ) -> Result<ExecutedToolBatch<Self::ToolResult>, LoopError> {
            self.trace
                .push(format!("tools:{}:{calls:?}", ticket.index()));
            if let Some(token) = self.cancel_during_tools.as_ref() {
                token.cancel();
            }
            Ok(ExecutedToolBatch {
                results: calls.into_iter().map(|value| value * 10).collect(),
            })
        }
    }

    #[async_trait]
    impl LoopCheckpoint<&'static str, usize> for ScriptedDriver {
        async fn commit_model_round(
            &mut self,
            ticket: RoundTicket,
            native: &&'static str,
        ) -> Result<(), LoopError> {
            self.trace
                .push(format!("model_commit:{}:{native}", ticket.index()));
            if self.fail_model_checkpoint {
                return Err(LoopError::new(
                    LoopErrorKind::Checkpoint,
                    LoopPhase::Streaming,
                    "scripted model checkpoint failure",
                ));
            }
            Ok(())
        }

        async fn commit_tool_batch(
            &mut self,
            ticket: RoundTicket,
            results: &[usize],
        ) -> Result<(), LoopError> {
            self.trace
                .push(format!("tool_commit:{}:{results:?}", ticket.index()));
            Ok(())
        }
    }

    #[async_trait]
    impl SteeringDriver for ScriptedDriver {
        async fn commit_steering(&mut self, ticket: RoundTicket) -> Result<usize, LoopError> {
            self.trace.push(format!("steering:{}", ticket.index()));
            Ok(self.steering.pop_front().unwrap_or(0))
        }
    }

    #[test]
    fn zero_means_unlimited_without_a_synthetic_final_round() {
        let controller = LoopController::from_configured_max(0);
        assert!(!controller.is_limited());
        assert_eq!(controller.configured_max_or_unlimited(), u32::MAX);
        assert!(!controller.current_ticket().unwrap().is_final());
        assert!(!controller.finish(false).hit_round_limit);
    }

    #[test]
    fn provider_retry_reuses_the_same_round() {
        let mut controller = LoopController::from_configured_max(3);
        let first = controller.current_ticket().unwrap();
        controller.mark_started(first);
        let retry = controller.current_ticket().unwrap();
        controller.mark_started(retry);

        assert_eq!(first, retry);
        assert_eq!(controller.finish(false).rounds_started, 1);

        controller.advance(retry);
        assert_eq!(controller.current_ticket().unwrap().index(), 1);
    }

    #[test]
    fn activation_grace_is_bounded_to_one_extension() {
        let mut controller = LoopController::from_configured_max(2);
        assert!(controller.grant_activation_grace());
        assert_eq!(controller.effective_max_rounds(), 3);
        assert!(!controller.grant_activation_grace());
        assert_eq!(controller.effective_max_rounds(), 3);
    }

    #[test]
    fn insertion_at_last_boundary_gets_a_followup_round() {
        let mut controller = LoopController::from_configured_max(4);
        for expected in 0..4 {
            let ticket = controller.current_ticket().unwrap();
            assert_eq!(ticket.index(), expected);
            controller.mark_started(ticket);
            if expected == 3 {
                assert!(controller.ensure_followup_after_insertion(ticket, 1));
            }
            controller.advance(ticket);
        }
        let followup = controller.current_ticket().expect("extension must exist");
        assert_eq!(followup.index(), 4);
        assert!(followup.is_final());
    }

    #[test]
    fn insertion_before_last_boundary_does_not_extend() {
        let mut controller = LoopController::from_configured_max(4);
        let first = controller.current_ticket().unwrap();
        controller.mark_started(first);
        assert!(!controller.ensure_followup_after_insertion(first, 1));
        assert_eq!(controller.effective_max_rounds(), 4);
    }

    #[test]
    fn natural_final_exit_is_not_reported_as_exhausted() {
        let mut controller = LoopController::from_configured_max(1);
        let ticket = controller.current_ticket().unwrap();
        assert!(ticket.is_final());
        controller.mark_started(ticket);
        controller.mark_natural_exit();

        let terminal = controller.finish(false);
        assert!(terminal.hit_round_limit);
        assert!(!terminal.rounds_exhausted);
    }

    #[test]
    fn cancellation_wins_over_round_limit() {
        let mut controller = LoopController::from_configured_max(1);
        let ticket = controller.current_ticket().unwrap();
        controller.mark_started(ticket);

        let terminal = controller.finish(true);
        assert!(terminal.cancelled);
        assert!(!terminal.hit_round_limit);
        assert!(!terminal.rounds_exhausted);
    }

    #[tokio::test]
    async fn bounded_execution_caps_concurrency_and_preserves_order() {
        let inflight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let futures = (0..20usize)
            .map(|index| {
                let inflight = inflight.clone();
                let peak = peak.clone();
                async move {
                    let current = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(current, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
                    inflight.fetch_sub(1, Ordering::SeqCst);
                    index
                }
            })
            .collect();

        let result = run_bounded_in_order(8, futures).await;
        assert_eq!(result, (0..20).collect::<Vec<_>>());
        assert!(peak.load(Ordering::SeqCst) <= 8);
        assert!(peak.load(Ordering::SeqCst) > 1);
    }

    #[tokio::test]
    async fn zero_concurrency_clamps_to_single_flight() {
        let futures = vec![std::future::ready(1), std::future::ready(2)];
        assert_eq!(run_bounded_in_order(0, futures).await, vec![1, 2]);
    }

    #[tokio::test]
    async fn port_driver_commits_each_boundary_in_order() {
        let mut driver = ScriptedDriver::new(vec![
            ScriptRound {
                native: "assistant+calls",
                calls: vec![2, 1],
                terminal: false,
            },
            ScriptRound {
                native: "assistant-final",
                calls: Vec::new(),
                terminal: true,
            },
        ]);
        let outcome = drive_loop(3, &CancellationToken::default(), &mut driver)
            .await
            .unwrap();
        assert_eq!(outcome.phase, LoopPhase::Completed);
        assert_eq!(
            driver.trace,
            [
                "prepare:0",
                "stream:0",
                "model_commit:0:assistant+calls",
                "tools:0:[2, 1]",
                "tool_commit:0:[20, 10]",
                "steering:0",
                "prepare:1",
                "stream:1",
                "model_commit:1:assistant-final",
            ]
        );
    }

    #[tokio::test]
    async fn model_checkpoint_failure_never_starts_tools() {
        let mut driver = ScriptedDriver::new(vec![ScriptRound {
            native: "assistant+call",
            calls: vec![1],
            terminal: false,
        }]);
        driver.fail_model_checkpoint = true;
        let error = drive_loop(2, &CancellationToken::default(), &mut driver)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), LoopErrorKind::Checkpoint);
        assert!(!driver.trace.iter().any(|event| event.starts_with("tools:")));
    }

    #[tokio::test]
    async fn cancellation_during_tools_commits_complete_batch_then_stops() {
        let cancellation = CancellationToken::default();
        let mut driver = ScriptedDriver::new(vec![ScriptRound {
            native: "assistant+calls",
            calls: vec![3, 1],
            terminal: false,
        }]);
        driver.cancel_during_tools = Some(cancellation.clone());
        let outcome = drive_loop(3, &cancellation, &mut driver).await.unwrap();
        assert_eq!(outcome.phase, LoopPhase::Cancelled);
        assert_eq!(
            driver.trace.last().map(String::as_str),
            Some("tool_commit:0:[30, 10]")
        );
    }

    #[tokio::test]
    async fn final_round_tool_call_commits_model_but_never_executes_tool() {
        let mut driver = ScriptedDriver::new(vec![ScriptRound {
            native: "malformed-final-call",
            calls: vec![7],
            terminal: false,
        }]);
        let error = drive_loop(1, &CancellationToken::default(), &mut driver)
            .await
            .unwrap_err();
        assert_eq!(error.kind(), LoopErrorKind::Protocol);
        assert_eq!(
            driver.trace.last().map(String::as_str),
            Some("model_commit:0:malformed-final-call")
        );
    }

    #[test]
    fn model_checkpoint_precedes_tool_execution_and_steering() {
        let mut machine = LoopStateMachine::from_configured_max(3);
        let ticket = machine.begin_round().unwrap().unwrap();
        assert_eq!(machine.phase(), LoopPhase::Streaming);
        assert_eq!(
            machine.commit_model_round(ticket, true).unwrap(),
            ModelRoundDecision::ExecuteTools
        );
        assert_eq!(machine.phase(), LoopPhase::ModelCommitted);
        machine.begin_tool_batch(ticket).unwrap();
        assert_eq!(machine.phase(), LoopPhase::ToolRunning);
        machine.commit_tool_batch(ticket).unwrap();
        assert_eq!(machine.phase(), LoopPhase::ToolCommitted);
        machine.commit_steering(ticket, 0).unwrap();
        assert_eq!(machine.phase(), LoopPhase::Ready);
        assert_eq!(machine.begin_round().unwrap().unwrap().index(), 1);
    }

    #[test]
    fn transition_order_is_fail_closed() {
        let mut machine = LoopStateMachine::from_configured_max(3);
        let ticket = machine.begin_round().unwrap().unwrap();
        let error = machine.begin_tool_batch(ticket).unwrap_err();
        assert_eq!(error.kind(), LoopErrorKind::Protocol);
        assert_eq!(error.phase(), LoopPhase::Streaming);
    }

    #[test]
    fn final_round_tool_call_is_protocol_error() {
        let mut machine = LoopStateMachine::from_configured_max(1);
        let ticket = machine.begin_round().unwrap().unwrap();
        assert!(ticket.is_final());
        let error = machine.commit_model_round(ticket, true).unwrap_err();
        assert_eq!(error.kind(), LoopErrorKind::Protocol);
        assert_eq!(machine.phase(), LoopPhase::Failed);
        assert_eq!(machine.finish(false).rounds_started, 1);
    }

    #[test]
    fn provider_retry_preserves_ticket_and_phase() {
        let mut machine = LoopStateMachine::from_configured_max(2);
        let first = machine.begin_round().unwrap().unwrap();
        machine.retry_provider(first).unwrap();
        let retry = machine.begin_round().unwrap().unwrap();
        assert_eq!(first, retry);
        assert_eq!(machine.finish(false).rounds_started, 1);
    }

    #[test]
    fn steering_cannot_land_before_tool_checkpoint() {
        let mut machine = LoopStateMachine::from_configured_max(2);
        let ticket = machine.begin_round().unwrap().unwrap();
        machine.commit_model_round(ticket, true).unwrap();
        let error = machine.commit_steering(ticket, 1).unwrap_err();
        assert_eq!(error.kind(), LoopErrorKind::Protocol);
    }

    #[test]
    fn checkpoint_failure_is_typed_and_terminal() {
        let mut machine = LoopStateMachine::from_configured_max(2);
        let _ticket = machine.begin_round().unwrap().unwrap();
        let error = machine.fail(LoopErrorKind::Checkpoint, "model checkpoint failed");
        assert_eq!(error.kind(), LoopErrorKind::Checkpoint);
        assert_eq!(error.phase(), LoopPhase::Streaming);
        assert_eq!(machine.phase(), LoopPhase::Failed);
    }

    #[test]
    fn cancellation_is_observable_at_each_stable_stage() {
        let phases = [
            LoopPhase::Ready,
            LoopPhase::Streaming,
            LoopPhase::ModelCommitted,
            LoopPhase::ToolRunning,
            LoopPhase::ToolCommitted,
        ];
        for target in phases {
            let mut machine = LoopStateMachine::from_configured_max(3);
            if target != LoopPhase::Ready {
                let ticket = machine.begin_round().unwrap().unwrap();
                if matches!(
                    target,
                    LoopPhase::ModelCommitted | LoopPhase::ToolRunning | LoopPhase::ToolCommitted
                ) {
                    machine.commit_model_round(ticket, true).unwrap();
                }
                if matches!(target, LoopPhase::ToolRunning | LoopPhase::ToolCommitted) {
                    machine.begin_tool_batch(ticket).unwrap();
                }
                if target == LoopPhase::ToolCommitted {
                    machine.commit_tool_batch(ticket).unwrap();
                }
            }
            assert_eq!(machine.phase(), target);
            let terminal = machine.cancel();
            assert!(terminal.cancelled);
            assert_eq!(machine.phase(), LoopPhase::Cancelled);
        }
    }

    /// The loop stores no provider payload. Opaque/native items therefore
    /// round-trip byte-for-byte for every supported adapter family.
    #[test]
    fn four_provider_native_history_fixtures_round_trip_opaque() {
        let fixtures = [
            (
                "anthropic",
                r#"[{\"type\":\"thinking\"},{\"type\":\"tool_use\"}]"#,
            ),
            (
                "openai_chat",
                r#"[{\"role\":\"assistant\",\"tool_calls\":[]}]"#,
            ),
            (
                "openai_responses",
                r#"[{\"type\":\"reasoning\"},{\"type\":\"function_call\"}]"#,
            ),
            (
                "codex",
                r#"[{\"type\":\"message\"},{\"type\":\"function_call\"}]"#,
            ),
        ];
        for (provider, native) in fixtures {
            let owned = native.to_string();
            let mut machine = LoopStateMachine::from_configured_max(2);
            let ticket = machine.begin_round().unwrap().unwrap();
            machine.commit_model_round(ticket, true).unwrap();
            machine.begin_tool_batch(ticket).unwrap();
            machine.commit_tool_batch(ticket).unwrap();
            machine.commit_steering(ticket, 0).unwrap();
            assert_eq!(owned, native, "{provider} history changed");
        }
    }
}
