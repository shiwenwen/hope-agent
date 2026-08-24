//! Authoritative entry for durable, multi-round conversation turns.
//!
//! Callers construct a source-neutral [`TurnRequest`]. A source-specific
//! [`TurnSubmission`] seals the source and kernel-owned execution policy before
//! the kernel-private [`ChatEngineParams`] reaches the engine.
//!
//! This stays kernel-owned: admission, Stop epochs and the durable turn ledger
//! must remain one globally authoritative boundary.

use crate::chat_engine::{ChatEngineParams, ChatSource};
use futures_util::FutureExt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, OnceLock};

pub use crate::chat_engine::stream_seq::{source_policy, TurnSourcePolicy};
pub use crate::chat_engine::{TurnFailure, TurnFailureKind};

/// Source-neutral output of the required Agent runtime port. Concrete Agent
/// instances and provider credentials never cross this boundary.
pub struct AgentTurnOutput {
    pub response: String,
    pub model_used: Option<crate::provider::ActiveModel>,
    pub usage: crate::chat_engine::CapturedUsage,
    pub terminal: crate::chat_engine::TurnTerminal,
}

/// Opaque, process-local proof that a foreground Desktop/HTTP turn won the
/// active-turn admission gate captured at transport entry. The guard cannot be
/// constructed or inspected outside the kernel and is held through terminal
/// convergence.
pub struct InteractiveTurnLease {
    session_id: String,
    turn_id: String,
    source: ChatSource,
    stop_admission: Option<crate::session::ForegroundStopAdmission>,
    guard: crate::chat_engine::active_turn::ActiveTurnGuard,
}

/// User-message persistence input paired with an InteractiveTurnLease.
/// Security policy and source are not fields: they come from the lease and
/// source-specific submission constructor.
pub struct InteractiveAdmission {
    lease: InteractiveTurnLease,
    user_message: crate::session::NewMessage,
    edit_message_id: Option<i64>,
    dispatch_identity: Option<(String, String)>,
    bootstrap_request_id: Option<String>,
    delete_new_session_on_cancel: bool,
    emit_session_created: bool,
}

impl InteractiveAdmission {
    #[allow(clippy::too_many_arguments)]
    pub fn desktop(
        lease: InteractiveTurnLease,
        user_message: crate::session::NewMessage,
        edit_message_id: Option<i64>,
        dispatch_identity: Option<(String, String)>,
        bootstrap_request_id: Option<String>,
        delete_new_session_on_cancel: bool,
        emit_session_created: bool,
    ) -> Self {
        Self {
            lease,
            user_message,
            edit_message_id,
            dispatch_identity,
            bootstrap_request_id,
            delete_new_session_on_cancel,
            emit_session_created,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn http(
        lease: InteractiveTurnLease,
        user_message: crate::session::NewMessage,
        edit_message_id: Option<i64>,
        dispatch_identity: Option<(String, String)>,
        bootstrap_request_id: Option<String>,
        delete_new_session_on_cancel: bool,
    ) -> Self {
        Self {
            lease,
            user_message,
            edit_message_id,
            dispatch_identity,
            bootstrap_request_id,
            delete_new_session_on_cancel,
            emit_session_created: false,
        }
    }
}

/// Compatibility request containing turn data and runtime snapshots, but no
/// caller-selected source or terminal policy.
///
/// This is the P7 replacement for the public `ChatEngineParams` bag. Follow-up
/// admission work can split the remaining trusted lineage/runtime fields into
/// narrower proof types without exposing the engine's control booleans again.
pub struct TurnRequest {
    session_id: String,
    agent_id: String,
    turn_id: Option<String>,
    message: String,
    incoming_turn: Option<crate::prompt_context::IncomingTurnWire>,
    display_text: Option<String>,
    attachments: Vec<crate::agent::Attachment>,
    session_db: Arc<crate::session::SessionDB>,
    model_selection: ModelSelection,
    resolved_temperature: Option<f64>,
    compact_config: crate::context_compact::CompactConfig,
    run_context: Option<crate::prompt_context::RunInstructionContext>,
    reasoning_effort: Option<String>,
    cancel: Arc<AtomicBool>,
    foreground_stop_admission: Option<crate::session::ForegroundStopAdmission>,
    plan_context_override: Option<crate::agent::PlanResolvedContext>,
    skill_allowed_tools: Vec<String>,
    denied_tools: Vec<String>,
    tool_scope: Option<crate::tool_defs::ToolScope>,
    subagent_depth: u32,
    steer_run_id: Option<String>,
    ui_surface: Option<crate::pet::ChatUiSurface>,
    event_sink: Arc<dyn crate::chat_engine::EventSink>,
}

impl TurnRequest {
    /// Construct the minimal executable payload. Optional capabilities start
    /// fail-closed/empty and are enabled through named methods; external
    /// producers cannot populate private engine fields with a struct literal.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: String,
        agent_id: String,
        message: String,
        session_db: Arc<crate::session::SessionDB>,
        compact_config: crate::context_compact::CompactConfig,
        cancel: Arc<AtomicBool>,
        event_sink: Arc<dyn crate::chat_engine::EventSink>,
    ) -> Self {
        Self {
            session_id,
            agent_id,
            turn_id: None,
            message,
            incoming_turn: None,
            display_text: None,
            attachments: Vec::new(),
            session_db,
            model_selection: ModelSelection::Configured {
                preferred_model: None,
                strict_preference: false,
            },
            resolved_temperature: None,
            compact_config,
            run_context: None,
            reasoning_effort: None,
            cancel,
            foreground_stop_admission: None,
            plan_context_override: None,
            skill_allowed_tools: Vec::new(),
            denied_tools: Vec::new(),
            tool_scope: None,
            subagent_depth: 0,
            steer_run_id: None,
            ui_surface: None,
            event_sink,
        }
    }

    /// Supply a routing preference, not a caller-resolved model chain.
    ///
    /// Persisted Session/Plan preferences should use `strict = false` so an
    /// unavailable stale preference can fall through to Agent/global policy.
    /// Explicit per-turn overrides use `strict = true` and fail closed when
    /// the selected Provider/model is unavailable in the admitted snapshot.
    pub fn with_model_preference(mut self, preferred_model: Option<String>, strict: bool) -> Self {
        self.model_selection = ModelSelection::Configured {
            preferred_model,
            strict_preference: strict,
        };
        self
    }

    /// Attach an isolated evaluation model chain. Source sealing rejects this
    /// capability for every non-Eval source, so production transports cannot
    /// bypass the kernel's model-routing policy.
    pub fn with_evaluation_model_chain(
        mut self,
        model_chain: Vec<crate::provider::ActiveModel>,
    ) -> Self {
        self.model_selection = ModelSelection::Evaluation(model_chain);
        self
    }

    /// Bind the visible `chat_turns` identity owned by Desktop/HTTP
    /// interactive admission. Sources without a `chat_turns` row are rejected
    /// if they attach this capability.
    pub fn with_turn_id(mut self, turn_id: String) -> Self {
        self.turn_id = Some(turn_id);
        self
    }

    pub fn with_incoming_turn(
        mut self,
        incoming_turn: Option<crate::prompt_context::IncomingTurnWire>,
    ) -> Self {
        self.incoming_turn = incoming_turn;
        self
    }

    pub fn with_display_text(mut self, display_text: Option<String>) -> Self {
        self.display_text = display_text;
        self
    }

    pub fn with_attachments(mut self, attachments: Vec<crate::agent::Attachment>) -> Self {
        self.attachments = attachments;
        self
    }

    pub fn with_temperature(mut self, temperature: Option<f64>) -> Self {
        self.resolved_temperature = temperature;
        self
    }

    pub fn with_run_context(
        mut self,
        run_context: Option<crate::prompt_context::RunInstructionContext>,
    ) -> Self {
        self.run_context = run_context;
        self
    }

    pub fn with_reasoning_effort(mut self, reasoning_effort: Option<String>) -> Self {
        self.reasoning_effort = reasoning_effort;
        self
    }

    #[doc(hidden)]
    pub fn with_foreground_stop_admission(
        mut self,
        admission: Option<crate::session::ForegroundStopAdmission>,
    ) -> Self {
        self.foreground_stop_admission = admission;
        self
    }

    pub fn with_plan_context_override(
        mut self,
        plan_context_override: Option<crate::agent::PlanResolvedContext>,
    ) -> Self {
        self.plan_context_override = plan_context_override;
        self
    }

    pub fn with_skill_allowed_tools(mut self, allowed: Vec<String>) -> Self {
        self.skill_allowed_tools = allowed;
        self
    }

    pub fn with_denied_tools(mut self, denied: Vec<String>) -> Self {
        self.denied_tools = denied;
        self
    }

    pub fn with_tool_scope(mut self, scope: Option<crate::tool_defs::ToolScope>) -> Self {
        self.tool_scope = scope;
        self
    }

    pub fn with_subagent_depth(mut self, depth: u32) -> Self {
        self.subagent_depth = depth;
        self
    }

    pub fn with_steer_run_id(mut self, run_id: Option<String>) -> Self {
        self.steer_run_id = run_id;
        self
    }

    pub fn with_ui_surface(mut self, surface: Option<crate::pet::ChatUiSurface>) -> Self {
        self.ui_surface = surface;
        self
    }

    fn into_engine_params(
        self,
        source: ChatSource,
        auto_approve_tools: bool,
        post_turn_effects: bool,
        abort_on_cancel: bool,
        persist_final_error_event: bool,
        origin_source: Option<crate::knowledge::KbAccessSource>,
        channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
        providers: Vec<crate::provider::ProviderConfig>,
        config_revision: [u8; 32],
    ) -> (ChatEngineParams, ModelSelection) {
        // Product routing metadata is admitted at the TurnKernel boundary and
        // intentionally does not enter model/runtime parameters yet.
        let model_selection = self.model_selection;
        let model_chain = match &model_selection {
            ModelSelection::Configured { .. } => Vec::new(),
            ModelSelection::Evaluation(chain) => chain.clone(),
        };
        (
            ChatEngineParams {
                session_id: self.session_id,
                agent_id: self.agent_id,
                turn_id: self.turn_id,
                pre_admitted_stream: None,
                active_turn_guard: None,
                ui_surface: self.ui_surface,
                message: self.message,
                incoming_turn: self.incoming_turn,
                display_text: self.display_text,
                attachments: self.attachments,
                session_db: self.session_db,
                model_chain,
                providers,
                config_revision,
                // Credentials never cross the public turn boundary. The engine's
                // scoped provider setup hydrates Codex OAuth only when needed.
                codex_token: None,
                resolved_temperature: self.resolved_temperature,
                compact_config: self.compact_config,
                run_context: self.run_context,
                reasoning_effort: self.reasoning_effort,
                cancel: self.cancel,
                foreground_stop_admission: self.foreground_stop_admission,
                plan_context_override: self.plan_context_override,
                skill_allowed_tools: self.skill_allowed_tools,
                denied_tools: self.denied_tools,
                tool_scope: self.tool_scope,
                subagent_depth: self.subagent_depth,
                steer_run_id: self.steer_run_id,
                auto_approve_tools,
                follow_global_reasoning_effort: false,
                post_turn_effects,
                abort_on_cancel,
                persist_final_error_event,
                source,
                origin_source,
                channel_kb_context,
                event_sink: self.event_sink,
            },
            model_selection,
        )
    }
}

enum ModelSelection {
    Configured {
        preferred_model: Option<String>,
        strict_preference: bool,
    },
    Evaluation(Vec<crate::provider::ActiveModel>),
}

/// Sealed source-specific turn request.
///
/// The inner compatibility payload is deliberately private: producers choose
/// a semantic constructor rather than supplying a free-form [`ChatSource`]
/// and a mutually inconsistent set of lifecycle booleans.
pub struct TurnSubmission {
    params: ChatEngineParams,
    model_selection: Option<ModelSelection>,
    routing_config: Option<Arc<crate::config::AppConfig>>,
    interactive: Option<InteractiveAdmission>,
    source_proof: Option<SourceProof>,
}

struct ProviderCredentialLease {
    providers: Vec<crate::provider::ProviderConfig>,
    routing_config: Option<Arc<crate::config::AppConfig>>,
    revision: [u8; 32],
}

impl ProviderCredentialLease {
    fn from_current_config() -> Self {
        let snapshot = crate::config::cached_config();
        let revision = provider_revision(&snapshot.providers);
        Self {
            providers: snapshot.providers.clone(),
            routing_config: Some(snapshot),
            revision,
        }
    }

    fn from_providers(providers: Vec<crate::provider::ProviderConfig>) -> Self {
        // Hashing the immutable snapshot makes the revision useful without
        // publishing a counter or secret-bearing config handle. The digest is
        // process-local admission metadata and is never emitted/logged.
        let revision = provider_revision(&providers);
        Self {
            providers,
            routing_config: None,
            revision,
        }
    }
}

fn provider_revision(providers: &[crate::provider::ProviderConfig]) -> [u8; 32] {
    serde_json::to_vec(providers)
        .map(|bytes| *blake3::hash(&bytes).as_bytes())
        .unwrap_or([0; 32])
}

enum SourceProof {
    Channel(crate::knowledge::ChannelKbContext),
}

/// Kernel-admitted execution capability. Only TurnKernel::admit can construct
/// it; queue/replay runners cannot rebuild trusted fields from a free-form bag.
pub struct AdmittedTurn {
    params: Option<ChatEngineParams>,
    drop_recovery: Option<AdmittedDropRecovery>,
}

struct AdmittedDropRecovery {
    db: Arc<crate::session::SessionDB>,
    session_id: String,
    turn_id: Option<String>,
    source: ChatSource,
    run_id: String,
    armed: bool,
}

impl AdmittedTurn {
    fn params(&self) -> &ChatEngineParams {
        self.params
            .as_ref()
            .expect("admitted turn parameters were already consumed")
    }

    #[doc(hidden)]
    pub fn into_runtime_params(mut self) -> ChatEngineParams {
        self.drop_recovery = None;
        self.params
            .take()
            .expect("admitted turn parameters were already consumed")
    }
}

impl Drop for AdmittedDropRecovery {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        crate::chat_engine::spawn_abandoned_stream_recovery(
            self.db.clone(),
            self.session_id.clone(),
            self.turn_id.clone(),
            self.source,
            self.run_id.clone(),
        );
    }
}

/// Required execution port implemented by `ha-agent-runtime`.
pub type AgentTurnFuture =
    Pin<Box<dyn Future<Output = Result<AgentTurnOutput, TurnFailure>> + Send + 'static>>;

#[derive(Clone, Copy)]
pub struct AgentTurnExecutor {
    pub execute: fn(AdmittedTurn) -> AgentTurnFuture,
}

static EXECUTOR: OnceLock<AgentTurnExecutor> = OnceLock::new();

pub fn register_agent_turn_executor(
    executor: AgentTurnExecutor,
) -> Result<(), crate::AlreadyRegistered> {
    EXECUTOR
        .set(executor)
        .map_err(|_| crate::AlreadyRegistered("agent turn executor"))
}

pub fn agent_turn_executor_is_registered() -> bool {
    EXECUTOR.get().is_some()
}

async fn execute(turn: AdmittedTurn) -> Result<AgentTurnOutput, TurnFailure> {
    let Some(executor) = EXECUTOR.get() else {
        #[cfg(test)]
        {
            return crate::chat_engine::engine::execute_admitted_turn_for_test(turn).await;
        }
        #[cfg(not(test))]
        return Err(TurnFailure::new(
            TurnFailureKind::Infrastructure,
            "agent turn runtime is not registered",
        ));
    };
    let future =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (executor.execute)(turn)))
            .map_err(|_| {
                TurnFailure::new(
                    TurnFailureKind::Panicked,
                    "agent turn runtime panicked before returning its execution future",
                )
            })?;
    std::panic::AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .map_err(|_| {
            TurnFailure::new(
                TurnFailureKind::Panicked,
                "agent turn runtime panicked during execution",
            )
        })?
}

impl TurnSubmission {
    fn seal(
        request: TurnRequest,
        source: ChatSource,
        auto_approve_tools: bool,
        post_turn_effects: bool,
        abort_on_cancel: bool,
        persist_final_error_event: bool,
        origin_source: Option<crate::knowledge::KbAccessSource>,
        channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
        provider_lease: ProviderCredentialLease,
    ) -> Self {
        let (params, model_selection) = request.into_engine_params(
            source,
            auto_approve_tools,
            post_turn_effects,
            abort_on_cancel,
            persist_final_error_event,
            origin_source,
            channel_kb_context,
            provider_lease.providers,
            provider_lease.revision,
        );
        Self {
            params,
            model_selection: Some(model_selection),
            routing_config: provider_lease.routing_config,
            interactive: None,
            source_proof: None,
        }
    }

    /// First-party desktop conversation.
    pub fn desktop(request: TurnRequest, admission: InteractiveAdmission) -> Self {
        let mut submission = Self::seal(
            request,
            ChatSource::Desktop,
            false,
            true,
            false,
            true,
            None,
            None,
            ProviderCredentialLease::from_current_config(),
        );
        submission.interactive = Some(admission);
        submission
    }

    /// Owner-authenticated HTTP conversation. The server's explicit
    /// deployment auto-approval switch remains authoritative.
    pub fn http(
        request: TurnRequest,
        admission: InteractiveAdmission,
        auto_approve_tools: bool,
    ) -> Self {
        let mut submission = Self::seal(
            request,
            ChatSource::Http,
            auto_approve_tools,
            true,
            false,
            true,
            None,
            None,
            ProviderCredentialLease::from_current_config(),
        );
        submission.interactive = Some(admission);
        submission
    }

    /// IM-channel conversation. Account-scoped auto approval is preserved.
    pub fn channel(
        request: TurnRequest,
        channel_kb_context: crate::knowledge::ChannelKbContext,
    ) -> Self {
        let mut submission = Self::seal(
            request,
            ChatSource::Channel,
            false,
            true,
            false,
            true,
            None,
            None,
            ProviderCredentialLease::from_current_config(),
        );
        submission.source_proof = Some(SourceProof::Channel(channel_kb_context));
        submission
    }

    /// Scheduled owner task.
    pub fn cron(request: TurnRequest) -> Self {
        Self::seal(
            request,
            ChatSource::Cron,
            false,
            true,
            false,
            true,
            None,
            None,
            ProviderCredentialLease::from_current_config(),
        )
    }

    /// Deterministic/live evaluation run. Fixture policy may explicitly
    /// enable auto approval, but the run has no foreground-user authority,
    /// lifecycle hooks, UI broadcast, or post-turn side effects.
    pub fn evaluation(
        request: TurnRequest,
        auto_approve_tools: bool,
        providers: Vec<crate::provider::ProviderConfig>,
    ) -> Self {
        Self::seal(
            request,
            ChatSource::Eval,
            auto_approve_tools,
            false,
            false,
            true,
            None,
            None,
            ProviderCredentialLease::from_providers(providers),
        )
    }

    /// Agent Client Protocol conversation. ACP protocol translation remains
    /// in `ha-acp`; lifecycle, durability, failover and finalization are owned
    /// by the shared turn kernel.
    pub fn acp(request: TurnRequest) -> Self {
        Self::seal(
            request,
            ChatSource::Acp,
            false,
            true,
            false,
            true,
            None,
            None,
            ProviderCredentialLease::from_current_config(),
        )
    }

    /// Kernel-owned background child execution.
    pub(crate) fn subagent(
        request: TurnRequest,
        origin_source: Option<crate::knowledge::KbAccessSource>,
        channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
    ) -> Self {
        Self::seal(
            request,
            ChatSource::Subagent,
            false,
            false,
            true,
            false,
            origin_source,
            channel_kb_context,
            ProviderCredentialLease::from_current_config(),
        )
    }

    /// Kernel-owned follow-up after a background result delivery.
    pub(crate) fn parent_injection(request: TurnRequest) -> Self {
        Self::seal(
            request,
            ChatSource::ParentInjection,
            false,
            false,
            true,
            false,
            None,
            None,
            ProviderCredentialLease::from_current_config(),
        )
    }

    /// Model-initiated cross-session turn.
    pub(crate) fn session_tool(
        request: TurnRequest,
        origin_source: Option<crate::knowledge::KbAccessSource>,
        channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
    ) -> Self {
        Self::seal(
            request,
            ChatSource::SessionTool,
            false,
            true,
            false,
            true,
            origin_source,
            channel_kb_context,
            ProviderCredentialLease::from_current_config(),
        )
    }
}

/// Kernel-owned turn entry.
#[derive(Debug, Default, Clone, Copy)]
pub struct TurnKernel;

impl TurnKernel {
    fn begin_interactive(
        admission: crate::chat_engine::active_turn::ForegroundRequestAdmission,
        session_id: &str,
        source: ChatSource,
        turn_id: String,
        client_request_id: Option<String>,
        cancel: Arc<AtomicBool>,
    ) -> Result<InteractiveTurnLease, crate::chat_engine::active_turn::ActiveTurnError> {
        debug_assert!(matches!(source, ChatSource::Desktop | ChatSource::Http));
        let stop_admission = admission.durable_stop_admission();
        let guard = crate::chat_engine::active_turn::try_acquire_foreground_request(
            admission,
            session_id,
            source,
            turn_id.clone(),
            client_request_id,
            cancel,
        )?;
        Ok(InteractiveTurnLease {
            session_id: session_id.to_string(),
            turn_id,
            source,
            stop_admission,
            guard,
        })
    }

    pub fn begin_desktop(
        admission: crate::chat_engine::active_turn::ForegroundRequestAdmission,
        session_id: &str,
        turn_id: String,
        client_request_id: Option<String>,
        cancel: Arc<AtomicBool>,
    ) -> Result<InteractiveTurnLease, crate::chat_engine::active_turn::ActiveTurnError> {
        Self::begin_interactive(
            admission,
            session_id,
            ChatSource::Desktop,
            turn_id,
            client_request_id,
            cancel,
        )
    }

    pub fn begin_http(
        admission: crate::chat_engine::active_turn::ForegroundRequestAdmission,
        session_id: &str,
        turn_id: String,
        client_request_id: Option<String>,
        cancel: Arc<AtomicBool>,
    ) -> Result<InteractiveTurnLease, crate::chat_engine::active_turn::ActiveTurnError> {
        Self::begin_interactive(
            admission,
            session_id,
            ChatSource::Http,
            turn_id,
            client_request_id,
            cancel,
        )
    }

    /// Validate source proof and atomically persist the interactive message,
    /// visible turn, durability run, and Stop-generation snapshot.
    pub async fn admit(mut submission: TurnSubmission) -> Result<AdmittedTurn, TurnFailure> {
        admit_persisted_turn_identity(
            submission.params.source,
            submission.params.turn_id.as_deref(),
        )?;
        admit_model_selection(
            &mut submission.params,
            submission.model_selection.take().ok_or_else(|| {
                TurnFailure::new(
                    TurnFailureKind::Infrastructure,
                    "turn submission has no model-selection capability",
                )
            })?,
            submission.routing_config.as_deref(),
        )?;
        admit_provider_lease(&submission.params)?;
        if let Some(proof) = submission.source_proof.take() {
            admit_source_proof(&mut submission.params, proof).await?;
        }
        if let Some(admission) = submission.interactive.take() {
            admit_interactive(&mut submission.params, admission).await?;
        }
        let drop_recovery = submission
            .params
            .pre_admitted_stream
            .as_ref()
            .map(|stream| AdmittedDropRecovery {
                db: submission.params.session_db.clone(),
                session_id: submission.params.session_id.clone(),
                turn_id: submission.params.turn_id.clone(),
                source: submission.params.source,
                run_id: stream.registration.run_id.clone(),
                armed: true,
            });
        Ok(AdmittedTurn {
            params: Some(submission.params),
            drop_recovery,
        })
    }

    /// Execute a previously admitted capability. This split lets a detached
    /// HTTP UI acknowledge durable acceptance without creating a second entry.
    pub async fn run_admitted(mut turn: AdmittedTurn) -> Result<AgentTurnOutput, TurnFailure> {
        let fallback = AdmittedFailureFallback::capture(turn.params());
        let route_all_codex = admitted_route_all_codex(turn.params());
        let mut drop_recovery = turn.drop_recovery.take();
        let result = execute(turn)
            .await
            .map_err(|failure| failure.with_route_all_codex(route_all_codex));
        if let Some(recovery) = drop_recovery.as_mut() {
            recovery.armed = false;
        }
        if let Err(failure) = &result {
            fallback.converge(failure).await;
        }
        result
    }

    /// Submit a sealed turn through the authoritative kernel entry.
    pub async fn submit(submission: TurnSubmission) -> Result<AgentTurnOutput, String> {
        let admitted = Self::admit(submission)
            .await
            .map_err(|failure| failure.to_string())?;
        Self::run_admitted(admitted)
            .await
            .map_err(|failure| failure.to_string())
    }

    /// Typed-failure entry for source adapters that need terminal metadata
    /// (for example IM auth copy) or for kernel-owned retry policy.
    pub async fn submit_classified(
        submission: TurnSubmission,
    ) -> Result<AgentTurnOutput, TurnFailure> {
        let admitted = Self::admit(submission).await?;
        Self::run_admitted(admitted).await
    }
}

fn admit_persisted_turn_identity(
    source: ChatSource,
    turn_id: Option<&str>,
) -> Result<(), TurnFailure> {
    if turn_id.is_some() && !matches!(source, ChatSource::Desktop | ChatSource::Http) {
        return Err(TurnFailure::new(
            TurnFailureKind::Infrastructure,
            format!(
                "{} turn cannot attach a Desktop/HTTP chat-turn identity",
                source.as_str()
            ),
        ));
    }
    Ok(())
}

fn admitted_route_all_codex(params: &ChatEngineParams) -> bool {
    !params.model_chain.is_empty()
        && params.model_chain.iter().all(|model| {
            params
                .providers
                .iter()
                .find(|provider| provider.id == model.provider_id)
                .is_some_and(|provider| provider.api_type.is_codex())
        })
}

fn admit_model_selection(
    params: &mut ChatEngineParams,
    selection: ModelSelection,
    routing_config: Option<&crate::config::AppConfig>,
) -> Result<(), TurnFailure> {
    match selection {
        ModelSelection::Evaluation(chain) => {
            if params.source != ChatSource::Eval {
                return Err(TurnFailure::new(
                    TurnFailureKind::Infrastructure,
                    "isolated evaluation model chains are forbidden for production turns",
                ));
            }
            params.model_chain = chain;
        }
        ModelSelection::Configured {
            preferred_model,
            strict_preference,
        } => {
            if params.source == ChatSource::Eval {
                return Err(TurnFailure::new(
                    TurnFailureKind::Infrastructure,
                    "evaluation turns require an isolated model chain",
                ));
            }
            let config = routing_config.ok_or_else(|| {
                TurnFailure::new(
                    TurnFailureKind::Infrastructure,
                    "production turn has no immutable routing-config snapshot",
                )
            })?;
            params.model_chain = resolve_configured_model_chain(
                &params.agent_id,
                preferred_model.as_deref(),
                strict_preference,
                config,
            )?;
        }
    }
    if params.model_chain.is_empty() {
        return Err(TurnFailure::new(
            TurnFailureKind::Infrastructure,
            "no model configured; add an enabled provider and select an active model",
        ));
    }
    Ok(())
}

fn resolve_configured_model_chain(
    agent_id: &str,
    preferred_model: Option<&str>,
    strict_preference: bool,
    config: &crate::config::AppConfig,
) -> Result<Vec<crate::provider::ActiveModel>, TurnFailure> {
    if strict_preference {
        let Some(preferred) = preferred_model else {
            return Err(TurnFailure::invalid_request(
                "strict model preference is empty",
            ));
        };
        let available = crate::provider::parse_model_ref(preferred).is_some_and(|model| {
            crate::provider::model_ref_is_available(&config.providers, &model)
        });
        if !available {
            return Err(TurnFailure::invalid_request(
                format!(
                    "selected model override is unavailable: {preferred}; choose an enabled provider and model"
                ),
            ));
        }
    }
    let agent_model = crate::agent_loader::load_agent(agent_id)
        .map(|definition| definition.config.model)
        .unwrap_or_default();
    let (primary, fallbacks) =
        crate::provider::resolve_model_chain_with_preferred(preferred_model, &agent_model, config);
    Ok(primary.into_iter().chain(fallbacks).collect())
}

/// Read-only preflight for protocol surfaces that must reject an unusable
/// Agent before a conversation Session is created. Turn admission repeats the
/// resolution against its own immutable snapshot and remains authoritative.
#[doc(hidden)]
pub fn validate_configured_model_route(agent_id: &str) -> Result<(), TurnFailure> {
    validate_configured_model_preference(agent_id, None, false)
}

#[doc(hidden)]
pub fn validate_configured_model_preference(
    agent_id: &str,
    preferred_model: Option<&str>,
    strict_preference: bool,
) -> Result<(), TurnFailure> {
    let config = crate::config::cached_config();
    if resolve_configured_model_chain(agent_id, preferred_model, strict_preference, &config)?
        .is_empty()
    {
        return Err(TurnFailure::new(
            TurnFailureKind::Infrastructure,
            format!("no model configured for agent {agent_id}"),
        ));
    }
    Ok(())
}

fn admit_provider_lease(params: &ChatEngineParams) -> Result<(), TurnFailure> {
    if params.config_revision == [0; 32] {
        return Err(TurnFailure::new(
            TurnFailureKind::Infrastructure,
            "provider credential lease has no immutable config revision",
        ));
    }
    for model in &params.model_chain {
        let mut matches = params
            .providers
            .iter()
            .filter(|provider| provider.id == model.provider_id);
        let Some(provider) = matches.next() else {
            return Err(TurnFailure::new(
                TurnFailureKind::Infrastructure,
                format!(
                    "model chain references provider {} outside the admitted config snapshot",
                    model.provider_id
                ),
            ));
        };
        if matches.next().is_some() {
            return Err(TurnFailure::new(
                TurnFailureKind::Infrastructure,
                format!(
                    "provider {} is ambiguous in the admitted config snapshot",
                    model.provider_id
                ),
            ));
        }
        if !provider.enabled {
            return Err(TurnFailure::new(
                TurnFailureKind::Infrastructure,
                format!(
                    "provider {} was disabled in the admitted config snapshot",
                    model.provider_id
                ),
            ));
        }
    }
    Ok(())
}

struct AdmittedFailureFallback {
    db: Arc<crate::session::SessionDB>,
    session_id: String,
    turn_id: Option<String>,
    stream_id: Option<String>,
    run: Option<(String, bool)>,
}

impl AdmittedFailureFallback {
    fn capture(params: &ChatEngineParams) -> Self {
        Self {
            db: params.session_db.clone(),
            session_id: params.session_id.clone(),
            turn_id: params.turn_id.clone(),
            stream_id: params
                .pre_admitted_stream
                .as_ref()
                .map(|stream| stream.stream_id.clone()),
            run: params.pre_admitted_stream.as_ref().map(|stream| {
                (
                    stream.registration.run_id.clone(),
                    stream.registration.persistent,
                )
            }),
        }
    }

    async fn converge(self, failure: &TurnFailure) {
        let message = failure.to_string();
        let status = match failure.kind {
            TurnFailureKind::Cancelled => crate::session::ChatTurnStatus::Interrupted,
            TurnFailureKind::ProviderExhausted
            | TurnFailureKind::Terminal
            | TurnFailureKind::Infrastructure
            | TurnFailureKind::Panicked => crate::session::ChatTurnStatus::Failed,
        };
        let interrupt = match failure.kind {
            TurnFailureKind::Cancelled => Some(crate::session::ChatTurnInterruptReason::UserStop),
            TurnFailureKind::ProviderExhausted => {
                Some(crate::session::ChatTurnInterruptReason::ProviderFailed)
            }
            TurnFailureKind::Terminal => match failure.reason() {
                Some(crate::failover::FailoverReason::CurrentToolGroupOverflow) => {
                    Some(crate::session::ChatTurnInterruptReason::CurrentToolGroupOverflow)
                }
                Some(crate::failover::FailoverReason::DispatchUnknown) => {
                    Some(crate::session::ChatTurnInterruptReason::DispatchUnknown)
                }
                _ => Some(crate::session::ChatTurnInterruptReason::Unknown),
            },
            TurnFailureKind::Infrastructure | TurnFailureKind::Panicked => {
                Some(crate::session::ChatTurnInterruptReason::Unknown)
            }
        };
        let turn_id = self.turn_id.clone();
        let run = self.run.clone();
        let message_for_db = message.clone();
        let converged = self
            .db
            .run(move |db| -> anyhow::Result<bool> {
                if let Some((run_id, true)) = run {
                    let _ = db.interrupt_stream_run(
                        &run_id,
                        0,
                        status,
                        interrupt.map(|reason| reason.as_str()),
                        Some(&message_for_db),
                    );
                }
                if let Some(turn_id) = turn_id.as_deref() {
                    return db.finish_chat_turn_once(
                        turn_id,
                        status,
                        interrupt,
                        Some(&message_for_db),
                        None,
                    );
                }
                Ok(false)
            })
            .await
            .unwrap_or(false);
        if converged {
            if let Some(turn_id) = self.turn_id.as_deref() {
                crate::chat_engine::stream_broadcast::broadcast_stream_end(
                    &self.session_id,
                    self.stream_id.as_deref(),
                    Some(turn_id),
                    Some(status),
                    interrupt,
                    Some(&message),
                );
                crate::chat_engine::active_turn::force_release(&self.session_id, turn_id);
            }
        }
        if let Some(stream_id) = self.stream_id.as_deref() {
            let _ = crate::chat_engine::stream_seq::end_if_stream(&self.session_id, stream_id);
        }
    }
}

async fn admit_source_proof(
    params: &mut ChatEngineParams,
    proof: SourceProof,
) -> Result<(), TurnFailure> {
    match proof {
        SourceProof::Channel(proof) => {
            if params.source != ChatSource::Channel {
                return Err(TurnFailure::new(
                    TurnFailureKind::Infrastructure,
                    "channel lineage proof was attached to a non-channel turn",
                ));
            }
            let session_id = params.session_id.clone();
            let session = params
                .session_db
                .run(move |db| db.get_session(&session_id))
                .await
                .map_err(TurnFailure::from)?
                .ok_or_else(|| {
                    TurnFailure::new(
                        TurnFailureKind::Infrastructure,
                        "channel session disappeared before turn admission",
                    )
                })?;
            let lineage = session.channel_info.ok_or_else(|| {
                TurnFailure::new(
                    TurnFailureKind::Infrastructure,
                    "channel turn requires durable session channel lineage",
                )
            })?;
            if session.agent_id != params.agent_id
                || lineage.channel_id != proof.channel_id
                || lineage.account_id != proof.account_id
                || lineage.chat_id != proof.chat_id
            {
                return Err(TurnFailure::new(
                    TurnFailureKind::Infrastructure,
                    "channel turn proof does not match durable session lineage",
                ));
            }
            let config = crate::config::cached_config();
            let account = config
                .channels
                .find_account(&lineage.account_id)
                .filter(|account| {
                    account.enabled && account.channel_id.to_string() == lineage.channel_id
                })
                .ok_or_else(|| {
                    TurnFailure::new(
                        TurnFailureKind::Infrastructure,
                        "channel account is disabled or no longer matches the session lineage",
                    )
                })?;
            params.auto_approve_tools = account.auto_approve_tools;
            params.channel_kb_context = Some(crate::knowledge::ChannelKbContext {
                channel_id: lineage.channel_id,
                account_id: lineage.account_id,
                chat_id: lineage.chat_id,
                is_group: !lineage.chat_type.eq_ignore_ascii_case("dm"),
            });
            Ok(())
        }
    }
}

async fn admit_interactive(
    params: &mut ChatEngineParams,
    mut admission: InteractiveAdmission,
) -> Result<(), TurnFailure> {
    let source = params.source;
    let turn_id = params.turn_id.clone().ok_or_else(|| {
        TurnFailure::new(
            TurnFailureKind::Infrastructure,
            "interactive turn is missing a visible turn id",
        )
    })?;
    if !matches!(source, ChatSource::Desktop | ChatSource::Http)
        || admission.lease.source != source
        || admission.lease.session_id != params.session_id
        || admission.lease.turn_id != turn_id
        || !Arc::ptr_eq(
            &crate::chat_engine::active_turn::current(&params.session_id)
                .map(|snapshot| snapshot.cancel)
                .ok_or_else(|| {
                    TurnFailure::new(
                        TurnFailureKind::Infrastructure,
                        "interactive active-turn lease is no longer current",
                    )
                })?,
            &params.cancel,
        )
    {
        return Err(TurnFailure::new(
            TurnFailureKind::Infrastructure,
            "interactive turn proof does not match the submitted request",
        ));
    }
    if admission.user_message.role != crate::session::MessageRole::User {
        return Err(TurnFailure::new(
            TurnFailureKind::Infrastructure,
            "interactive admission requires a user message",
        ));
    }
    admission.user_message.source = Some(source.as_str().to_string());
    let title_attachments_meta = admission.user_message.attachments_meta.clone();

    let stream_id = crate::chat_engine::stream_seq::begin(&params.session_id, source)
        .map_err(|error| TurnFailure::new(TurnFailureKind::Infrastructure, error.to_string()))?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let create = crate::session::CreateStreamRun {
        run_id,
        session_id: params.session_id.clone(),
        source: source.as_str().to_string(),
        stream_id: Some(stream_id.clone()),
        turn_id: Some(turn_id.clone()),
        provider_shape: None,
    };
    let session_id = params.session_id.clone();
    let user_message = admission.user_message;
    let message_for_stop = user_message.content.clone();
    let queue_request_id = user_message.queue_request_id.clone();
    let edit_message_id = admission.edit_message_id;
    let ui_surface = params.ui_surface;
    let (dispatch_request_id, dispatch_fingerprint) = admission
        .dispatch_identity
        .map(|(request_id, fingerprint)| (Some(request_id), Some(fingerprint)))
        .unwrap_or((None, None));
    let db = params.session_db.clone();
    // Queued turns retain the Stop generation from their durable enqueue
    // record. Direct turns normally carry the same proof as the transport
    // lease. Prefer the explicit durable record when present.
    let stop_admission = params
        .foreground_stop_admission
        .or(admission.lease.stop_admission);
    let session_for_db = session_id.clone();
    let turn_for_db = turn_id.clone();
    let stream_for_db = stream_id.clone();
    let create_for_db = create.clone();
    let outcome = db
        .run(move |db| {
            crate::chat_engine::active_turn::with_persistence_target(
                &session_for_db,
                &turn_for_db,
                || -> anyhow::Result<(_, crate::session::StreamRunRegistration)> {
                    if let Some(message_id) = edit_message_id {
                        return db.replace_last_user_message_for_edit_and_admit_stream(
                            &session_for_db,
                            message_id,
                            &user_message,
                            &turn_for_db,
                            source.as_str(),
                            ui_surface,
                            dispatch_request_id.as_deref(),
                            dispatch_fingerprint.as_deref(),
                            stop_admission,
                            &create_for_db,
                        );
                    }
                    let (message_id, _turn, registration) = db.admit_interactive_chat_turn(
                        &turn_for_db,
                        &session_for_db,
                        source.as_str(),
                        &stream_for_db,
                        &user_message,
                        ui_surface,
                        dispatch_request_id.as_deref(),
                        dispatch_fingerprint.as_deref(),
                        stop_admission,
                        &create_for_db,
                    )?;
                    Ok((message_id, registration))
                },
            )
        })
        .await;

    let registration = match outcome {
        Ok(crate::chat_engine::active_turn::PersistenceTargetOutcome::Committed((
            _,
            registration,
        ))) => registration,
        Ok(crate::chat_engine::active_turn::PersistenceTargetOutcome::CommittedAfterCancel((
            _,
            _registration,
        ))) => {
            settle_cancelled_interactive(
                params.session_db.clone(),
                session_id,
                turn_id,
                stream_id,
                message_for_stop,
                source,
                admission.bootstrap_request_id,
                admission.delete_new_session_on_cancel,
                queue_request_id,
                true,
            )
            .await;
            return Err(TurnFailure::cancelled(
                "chat stopped while prompt persistence completed",
            ));
        }
        Ok(crate::chat_engine::active_turn::PersistenceTargetOutcome::CancelledBeforeCommit) => {
            settle_cancelled_interactive(
                params.session_db.clone(),
                session_id,
                turn_id,
                stream_id,
                message_for_stop,
                source,
                admission.bootstrap_request_id,
                admission.delete_new_session_on_cancel,
                queue_request_id,
                false,
            )
            .await;
            return Err(TurnFailure::cancelled(
                "chat stopped before prompt persistence completed",
            ));
        }
        Err(error) => {
            let _ = crate::chat_engine::stream_seq::end_if_stream(&session_id, &stream_id);
            if let Some(request_id) = queue_request_id {
                let sid = session_id.clone();
                let tid = turn_id.clone();
                let _ = params
                    .session_db
                    .run(move |db| {
                        db.reconcile_failed_turn_message_dispatch(&sid, &request_id, &tid)
                    })
                    .await;
            }
            return Err(TurnFailure::new(
                TurnFailureKind::Infrastructure,
                error.to_string(),
            ));
        }
    };

    let title_session = params.session_id.clone();
    let title_prompt = message_for_stop;
    let _ = params
        .session_db
        .run(move |db| {
            crate::session::ensure_first_message_title(
                db,
                &title_session,
                &title_prompt,
                title_attachments_meta.as_deref(),
            )
        })
        .await;
    if admission.emit_session_created {
        params.event_sink.send(
            &serde_json::json!({
                "type": "session_created",
                "session_id": &params.session_id,
            })
            .to_string(),
        );
    }
    if source == ChatSource::Desktop {
        params.event_sink.send(
            &serde_json::json!({
                "type": "turn_started",
                "session_id": &params.session_id,
                "turn_id": &turn_id,
                "stream_id": &stream_id,
            })
            .to_string(),
        );
    }
    params.pre_admitted_stream = Some(crate::chat_engine::PreAdmittedStream {
        stream_id,
        registration,
    });
    params.foreground_stop_admission = stop_admission;
    params.active_turn_guard = Some(admission.lease.guard);
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn settle_cancelled_interactive(
    db: Arc<crate::session::SessionDB>,
    session_id: String,
    turn_id: String,
    stream_id: String,
    user_message: String,
    source: ChatSource,
    bootstrap_request_id: Option<String>,
    delete_new_session: bool,
    queue_request_id: Option<String>,
    persisted: bool,
) {
    crate::hooks::set_user_prompt_context(&session_id, None);
    if delete_new_session || !persisted {
        if let Some(cleanup) = crate::chat_engine::stop::PreTurnCancelCleanup::begin(
            db.clone(),
            session_id.clone(),
            bootstrap_request_id,
            delete_new_session,
            queue_request_id.map(|request_id| (request_id, turn_id.clone())),
        ) {
            cleanup.spawn();
        }
    } else {
        let _ = crate::chat_engine::stop::finalize_persisted_user_stop(
            db,
            session_id.clone(),
            turn_id.clone(),
            user_message,
            source,
        )
        .await;
    }
    crate::chat_engine::stream_broadcast::broadcast_stream_end(
        &session_id,
        Some(&stream_id),
        Some(&turn_id),
        Some(crate::session::ChatTurnStatus::Interrupted),
        Some(crate::session::ChatTurnInterruptReason::UserStop),
        None,
    );
    crate::chat_engine::active_turn::force_release(&session_id, &turn_id);
    let _ = crate::chat_engine::stream_seq::end_if_stream(&session_id, &stream_id);
}

/// Typed-failure variant used by kernel-owned subagent policy.
pub(crate) async fn submit_classified(
    submission: TurnSubmission,
) -> Result<AgentTurnOutput, TurnFailure> {
    TurnKernel::submit_classified(submission).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interactive_request(
        db: Arc<crate::session::SessionDB>,
        session_id: String,
        turn_id: String,
        cancel: Arc<AtomicBool>,
    ) -> TurnRequest {
        TurnRequest::new(
            session_id,
            crate::agent_loader::DEFAULT_AGENT_ID.to_string(),
            "hello".to_string(),
            db,
            crate::context_compact::CompactConfig::default(),
            cancel,
            Arc::new(crate::chat_engine::NoopEventSink),
        )
        .with_turn_id(turn_id)
        .with_reasoning_effort(Some("none".to_string()))
        .with_ui_surface(Some(crate::pet::ChatUiSurface::MainChat))
    }

    #[test]
    fn source_policy_matches_current_entrypoint_contracts() {
        assert!(source_policy(ChatSource::Desktop).carries_foreground_user_intent);
        assert!(source_policy(ChatSource::Http).broadcasts_to_user_ui);
        assert!(!source_policy(ChatSource::Channel).broadcasts_to_user_ui);
        assert!(!source_policy(ChatSource::Subagent).tracks_seq);
        assert!(source_policy(ChatSource::ParentInjection).broadcasts_to_user_ui);
        assert!(!source_policy(ChatSource::SessionTool).carries_foreground_user_intent);
        assert!(source_policy(ChatSource::Cron).holds_foreground_idle_guard);
        assert!(!source_policy(ChatSource::Eval).carries_foreground_user_intent);
        assert!(!source_policy(ChatSource::Eval).broadcasts_to_user_ui);
        assert!(source_policy(ChatSource::Eval).tracks_seq);
        assert!(!source_policy(ChatSource::Eval).fires_user_lifecycle_hooks);
        assert!(source_policy(ChatSource::Acp).holds_foreground_idle_guard);
        assert!(source_policy(ChatSource::Acp).fires_user_lifecycle_hooks);
        assert!(admit_persisted_turn_identity(ChatSource::Http, Some("turn-1")).is_ok());
        assert!(admit_persisted_turn_identity(ChatSource::Acp, None).is_ok());
        assert!(admit_persisted_turn_identity(ChatSource::Acp, Some("hook-only")).is_err());
    }

    #[test]
    fn provider_lease_rejects_model_from_another_config_snapshot() {
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(
                &tempfile::tempdir().unwrap().path().join("sessions.db"),
            )
            .unwrap(),
        );
        let request = TurnRequest::new(
            "session".to_string(),
            crate::agent_loader::DEFAULT_AGENT_ID.to_string(),
            "hello".to_string(),
            db,
            crate::context_compact::CompactConfig::default(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(crate::chat_engine::NoopEventSink),
        )
        .with_evaluation_model_chain(vec![crate::provider::ActiveModel {
            provider_id: "stale-provider".to_string(),
            model_id: "model".to_string(),
        }]);
        let lease = ProviderCredentialLease::from_providers(Vec::new());
        let (params, _) = request.into_engine_params(
            ChatSource::Eval,
            false,
            false,
            false,
            true,
            None,
            None,
            lease.providers,
            lease.revision,
        );

        let error = admit_provider_lease(&params).expect_err("stale provider must fail closed");
        assert_eq!(error.kind, TurnFailureKind::Infrastructure);
        assert!(error
            .to_string()
            .contains("outside the admitted config snapshot"));
    }

    #[test]
    fn strict_model_preference_fails_closed_before_fallback() {
        let config = crate::config::AppConfig::default();
        let error = resolve_configured_model_chain(
            crate::agent_loader::DEFAULT_AGENT_ID,
            Some("missing-provider::missing-model"),
            true,
            &config,
        )
        .expect_err("strict unavailable override must not fall through");
        assert!(error.to_string().contains("override is unavailable"));
        assert!(error.is_invalid_request());
    }

    #[test]
    fn isolated_evaluation_chain_is_rejected_for_production_source() {
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(
                &tempfile::tempdir().unwrap().path().join("sessions.db"),
            )
            .unwrap(),
        );
        let request = TurnRequest::new(
            "session".to_string(),
            crate::agent_loader::DEFAULT_AGENT_ID.to_string(),
            "hello".to_string(),
            db,
            crate::context_compact::CompactConfig::default(),
            Arc::new(AtomicBool::new(false)),
            Arc::new(crate::chat_engine::NoopEventSink),
        )
        .with_evaluation_model_chain(Vec::new());
        let lease = ProviderCredentialLease::from_providers(Vec::new());
        let (mut params, selection) = request.into_engine_params(
            ChatSource::Desktop,
            false,
            true,
            false,
            true,
            None,
            None,
            lease.providers,
            lease.revision,
        );
        let error = admit_model_selection(&mut params, selection, None)
            .expect_err("production source must not admit fixture-selected models");
        assert!(error.to_string().contains("forbidden for production turns"));
    }

    #[tokio::test]
    async fn desktop_admission_mints_stream_identity_in_the_atomic_ledger_boundary() {
        let _lock = crate::chat_engine::active_turn::test_lock();
        let dir = tempfile::tempdir().unwrap();
        let db = Arc::new(
            crate::session::SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db"))
                .unwrap(),
        );
        let session = db
            .create_session(crate::agent_loader::DEFAULT_AGENT_ID)
            .unwrap();
        let turn_id = "kernel-desktop-turn".to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let foreground_admission =
            crate::chat_engine::active_turn::begin_durable_foreground_request(
                db.as_ref(),
                Some(&session.id),
            )
            .unwrap();
        let direct_admission = db
            .reserve_direct_turn_admission(
                &session.id,
                &turn_id,
                crate::session::QueuedTurnMessageSource::Desktop,
                foreground_admission.durable_stop_admission(),
            )
            .unwrap()
            .expect("desktop transport must reserve direct FIFO admission");
        let lease = TurnKernel::begin_desktop(
            foreground_admission,
            &session.id,
            turn_id.clone(),
            Some("client-request".to_string()),
            cancel.clone(),
        )
        .unwrap();
        let request = interactive_request(db.clone(), session.id.clone(), turn_id.clone(), cancel);
        let admission = InteractiveAdmission::desktop(
            lease,
            crate::session::NewMessage::user("hello"),
            None,
            None,
            None,
            false,
            false,
        );

        let mut submission = TurnSubmission::desktop(request, admission);
        let admission = submission
            .interactive
            .take()
            .expect("desktop submission carries interactive admission");
        admit_interactive(&mut submission.params, admission)
            .await
            .unwrap();
        let params = submission.params;
        let stream = params
            .pre_admitted_stream
            .as_ref()
            .expect("interactive admission must mint a stream run");
        let turn = db.get_chat_turn(&turn_id).unwrap().unwrap();
        let persisted_run = db.latest_stream_run(&session.id).unwrap().unwrap();
        assert_eq!(turn.stream_id.as_deref(), Some(stream.stream_id.as_str()));
        assert_eq!(persisted_run.run_id, stream.registration.run_id);
        assert_eq!(persisted_run.turn_id.as_deref(), Some(turn_id.as_str()));
        assert_eq!(db.load_session_messages(&session.id).unwrap().len(), 1);
        assert!(!db.release_direct_turn_admission(direct_admission).unwrap());
        drop(params);
    }
}
