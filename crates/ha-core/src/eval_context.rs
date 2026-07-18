//! Opt-in attribution for isolated real-model evaluation campaigns.
//!
//! The registry is deliberately inert for normal product traffic. A server
//! accepts [`EvalRunContext`] only when `HA_MODEL_EVAL_MODE=1`, registers the
//! root Session for the duration of a turn, and lets model/tool instrumentation
//! enrich the matching trial. No Provider secret, prompt, or tool output is
//! retained here.

use crate::model_usage::ModelUsageEvent;
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

const MAX_RETAINED_TRIALS: usize = 256;
const MAX_TRIAL_EVENTS: usize = 4_096;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalRunContext {
    pub evidence_kind: String,
    pub campaign_id: String,
    pub plan_digest: String,
    pub suite_id: String,
    pub suite_version: String,
    pub suite_digest: String,
    pub case_id: String,
    pub case_digest: String,
    pub trial_id: String,
    pub trial_index: u16,
    pub arm: String,
    pub fault_profile: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orchestration_profile: Option<String>,
    pub trace_id: String,
    pub root_span_id: String,
    pub model_role: String,
    pub seed: u64,
    pub source: String,
    pub commit_sha: String,
    pub dirty: bool,
    pub app_version: String,
    #[serde(default)]
    pub required_signals: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub faults: Vec<EvalFaultDirective>,
    /// Strict per-trial runtime ceilings copied from the immutable plan. Cost
    /// is represented as integer micro-dollars so the context remains Eq and
    /// cannot acquire NaN/rounding ambiguity while crossing async boundaries.
    #[serde(default)]
    pub budget: EvalBudgetLimits,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalBudgetLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_wall_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_model_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_micros: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tool_calls: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_agents: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_concurrency: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalFaultKind {
    ProviderResponse,
    ToolResponse,
    SchedulerOrder,
    ProcessRestart,
    StorageState,
    UserEvent,
    EnvironmentEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvalFaultDirective {
    pub id: String,
    pub kind: EvalFaultKind,
    pub trigger: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    pub max_activations: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalFaultAction {
    pub fault_id: String,
    pub delay_ms: u64,
    pub error_class: Option<&'static str>,
}

impl EvalRunContext {
    pub fn validate(&self) -> Result<()> {
        for (label, value) in [
            ("evidenceKind", self.evidence_kind.as_str()),
            ("campaignId", self.campaign_id.as_str()),
            ("suiteId", self.suite_id.as_str()),
            ("suiteVersion", self.suite_version.as_str()),
            ("caseId", self.case_id.as_str()),
            ("trialId", self.trial_id.as_str()),
            ("arm", self.arm.as_str()),
            ("faultProfile", self.fault_profile.as_str()),
            ("traceId", self.trace_id.as_str()),
            ("rootSpanId", self.root_span_id.as_str()),
            ("modelRole", self.model_role.as_str()),
            ("source", self.source.as_str()),
            ("appVersion", self.app_version.as_str()),
        ] {
            if value.is_empty()
                || value.len() > 128
                || !value
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
            {
                bail!("eval context {label} has an invalid identifier");
            }
        }
        if self.evidence_kind != "model_campaign"
            || !matches!(
                self.source.as_str(),
                "local_app" | "local_cli" | "dedicated_runner" | "github_actions"
            )
            || !is_lower_hex(&self.plan_digest, 64)
            || !is_lower_hex(&self.suite_digest, 64)
            || !is_lower_hex(&self.case_digest, 64)
            || !matches!(self.commit_sha.len(), 40 | 64)
            || !self.commit_sha.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("eval context contains invalid immutable evidence identity");
        }
        if !matches!(self.fault_profile.as_str(), "clean" | "chaos")
            || self
                .orchestration_profile
                .as_deref()
                .is_some_and(|profile| !valid_eval_identifier(profile, 96))
        {
            bail!("eval context contains an invalid experiment profile");
        }
        if self.required_signals.len() > 32 {
            bail!("eval context declares too many required signals");
        }
        for signal in &self.required_signals {
            if signal.is_empty()
                || signal.len() > 64
                || !signal
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
            {
                bail!("eval context contains an invalid required signal");
            }
        }
        if self.faults.len() > 16 {
            bail!("eval context declares too many fault directives");
        }
        let mut fault_ids = HashSet::new();
        for fault in &self.faults {
            if !fault_ids.insert(fault.id.as_str())
                || !valid_eval_identifier(&fault.id, 96)
                || !valid_eval_identifier(&fault.trigger, 128)
                || !(1..=8).contains(&fault.max_activations)
                || fault.params.len() > 16
                || fault.params.iter().any(|(key, value)| {
                    !valid_eval_identifier(key, 64)
                        || !(value.is_boolean()
                            || value.is_number()
                            || value.as_str().is_some_and(|value| value.len() <= 256)
                            || value.as_array().is_some_and(|items| {
                                items.len() <= 32
                                    && items.iter().all(|item| {
                                        item.is_number()
                                            || item.as_str().is_some_and(|value| value.len() <= 64)
                                    })
                            }))
                })
            {
                bail!("eval context contains an invalid fault directive");
            }
        }
        // A zero remaining ceiling is valid for a durable continuation: it
        // means the pre-restart phase consumed that resource and the next
        // model/tool/child operation must fail closed immediately.
        Ok(())
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_eval_identifier(value: &str, maximum: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':'))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalToolOutcome {
    Succeeded,
    Failed,
    Cancelled,
    ParseError,
}

#[derive(Debug, Clone)]
struct SessionEntry {
    context: EvalRunContext,
    depth: u16,
    /// Stable parent for observations emitted by this Session. Async retain
    /// guards keep work alive but do not replace this with an unrelated job
    /// merely because that job happened to register most recently.
    causal_span_id: String,
    /// Active causal spans for this Session. The most recently registered
    /// span is the parent for model/tool/lifecycle observations emitted on
    /// the current task. Guards remove their own span on drop, so nested turns,
    /// async jobs and child Agents do not leave a dangling synthetic parent.
    active_spans: Vec<String>,
}

#[derive(Debug)]
struct TrialAccumulator {
    context: EvalRunContext,
    root_session_id: String,
    started: Instant,
    root_closed: bool,
    turn_count: u32,
    active_sessions: HashSet<String>,
    all_sessions: BTreeSet<String>,
    active_work: u32,
    background_model_work: u32,
    max_concurrency: u32,
    max_agent_depth: u16,
    observed_signals: BTreeSet<String>,
    sequence: u64,
    model_calls: u64,
    model_retries: u64,
    failovers: u64,
    model_active_ms: u64,
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    ttft_ms: Option<u64>,
    cost_usd: Option<f64>,
    price_components: BTreeSet<String>,
    attempted_tools: u64,
    logical_tool_calls: HashSet<String>,
    tool_call_fingerprints: HashSet<String>,
    duplicate_tools: u64,
    pending_tool_results: HashSet<String>,
    consumed_tool_results: u64,
    succeeded_tools: u64,
    failed_tools: u64,
    cancelled_tools: u64,
    parse_errors: u64,
    tool_active_ms: u64,
    queue_wait_ms: u64,
    spawned_agents: u64,
    child_active_ms: u64,
    async_jobs: u64,
    loop_iterations: u64,
    replans: u64,
    checkpoints: u64,
    resumes: u64,
    handoffs: u64,
    injection_keys: HashSet<String>,
    duplicate_injections: u64,
    budget_exhausted: bool,
    budget_exhaustion_reasons: BTreeSet<String>,
    warnings: Vec<String>,
    fault_activations: HashMap<String, u32>,
    events: Vec<EvalEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EvalEvent {
    seq: u64,
    event: String,
    timestamp_ms: u64,
    span_id: String,
    parent_span_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    status: String,
    duration_ms: u64,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    attributes: BTreeMap<String, Value>,
}

impl TrialAccumulator {
    fn new(context: EvalRunContext, session_id: String) -> Self {
        let mut all_sessions = BTreeSet::new();
        all_sessions.insert(session_id.clone());
        Self {
            context,
            root_session_id: session_id.clone(),
            started: Instant::now(),
            root_closed: false,
            turn_count: 0,
            active_sessions: HashSet::new(),
            all_sessions,
            active_work: 0,
            background_model_work: 0,
            max_concurrency: 0,
            max_agent_depth: 0,
            observed_signals: BTreeSet::new(),
            sequence: 1,
            model_calls: 0,
            model_retries: 0,
            failovers: 0,
            model_active_ms: 0,
            input_tokens: Some(0),
            output_tokens: Some(0),
            cache_read_tokens: Some(0),
            cache_write_tokens: Some(0),
            ttft_ms: None,
            cost_usd: Some(0.0),
            price_components: BTreeSet::new(),
            attempted_tools: 0,
            logical_tool_calls: HashSet::new(),
            tool_call_fingerprints: HashSet::new(),
            duplicate_tools: 0,
            pending_tool_results: HashSet::new(),
            consumed_tool_results: 0,
            succeeded_tools: 0,
            failed_tools: 0,
            cancelled_tools: 0,
            parse_errors: 0,
            tool_active_ms: 0,
            queue_wait_ms: 0,
            spawned_agents: 0,
            child_active_ms: 0,
            async_jobs: 0,
            loop_iterations: 0,
            replans: 0,
            checkpoints: 0,
            resumes: 0,
            handoffs: 0,
            injection_keys: HashSet::new(),
            duplicate_injections: 0,
            budget_exhausted: false,
            budget_exhaustion_reasons: BTreeSet::new(),
            warnings: Vec::new(),
            fault_activations: HashMap::new(),
            events: Vec::new(),
        }
    }

    fn bump(&mut self) {
        self.sequence = self.sequence.saturating_add(1);
    }

    fn new_span_id(&self, event: &str, key: Option<&str>) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.context.trial_id.as_bytes());
        hasher.update(self.sequence.to_be_bytes());
        hasher.update(event.as_bytes());
        if let Some(key) = key {
            hasher.update(key.as_bytes());
        }
        let digest = format!("{:x}", hasher.finalize());
        format!("span_{}", &digest[..24])
    }

    fn start_span(
        &mut self,
        event: impl Into<String>,
        key: Option<String>,
        parent_span_id: impl Into<String>,
    ) -> String {
        let event = event.into();
        let span_id = self.new_span_id(&event, key.as_deref());
        self.push_event_for_span(
            event,
            key,
            "started",
            0,
            span_id.clone(),
            parent_span_id.into(),
            BTreeMap::new(),
        );
        span_id
    }

    fn push_leaf_event(
        &mut self,
        event: impl Into<String>,
        key: Option<String>,
        status: impl Into<String>,
        duration_ms: u64,
        parent_span_id: impl Into<String>,
    ) -> String {
        self.push_leaf_event_with_attributes(
            event,
            key,
            status,
            duration_ms,
            parent_span_id,
            BTreeMap::new(),
        )
    }

    fn push_leaf_event_with_attributes(
        &mut self,
        event: impl Into<String>,
        key: Option<String>,
        status: impl Into<String>,
        duration_ms: u64,
        parent_span_id: impl Into<String>,
        attributes: BTreeMap<String, Value>,
    ) -> String {
        let event = event.into();
        let span_id = self.new_span_id(&event, key.as_deref());
        self.push_event_for_span(
            event,
            key,
            status,
            duration_ms,
            span_id.clone(),
            parent_span_id.into(),
            attributes,
        );
        span_id
    }

    #[allow(clippy::too_many_arguments)]
    fn push_event_for_span(
        &mut self,
        event: impl Into<String>,
        key: Option<String>,
        status: impl Into<String>,
        duration_ms: u64,
        span_id: String,
        parent_span_id: String,
        attributes: BTreeMap<String, Value>,
    ) {
        if self.events.len() >= MAX_TRIAL_EVENTS {
            if !self
                .warnings
                .iter()
                .any(|warning| warning == "eval event buffer exhausted")
            {
                self.warnings
                    .push("eval event buffer exhausted".to_string());
            }
            return;
        }
        let timestamp_ms = self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        let event = event.into();
        self.events.push(EvalEvent {
            seq: self.sequence,
            event,
            timestamp_ms,
            span_id,
            parent_span_id,
            key,
            status: status.into(),
            duration_ms,
            attributes,
        });
        self.bump();
    }
}

/// Register one production HTTP chat turn under a longer-lived trial root.
/// The turn guard closes when the handler returns; the Harness explicitly
/// closes the trial root after its final scripted/replay user turn.
pub fn register_http_turn_session(
    session_id: &str,
    context: EvalRunContext,
) -> Result<EvalSessionGuard> {
    context.validate()?;
    let mut state = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("eval context registry is poisoned"))?;
    let trial_id = context.trial_id.clone();
    let new_trial = !state.trials.contains_key(&trial_id);
    if let Some(existing) = state.trials.get(&trial_id) {
        if existing.context != context || existing.root_session_id != session_id {
            bail!("trial already exists with a different eval context or root Session");
        }
        if existing.root_closed {
            bail!("trial root is already closed");
        }
    } else {
        state.trials.insert(
            trial_id.clone(),
            TrialAccumulator::new(context.clone(), session_id.to_string()),
        );
        state.order.retain(|existing| existing != &trial_id);
        state.order.push_back(trial_id.clone());
        prune_trials(&mut state);
    }

    let (span_id, root_span_id) = {
        let trial = state
            .trials
            .get_mut(&trial_id)
            .ok_or_else(|| anyhow::anyhow!("model evaluation trial was not registered"))?;
        trial.active_work = trial.active_work.saturating_add(1);
        trial.turn_count = trial.turn_count.saturating_add(1);
        trial.active_sessions.insert(session_id.to_string());
        trial.all_sessions.insert(session_id.to_string());
        trial.max_concurrency = trial.max_concurrency.max(trial.active_work);
        let root_span_id = trial.context.root_span_id.clone();
        let span_id = trial.start_span(
            "session.turn",
            Some(session_id.to_string()),
            root_span_id.clone(),
        );
        if new_trial {
            activate_passive_faults(trial, EvalFaultKind::ProcessRestart, None, &span_id);
        } else {
            activate_passive_faults(trial, EvalFaultKind::UserEvent, None, &span_id);
        }
        (span_id, root_span_id)
    };
    match state.sessions.get_mut(session_id) {
        Some(entry) => {
            if entry.context != context {
                bail!("Session already has a different eval context");
            }
            entry.causal_span_id.clone_from(&span_id);
            entry.active_spans.push(span_id.clone());
        }
        None => {
            state.sessions.insert(
                session_id.to_string(),
                SessionEntry {
                    context,
                    depth: 0,
                    causal_span_id: span_id.clone(),
                    active_spans: vec![span_id.clone()],
                },
            );
        }
    }
    Ok(EvalSessionGuard {
        session_id: session_id.to_string(),
        trial_id,
        root: false,
        event: "session.turn",
        span_id,
        parent_span_id: root_span_id,
        started: Instant::now(),
        armed: true,
    })
}

/// Idempotently close the trial root after the final user turn. Descendant
/// work remains visible through `active_work`, so a premature assistant return
/// still yields an open trace until cleanup/cancellation settles it.
pub fn finish_trial_root(trial_id: &str) -> Result<()> {
    let mut state = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("eval context registry is poisoned"))?;
    let dormant_root_session = {
        let trial = state
            .trials
            .get_mut(trial_id)
            .ok_or_else(|| anyhow::anyhow!("model evaluation trial was not found"))?;
        if !trial.root_closed {
            trial.root_closed = true;
            trial.push_leaf_event(
                "session.root.closed",
                Some(trial.root_session_id.clone()),
                "completed",
                0,
                trial.context.root_span_id.clone(),
            );
        }
        trial.root_session_id.clone()
    };
    if state
        .sessions
        .get(&dormant_root_session)
        .is_some_and(|entry| entry.active_spans.is_empty())
    {
        state.sessions.remove(&dormant_root_session);
    }
    Ok(())
}

#[derive(Default)]
struct Registry {
    sessions: HashMap<String, SessionEntry>,
    trials: HashMap<String, TrialAccumulator>,
    order: VecDeque<String>,
}

fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

#[must_use]
pub struct EvalSessionGuard {
    session_id: String,
    trial_id: String,
    root: bool,
    event: &'static str,
    span_id: String,
    parent_span_id: String,
    started: Instant,
    armed: bool,
}

impl EvalSessionGuard {
    pub fn release(mut self) {
        self.unregister();
    }

    fn unregister(&mut self) {
        if !self.armed {
            return;
        }
        if let Ok(mut state) = registry().lock() {
            let root_identity = state
                .trials
                .get(&self.trial_id)
                .map(|trial| (trial.root_closed, trial.context.root_span_id.clone()));
            let (session_became_inactive, keep_root_identity) = state
                .sessions
                .get_mut(&self.session_id)
                .map(|entry| {
                    if let Some(index) = entry
                        .active_spans
                        .iter()
                        .rposition(|span| span == &self.span_id)
                    {
                        entry.active_spans.remove(index);
                    }
                    let inactive = entry.active_spans.is_empty();
                    let keep_root = inactive
                        && entry.depth == 0
                        && root_identity.as_ref().is_some_and(|(closed, _)| !closed);
                    if keep_root {
                        entry
                            .causal_span_id
                            .clone_from(&root_identity.as_ref().unwrap().1);
                    }
                    (inactive, keep_root)
                })
                .unwrap_or((false, false));
            let remove_session = session_became_inactive && !keep_root_identity;
            if remove_session {
                state.sessions.remove(&self.session_id);
            }
            if let Some(trial) = state.trials.get_mut(&self.trial_id) {
                trial.active_work = trial.active_work.saturating_sub(1);
                if self.event == "model_automation.run" {
                    trial.background_model_work = trial.background_model_work.saturating_sub(1);
                }
                if self.event == "subagent.run" {
                    trial.child_active_ms = trial.child_active_ms.saturating_add(
                        self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    );
                }
                trial.push_event_for_span(
                    self.event,
                    Some(self.session_id.clone()),
                    "completed",
                    self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
                    self.span_id.clone(),
                    self.parent_span_id.clone(),
                    BTreeMap::new(),
                );
                if session_became_inactive {
                    trial.active_sessions.remove(&self.session_id);
                }
                if self.root && !trial.root_closed {
                    trial.root_closed = true;
                    trial.push_leaf_event(
                        "session.root.closed",
                        Some(trial.root_session_id.clone()),
                        "completed",
                        0,
                        trial.context.root_span_id.clone(),
                    );
                }
            }
            let dormant_root_session = state.trials.get(&self.trial_id).and_then(|trial| {
                (trial.root_closed && trial.active_work == 0).then(|| trial.root_session_id.clone())
            });
            if let Some(root_session_id) = dormant_root_session {
                if state
                    .sessions
                    .get(&root_session_id)
                    .is_some_and(|entry| entry.active_spans.is_empty())
                {
                    state.sessions.remove(&root_session_id);
                }
            }
        }
        self.armed = false;
    }
}

impl Drop for EvalSessionGuard {
    fn drop(&mut self) {
        self.unregister();
    }
}

pub fn model_eval_mode_enabled() -> bool {
    std::env::var("HA_MODEL_EVAL_MODE").as_deref() == Ok("1")
}

pub fn register_root_session(
    session_id: &str,
    context: EvalRunContext,
) -> Result<EvalSessionGuard> {
    context.validate()?;
    let mut state = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("eval context registry is poisoned"))?;
    if state.sessions.contains_key(session_id) {
        bail!("session already has an active eval context");
    }
    let trial_id = context.trial_id.clone();
    state.trials.insert(
        trial_id.clone(),
        TrialAccumulator::new(context, session_id.to_string()),
    );
    state.order.retain(|existing| existing != &trial_id);
    state.order.push_back(trial_id.clone());
    prune_trials(&mut state);
    let trial = state
        .trials
        .get_mut(&trial_id)
        .ok_or_else(|| anyhow::anyhow!("model evaluation root trial was not registered"))?;
    trial.active_work = 1;
    trial.max_concurrency = 1;
    trial.turn_count = 1;
    trial.active_sessions.insert(session_id.to_string());
    let parent_span_id = trial.context.root_span_id.clone();
    let span_id = trial.start_span(
        "session.root",
        Some(session_id.to_string()),
        parent_span_id.clone(),
    );
    let stored_context = trial.context.clone();
    state.sessions.insert(
        session_id.to_string(),
        SessionEntry {
            context: stored_context,
            depth: 0,
            causal_span_id: span_id.clone(),
            active_spans: vec![span_id.clone()],
        },
    );
    Ok(EvalSessionGuard {
        session_id: session_id.to_string(),
        trial_id,
        root: true,
        event: "session.root",
        span_id,
        parent_span_id,
        started: Instant::now(),
        armed: true,
    })
}

/// Propagate a parent Session's immutable campaign identity to a child Session.
/// The returned guard must live for the full child execution.
pub fn inherit_session(
    parent_session_id: &str,
    child_session_id: &str,
) -> Option<EvalSessionGuard> {
    let context = context_for_session(parent_session_id)?;
    register_child_session_from_parent(parent_session_id, child_session_id, context).ok()
}

pub fn register_child_session(
    child_session_id: &str,
    context: EvalRunContext,
) -> Result<EvalSessionGuard> {
    register_child_session_with_depth(None, child_session_id, context, 1)
}

pub fn register_child_session_from_parent(
    parent_session_id: &str,
    child_session_id: &str,
    context: EvalRunContext,
) -> Result<EvalSessionGuard> {
    let depth = registry()
        .lock()
        .ok()
        .and_then(|state| {
            state
                .sessions
                .get(parent_session_id)
                .map(|entry| entry.depth)
        })
        .unwrap_or(0)
        .saturating_add(1);
    register_child_session_with_depth(Some(parent_session_id), child_session_id, context, depth)
}

fn register_child_session_with_depth(
    parent_session_id: Option<&str>,
    child_session_id: &str,
    context: EvalRunContext,
    depth: u16,
) -> Result<EvalSessionGuard> {
    context.validate()?;
    let mut state = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("eval context registry is poisoned"))?;
    if state.sessions.contains_key(child_session_id) {
        bail!("child session already has an active eval context");
    }
    let trial_id = context.trial_id.clone();
    let parent_span_id = parent_session_id
        .and_then(|parent| state.sessions.get(parent))
        .map(|entry| entry.causal_span_id.clone())
        .unwrap_or_else(|| context.root_span_id.clone());
    let span_id = if let Some(trial) = state.trials.get_mut(&trial_id) {
        if trial
            .context
            .budget
            .max_agents
            .is_some_and(|limit| trial.spawned_agents >= u64::from(limit))
        {
            mark_budget_exhausted(trial, "agents", "subagent", &parent_span_id);
            bail!("evaluation budget exhausted: agents");
        }
        if trial
            .context
            .budget
            .max_concurrency
            .is_some_and(|limit| trial.active_work >= limit)
        {
            mark_budget_exhausted(trial, "concurrency", "subagent", &parent_span_id);
            bail!("evaluation budget exhausted: concurrency");
        }
        trial.active_sessions.insert(child_session_id.to_string());
        trial.all_sessions.insert(child_session_id.to_string());
        trial.active_work = trial.active_work.saturating_add(1);
        trial.max_concurrency = trial.max_concurrency.max(trial.active_work);
        trial.max_agent_depth = trial.max_agent_depth.max(depth);
        trial.spawned_agents = trial.spawned_agents.saturating_add(1);
        trial.observed_signals.insert("subagent".to_string());
        trial.start_span(
            "subagent.run",
            Some(child_session_id.to_string()),
            parent_span_id.clone(),
        )
    } else {
        bail!("child Session references an unknown model-eval trial");
    };
    state.sessions.insert(
        child_session_id.to_string(),
        SessionEntry {
            context,
            depth,
            causal_span_id: span_id.clone(),
            active_spans: vec![span_id.clone()],
        },
    );
    Ok(EvalSessionGuard {
        session_id: child_session_id.to_string(),
        trial_id,
        root: false,
        event: "subagent.run",
        span_id,
        parent_span_id,
        started: Instant::now(),
        armed: true,
    })
}

/// Keep attribution for background work that continues in the same Session
/// after the foreground turn returns.
pub fn retain_session(session_id: &str) -> Option<EvalSessionGuard> {
    let mut state = registry().lock().ok()?;
    let (trial_id, parent_span_id) = {
        let entry = state.sessions.get(session_id)?;
        (entry.context.trial_id.clone(), entry.causal_span_id.clone())
    };
    let span_id = if let Some(trial) = state.trials.get_mut(&trial_id) {
        trial.active_work = trial.active_work.saturating_add(1);
        trial.max_concurrency = trial.max_concurrency.max(trial.active_work);
        trial.async_jobs = trial.async_jobs.saturating_add(1);
        trial.observed_signals.insert("async_jobs".to_string());
        trial.start_span(
            "async_job.run",
            Some(session_id.to_string()),
            parent_span_id.clone(),
        )
    } else {
        return None;
    };
    state
        .sessions
        .get_mut(session_id)?
        .active_spans
        .push(span_id.clone());
    Some(EvalSessionGuard {
        session_id: session_id.to_string(),
        trial_id,
        root: false,
        event: "async_job.run",
        span_id,
        parent_span_id,
        started: Instant::now(),
        armed: true,
    })
}

/// Keep the evaluation identity alive for product-owned background model
/// automation such as session-title generation and memory extraction. These
/// calls are real Provider work and must be included in trial Token/cost
/// accounting, but they are reported separately from durable async jobs so a
/// cosmetic background call cannot masquerade as unfinished task work.
pub fn retain_model_automation(session_id: &str) -> Result<Option<EvalSessionGuard>> {
    ensure_model_budget(Some(session_id))?;
    ensure_background_work_budget(Some(session_id))?;
    let mut state = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("eval context registry is poisoned"))?;
    let Some(entry) = state.sessions.get(session_id) else {
        return Ok(None);
    };
    let trial_id = entry.context.trial_id.clone();
    let Some(trial) = state.trials.get_mut(&trial_id) else {
        return Ok(None);
    };
    let parent_span_id = trial.context.root_span_id.clone();
    trial.active_work = trial.active_work.saturating_add(1);
    trial.background_model_work = trial.background_model_work.saturating_add(1);
    trial.max_concurrency = trial.max_concurrency.max(trial.active_work);
    let span_id = trial.start_span(
        "model_automation.run",
        Some(session_id.to_string()),
        parent_span_id.clone(),
    );
    let Some(entry) = state.sessions.get_mut(session_id) else {
        return Ok(None);
    };
    entry.active_spans.push(span_id.clone());
    Ok(Some(EvalSessionGuard {
        session_id: session_id.to_string(),
        trial_id,
        root: false,
        event: "model_automation.run",
        span_id,
        parent_span_id,
        started: Instant::now(),
        armed: true,
    }))
}

pub fn context_for_session(session_id: &str) -> Option<EvalRunContext> {
    registry()
        .lock()
        .ok()?
        .sessions
        .get(session_id)
        .map(|entry| entry.context.clone())
}

/// Enforce compute-matched single-Agent comparison arms at both schema and
/// execution boundaries. The restriction is active only for authenticated
/// model-eval Sessions and cannot affect normal product traffic.
pub fn tool_allowed_for_experiment(session_id: Option<&str>, tool_name: &str) -> bool {
    let Some(context) = session_id.and_then(context_for_session) else {
        return true;
    };
    if context
        .orchestration_profile
        .as_deref()
        .is_some_and(|profile| profile.starts_with("single_agent"))
    {
        !matches!(
            tool_name,
            crate::tools::TOOL_SUBAGENT | crate::tools::TOOL_TEAM
        )
    } else {
        true
    }
}

/// Fail closed before a Provider request once an immutable per-trial ceiling
/// has been reached. Usage-based ceilings are checked against completed
/// Provider attempts; output overshoot is additionally bounded by
/// [`remaining_output_tokens`].
pub fn ensure_model_budget(session_id: Option<&str>) -> Result<()> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    with_budget_check(session_id, "model", |trial| {
        let budget = &trial.context.budget;
        if budget
            .max_wall_ms
            .is_some_and(|limit| elapsed_ms(trial) >= limit)
        {
            return Some("wall_time".to_string());
        }
        if budget
            .max_model_calls
            .is_some_and(|limit| trial.model_calls >= limit)
        {
            return Some("model_calls".to_string());
        }
        if exact_limit_reached(
            trial.input_tokens,
            budget.max_input_tokens,
            trial.model_calls,
        ) {
            return Some("input_tokens".to_string());
        }
        if exact_limit_reached(
            trial.output_tokens,
            budget.max_output_tokens,
            trial.model_calls,
        ) {
            return Some("output_tokens".to_string());
        }
        if let Some(limit) = budget.max_cost_micros {
            match trial.cost_usd {
                Some(cost) if usd_to_micros(cost) >= limit => return Some("cost".to_string()),
                None if trial.model_calls > 0 => return Some("cost_unknown".to_string()),
                _ => {}
            }
        }
        None
    })
}

/// Remaining exact Provider output-token allowance for the next main round.
/// `None` means no eval ceiling is active; callers still apply product limits.
pub fn remaining_output_tokens(session_id: Option<&str>) -> Option<u64> {
    let session_id = session_id?;
    let state = registry().lock().ok()?;
    let entry = state.sessions.get(session_id)?;
    let trial = state.trials.get(&entry.context.trial_id)?;
    let limit = trial.context.budget.max_output_tokens?;
    let used = match trial.output_tokens {
        Some(value) => value,
        None if trial.model_calls == 0 => 0,
        None => limit,
    };
    Some(limit.saturating_sub(used))
}

/// Reject one more tool attempt before dispatch when the strict per-trial
/// tool-call or wall budget has been reached.
pub fn ensure_tool_budget(session_id: Option<&str>) -> Result<()> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    with_budget_check(session_id, "tool", |trial| {
        let budget = &trial.context.budget;
        if budget
            .max_wall_ms
            .is_some_and(|limit| elapsed_ms(trial) >= limit)
        {
            return Some("wall_time".to_string());
        }
        budget
            .max_tool_calls
            .filter(|limit| trial.attempted_tools >= *limit)
            .map(|_| "tool_calls".to_string())
    })
}

/// Background work consumes an execution slot in addition to its initiating
/// tool call. This check is performed before a durable job row is created.
pub fn ensure_background_work_budget(session_id: Option<&str>) -> Result<()> {
    let Some(session_id) = session_id else {
        return Ok(());
    };
    with_budget_check(session_id, "async_work", |trial| {
        trial
            .context
            .budget
            .max_concurrency
            .filter(|limit| trial.active_work >= *limit)
            .map(|_| "concurrency".to_string())
    })
}

fn elapsed_ms(trial: &TrialAccumulator) -> u64 {
    trial
        .started
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

fn exact_limit_reached(value: Option<u64>, limit: Option<u64>, observed_calls: u64) -> bool {
    match (value, limit) {
        (Some(value), Some(limit)) => value >= limit,
        (None, Some(_)) => observed_calls > 0,
        _ => false,
    }
}

fn usd_to_micros(value: f64) -> u64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value * 1_000_000.0).ceil().min(u64::MAX as f64) as u64
}

fn with_budget_check(
    session_id: &str,
    operation: &str,
    check: impl FnOnce(&TrialAccumulator) -> Option<String>,
) -> Result<()> {
    let mut state = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("eval context registry is poisoned"))?;
    let (trial_id, parent_span_id) = match state.sessions.get(session_id) {
        Some(entry) => (entry.context.trial_id.clone(), entry.causal_span_id.clone()),
        None => return Ok(()),
    };
    let Some(trial) = state.trials.get_mut(&trial_id) else {
        return Ok(());
    };
    let Some(reason) = check(trial) else {
        return Ok(());
    };
    mark_budget_exhausted(trial, &reason, operation, &parent_span_id);
    bail!("evaluation budget exhausted: {reason}")
}

fn mark_budget_exhausted(
    trial: &mut TrialAccumulator,
    reason: &str,
    operation: &str,
    parent_span_id: &str,
) {
    trial.budget_exhausted = true;
    if trial.budget_exhaustion_reasons.insert(reason.to_string()) {
        trial.push_leaf_event(
            "budget.exhausted",
            Some(format!("{operation}:{reason}")),
            "blocked",
            0,
            parent_span_id.to_string(),
        );
    }
}

pub fn enrich_usage_metadata(event: &mut ModelUsageEvent) {
    let Some(session_id) = event.session_id.as_deref() else {
        return;
    };
    let Some(context) = context_for_session(session_id) else {
        return;
    };
    let eval = serde_json::to_value(&context).unwrap_or(Value::Null);
    match event.metadata.as_mut() {
        Some(Value::Object(object)) => {
            object.insert("eval".to_string(), eval);
        }
        Some(existing) => {
            let previous = std::mem::replace(existing, Value::Null);
            *existing = json!({"eval": eval, "original": previous});
        }
        None => event.metadata = Some(json!({"eval": eval})),
    }
}

pub fn record_model_usage(event: &ModelUsageEvent) {
    let Some(session_id) = event.session_id.as_deref() else {
        return;
    };
    with_trial_for_session(session_id, |trial, parent_span_id| {
        // Only a successful main chat round can consume tool results from the
        // preceding round. Background title/summary/judge calls share the
        // Session id but never receive that tool-result history, while a
        // failed Provider attempt did not produce reasoning from it. Counting
        // either would turn genuinely unused results into false positives.
        if event.success && event.operation.as_deref() == Some("chat_round") {
            trial.consumed_tool_results = trial
                .consumed_tool_results
                .saturating_add(trial.pending_tool_results.len() as u64);
            trial.pending_tool_results.clear();
        }
        trial.model_calls = trial.model_calls.saturating_add(1);
        trial.model_active_ms = trial
            .model_active_ms
            .saturating_add(event.duration_ms.unwrap_or(0));
        add_exact_usage(&mut trial.input_tokens, event.input_tokens);
        add_exact_usage(&mut trial.output_tokens, event.output_tokens);
        add_exact_usage(&mut trial.cache_read_tokens, event.cache_read_input_tokens);
        add_exact_usage(
            &mut trial.cache_write_tokens,
            event.cache_creation_input_tokens,
        );
        trial.ttft_ms = match (trial.ttft_ms, event.ttft_ms) {
            (Some(current), Some(value)) => Some(current.min(value)),
            (None, value) | (value, None) => value,
        };
        match configured_eval_cost(event) {
            Some((cost, component)) if trial.cost_usd.is_some() => {
                trial.cost_usd = Some(trial.cost_usd.unwrap_or(0.0) + cost);
                trial.price_components.insert(component);
            }
            Some(_) => {}
            None => trial.cost_usd = None,
        }
        let budget = &trial.context.budget;
        let mut exhaustion_reasons = Vec::new();
        if budget
            .max_input_tokens
            .zip(trial.input_tokens)
            .is_some_and(|(limit, value)| value > limit)
        {
            exhaustion_reasons.push("input_tokens");
        }
        if budget
            .max_output_tokens
            .zip(trial.output_tokens)
            .is_some_and(|(limit, value)| value > limit)
        {
            exhaustion_reasons.push("output_tokens");
        }
        if let Some(limit) = budget.max_cost_micros {
            match trial.cost_usd {
                Some(cost) if usd_to_micros(cost) > limit => exhaustion_reasons.push("cost"),
                None => exhaustion_reasons.push("cost_unknown"),
                _ => {}
            }
        }
        for reason in exhaustion_reasons {
            mark_budget_exhausted(trial, reason, "model", &parent_span_id);
        }
        trial.observed_signals.insert("model".to_string());
        let mut attributes = BTreeMap::new();
        insert_event_text(&mut attributes, "providerId", event.provider_id.as_deref());
        insert_event_text(
            &mut attributes,
            "providerName",
            event.provider_name.as_deref(),
        );
        insert_event_text(&mut attributes, "modelId", event.model_id.as_deref());
        insert_event_text(&mut attributes, "operation", event.operation.as_deref());
        insert_event_u64(&mut attributes, "inputTokens", event.input_tokens);
        insert_event_u64(&mut attributes, "outputTokens", event.output_tokens);
        insert_event_u64(
            &mut attributes,
            "cacheReadTokens",
            event.cache_read_input_tokens,
        );
        insert_event_u64(
            &mut attributes,
            "cacheWriteTokens",
            event.cache_creation_input_tokens,
        );
        insert_event_u64(&mut attributes, "ttftMs", event.ttft_ms);
        attributes.insert("success".to_string(), Value::Bool(event.success));
        if let Some(error) = event.error.as_deref() {
            attributes.insert(
                "errorClass".to_string(),
                Value::String(crate::failover::classify_error(error).as_str().to_string()),
            );
        }
        trial.push_leaf_event_with_attributes(
            "model.call",
            event.request_key.clone(),
            if event.success { "succeeded" } else { "failed" },
            event.duration_ms.unwrap_or(0),
            parent_span_id,
            attributes,
        );
    });
}

pub fn record_tool_result(
    session_id: Option<&str>,
    tool_name: &str,
    call_id: &str,
    arguments_digest: &str,
    outcome: EvalToolOutcome,
    duration_ms: u64,
) {
    record_tool_result_with_digest(
        session_id,
        tool_name,
        call_id,
        arguments_digest,
        None,
        outcome,
        duration_ms,
    );
}

pub fn record_tool_result_with_digest(
    session_id: Option<&str>,
    tool_name: &str,
    call_id: &str,
    arguments_digest: &str,
    result_digest: Option<&str>,
    outcome: EvalToolOutcome,
    duration_ms: u64,
) {
    let Some(session_id) = session_id else {
        return;
    };
    with_trial_for_session(session_id, |trial, parent_span_id| {
        trial.attempted_tools = trial.attempted_tools.saturating_add(1);
        trial.logical_tool_calls.insert(call_id.to_string());
        let fingerprint = format!("{tool_name}\0{arguments_digest}");
        if !trial.tool_call_fingerprints.insert(fingerprint) {
            trial.duplicate_tools = trial.duplicate_tools.saturating_add(1);
        }
        trial.tool_active_ms = trial.tool_active_ms.saturating_add(duration_ms);
        match outcome {
            EvalToolOutcome::Succeeded => {
                trial.succeeded_tools = trial.succeeded_tools.saturating_add(1);
                trial.pending_tool_results.insert(call_id.to_string());
            }
            EvalToolOutcome::Failed => trial.failed_tools = trial.failed_tools.saturating_add(1),
            EvalToolOutcome::Cancelled => {
                trial.cancelled_tools = trial.cancelled_tools.saturating_add(1)
            }
            EvalToolOutcome::ParseError => {
                trial.failed_tools = trial.failed_tools.saturating_add(1);
                trial.parse_errors = trial.parse_errors.saturating_add(1);
            }
        }
        trial.observed_signals.insert("tool".to_string());
        mark_tool_signal(trial, tool_name);
        let mut attributes = BTreeMap::new();
        insert_event_text(&mut attributes, "toolName", Some(tool_name));
        if is_lower_hex(arguments_digest, 64) {
            attributes.insert(
                "argumentsDigest".to_string(),
                Value::String(arguments_digest.to_string()),
            );
        }
        if let Some(result_digest) = result_digest.filter(|value| is_lower_hex(value, 64)) {
            attributes.insert(
                "resultDigest".to_string(),
                Value::String(result_digest.to_string()),
            );
        }
        trial.push_leaf_event_with_attributes(
            format!("tool.{tool_name}"),
            Some(call_id.to_string()),
            match outcome {
                EvalToolOutcome::Succeeded => "succeeded",
                EvalToolOutcome::Failed => "failed",
                EvalToolOutcome::Cancelled => "cancelled",
                EvalToolOutcome::ParseError => "parse_error",
            },
            duration_ms,
            parent_span_id,
            attributes,
        );
    });
}

fn insert_event_text(attributes: &mut BTreeMap<String, Value>, key: &str, value: Option<&str>) {
    let Some(value) = value else {
        return;
    };
    let normalized = value
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n'))
        .take(256)
        .collect::<String>();
    if !normalized.is_empty() {
        attributes.insert(key.to_string(), Value::String(normalized));
    }
}

fn insert_event_u64(attributes: &mut BTreeMap<String, Value>, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        attributes.insert(key.to_string(), Value::from(value));
    }
}

pub fn record_signal(session_id: Option<&str>, signal: &str) {
    let Some(session_id) = session_id else {
        return;
    };
    with_trial_for_session(session_id, |trial, parent_span_id| {
        trial.observed_signals.insert(signal.to_string());
        trial.push_leaf_event(
            format!("signal.{signal}"),
            None,
            "observed",
            0,
            parent_span_id,
        );
    });
}

pub fn record_queue_wait(session_id: Option<&str>, kind: &str, key: &str, duration_ms: u64) {
    let Some(session_id) = session_id else {
        return;
    };
    with_trial_for_session(session_id, |trial, parent_span_id| {
        trial.queue_wait_ms = trial.queue_wait_ms.saturating_add(duration_ms);
        trial.push_leaf_event(
            format!("{kind}.queue_wait"),
            Some(key.to_string()),
            "completed",
            duration_ms,
            parent_span_id,
        );
    });
}

/// Record a retry transition between Provider attempts. Concrete successful
/// and failed requests are counted independently at the Provider-round
/// boundary; this event measures only the retry/failover decision so attempts
/// are never double-counted.
pub fn record_model_retry(session_id: &str, failover: bool, reason: &str, delay_ms: u64) {
    with_trial_for_session(session_id, |trial, parent_span_id| {
        trial.model_retries = trial.model_retries.saturating_add(1);
        if failover {
            trial.failovers = trial.failovers.saturating_add(1);
        }
        trial.push_leaf_event(
            if failover {
                "model.failover"
            } else {
                "model.retry"
            },
            Some(reason.to_string()),
            "completed",
            delay_ms,
            parent_span_id,
        );
    });
}

/// Return a deterministic, registered Provider fault for the next model call.
/// Normal product traffic has no EvalRunContext and pays only the registry
/// lookup. The caller remains responsible for routing the synthetic error
/// through the production failover path.
pub fn provider_fault_action(session_id: Option<&str>) -> Option<EvalFaultAction> {
    take_fault_action(session_id?, FaultTarget::Provider, "model")
}

/// Return a registered Tool/Storage/Environment fault for this concrete tool
/// dispatch. Manifest strings never become commands; matching and effects are
/// fixed in this audited Rust registry.
pub fn tool_fault_action(session_id: Option<&str>, tool_name: &str) -> Option<EvalFaultAction> {
    take_fault_action(session_id?, FaultTarget::Tool, tool_name)
}

/// Deterministic bounded delay used to exercise queue ordering without random
/// sleeps. It is selected from stable trial/fault/work IDs and emitted as a
/// `fault.activated` event before the scheduler starts the work item.
pub fn scheduler_fault_action(session_id: Option<&str>, work_id: &str) -> Option<EvalFaultAction> {
    take_fault_action(session_id?, FaultTarget::Scheduler, work_id)
}

#[derive(Clone, Copy)]
enum FaultTarget {
    Provider,
    Tool,
    Scheduler,
}

fn take_fault_action(
    session_id: &str,
    target: FaultTarget,
    subject: &str,
) -> Option<EvalFaultAction> {
    let mut state = registry().lock().ok()?;
    let (trial_id, parent_span_id) = {
        let session = state.sessions.get(session_id)?;
        (
            session.context.trial_id.clone(),
            session.causal_span_id.clone(),
        )
    };
    let trial = state.trials.get_mut(&trial_id)?;
    let directives = trial.context.faults.clone();
    for fault in directives {
        let current = trial.fault_activations.get(&fault.id).copied().unwrap_or(0);
        if current >= fault.max_activations || !fault_matches(&fault, target, subject) {
            continue;
        }
        trial
            .fault_activations
            .insert(fault.id.clone(), current.saturating_add(1));
        let action = fault_action(&trial.context.trial_id, &fault, target, subject);
        if matches!(target, FaultTarget::Scheduler) {
            trial.queue_wait_ms = trial.queue_wait_ms.saturating_add(action.delay_ms);
        }
        trial.observed_signals.insert("fault".to_string());
        trial.push_leaf_event(
            "fault.activated",
            Some(fault.id.clone()),
            "activated",
            action.delay_ms,
            parent_span_id.clone(),
        );
        return Some(action);
    }
    None
}

fn fault_matches(fault: &EvalFaultDirective, target: FaultTarget, subject: &str) -> bool {
    let tool = subject.to_ascii_lowercase();
    match (fault.kind, target) {
        (EvalFaultKind::ProviderResponse, FaultTarget::Provider) => true,
        (EvalFaultKind::SchedulerOrder, FaultTarget::Scheduler) => true,
        (EvalFaultKind::ToolResponse, FaultTarget::Tool) => match fault.trigger.as_str() {
            "first_read_and_delayed_write_ack" => {
                tool.contains("read") || tool.contains("write") || tool.contains("edit")
            }
            "publish_permanent_failure" => {
                tool.contains("publish") || tool.contains("write") || tool.contains("exec")
            }
            "timeout_429_business_failure" => true,
            "slow_and_untrusted_source" => {
                tool.contains("web") || tool.contains("read") || tool.contains("search")
            }
            "first_hidden_test_failure" | "transient_test_service_failure" => {
                tool.contains("exec") || tool.contains("bash") || tool.contains("test")
            }
            _ => true,
        },
        (EvalFaultKind::StorageState, FaultTarget::Tool) => {
            tool.contains("write")
                || tool.contains("edit")
                || tool.contains("patch")
                || tool.contains("note")
        }
        (EvalFaultKind::EnvironmentEvent, FaultTarget::Tool) => match fault.trigger.as_str() {
            "service_restart_and_browser_disconnect" => {
                tool.contains("browser") || tool.contains("web") || tool.contains("exec")
            }
            "malicious_scope_escalation_fixture" => {
                tool.contains("memory") || tool.contains("knowledge") || tool.contains("note")
            }
            _ => false,
        },
        _ => false,
    }
}

fn fault_action(
    trial_id: &str,
    fault: &EvalFaultDirective,
    target: FaultTarget,
    subject: &str,
) -> EvalFaultAction {
    let configured_delay = fault
        .params
        .get("delayMs")
        .and_then(Value::as_u64)
        .map(|value| value.min(2_000));
    let mut hasher = Sha256::new();
    hasher.update(trial_id.as_bytes());
    hasher.update([0]);
    hasher.update(fault.id.as_bytes());
    hasher.update([0]);
    hasher.update(subject.as_bytes());
    let digest = hasher.finalize();
    let stable_delay = 25 + u64::from(digest[0]) % 176;
    let (delay_ms, error_class) = match (fault.kind, target, fault.trigger.as_str()) {
        (EvalFaultKind::SchedulerOrder, _, _) => (configured_delay.unwrap_or(stable_delay), None),
        (EvalFaultKind::ToolResponse, _, "first_read_and_delayed_write_ack")
        | (EvalFaultKind::ToolResponse, _, "slow_and_untrusted_source") => {
            (configured_delay.unwrap_or(stable_delay), None)
        }
        (EvalFaultKind::ProviderResponse, _, _) => (
            configured_delay.unwrap_or(0),
            Some("injected_provider_response"),
        ),
        (EvalFaultKind::StorageState, _, _) => (0, Some("injected_storage_conflict")),
        (EvalFaultKind::EnvironmentEvent, _, _) => (0, Some("injected_environment_failure")),
        _ => (
            configured_delay.unwrap_or(0),
            Some("injected_tool_response"),
        ),
    };
    EvalFaultAction {
        fault_id: fault.id.clone(),
        delay_ms,
        error_class,
    }
}

fn activate_passive_faults(
    trial: &mut TrialAccumulator,
    kind: EvalFaultKind,
    trigger: Option<&str>,
    parent_span_id: &str,
) {
    let directives = trial.context.faults.clone();
    for fault in directives.into_iter().filter(|fault| {
        fault.kind == kind && trigger.is_none_or(|trigger| fault.trigger == trigger)
    }) {
        let current = trial.fault_activations.get(&fault.id).copied().unwrap_or(0);
        if current >= fault.max_activations {
            continue;
        }
        trial
            .fault_activations
            .insert(fault.id.clone(), current.saturating_add(1));
        trial.observed_signals.insert("fault".to_string());
        trial.push_leaf_event(
            "fault.activated",
            Some(fault.id),
            "activated",
            0,
            parent_span_id.to_string(),
        );
    }
}

/// Record a bounded, payload-free lifecycle transition from a durable Hope
/// control plane. Callers pass only stable IDs and enum-like status strings;
/// prompts, tool arguments, database rows, and error bodies never enter the
/// evaluation registry.
pub fn record_lifecycle_event(
    session_id: Option<&str>,
    signal: &str,
    event: &str,
    key: Option<&str>,
    status: &str,
    duration_ms: u64,
) {
    let Some(session_id) = session_id else {
        return;
    };
    with_trial_for_session(session_id, |trial, parent_span_id| {
        trial.observed_signals.insert(signal.to_string());
        match event {
            "loop.iteration" => {
                trial.loop_iterations = trial.loop_iterations.saturating_add(1);
                if trial.loop_iterations == 3 {
                    activate_passive_faults(
                        trial,
                        EvalFaultKind::EnvironmentEvent,
                        Some("after_loop_iteration_3"),
                        &parent_span_id,
                    );
                }
            }
            "goal.replanned" | "workflow.replanned" => {
                trial.replans = trial.replans.saturating_add(1)
            }
            "workflow.checkpoint" => trial.checkpoints = trial.checkpoints.saturating_add(1),
            "workflow.resumed" | "loop.resumed" => trial.resumes = trial.resumes.saturating_add(1),
            "agent.result_injected" => {
                let injection_key = key.unwrap_or("__missing__").to_string();
                if trial.injection_keys.insert(injection_key) {
                    trial.handoffs = trial.handoffs.saturating_add(1);
                } else {
                    trial.duplicate_injections = trial.duplicate_injections.saturating_add(1);
                }
            }
            _ => {}
        }
        trial.push_leaf_event(
            event.to_string(),
            key.map(str::to_string),
            status.to_string(),
            duration_ms,
            parent_span_id,
        );
    });
}

pub fn telemetry_snapshot(trial_id: &str) -> Option<Value> {
    let state = registry().lock().ok()?;
    let trial = state.trials.get(trial_id)?;
    let required = trial
        .context
        .required_signals
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let missing = required
        .difference(&trial.observed_signals)
        .cloned()
        .collect::<Vec<_>>();
    let trace_closed =
        trial.root_closed && trial.active_sessions.is_empty() && trial.active_work == 0;
    let attribution = if trace_closed && missing.is_empty() {
        "complete"
    } else if trial.model_calls > 0 || trial.attempted_tools > 0 {
        "partial"
    } else {
        "missing"
    };
    let wall_ms = trial
        .started
        .elapsed()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64;
    let mut warnings = trial.warnings.clone();
    if !missing.is_empty() {
        warnings.push(format!("missing required signals: {}", missing.join(", ")));
    }
    if !trace_closed {
        warnings.push("trial trace still has active sessions".to_string());
    }
    let span_count = 1u64.saturating_add(
        trial
            .events
            .iter()
            .map(|event| event.span_id.as_str())
            .collect::<HashSet<_>>()
            .len() as u64,
    );
    let price_snapshot_digest = (!trial.price_components.is_empty()).then(|| {
        let mut hasher = Sha256::new();
        for component in &trial.price_components {
            hasher.update(component.as_bytes());
            hasher.update([0]);
        }
        format!("{:x}", hasher.finalize())
    });
    Some(json!({
        "attribution": attribution,
        "backgroundModelWork": trial.background_model_work,
        "budgetExhausted": trial.budget_exhausted,
        "budgetExhaustionReasons": trial.budget_exhaustion_reasons,
        "timings": {
            "wallMs": wall_ms,
            "environmentSetupMs": 0,
            "environmentCleanupMs": 0,
            "modelActiveMs": trial.model_active_ms,
            "toolActiveMs": trial.tool_active_ms,
            "queueWaitMs": trial.queue_wait_ms,
            "approvalWaitMs": 0,
            "environmentWaitMs": 0,
            "criticalPathMs": wall_ms,
            "ttftMs": trial.ttft_ms,
        },
        "tokens": {
            "input": trial.input_tokens,
            "output": trial.output_tokens,
            "cacheRead": trial.cache_read_tokens,
            "cacheWrite": trial.cache_write_tokens,
            "reasoning": Value::Null,
            "usageSource": "hope_model_usage_events",
        },
        "cost": {
            "totalUsd": trial.cost_usd,
            "agentUsd": trial.cost_usd,
            "simulatorUsd": Value::Null,
            "judgeUsd": Value::Null,
            "priceSnapshotDigest": price_snapshot_digest,
        },
        "tools": {
            "attempted": trial.attempted_tools,
            "logicalCalls": trial.logical_tool_calls.len(),
            "succeeded": trial.succeeded_tools,
            "failed": trial.failed_tools,
            "cancelled": trial.cancelled_tools,
            "retries": trial.attempted_tools.saturating_sub(trial.logical_tool_calls.len() as u64),
            "parseErrors": trial.parse_errors,
            "invalid": trial.parse_errors,
            "duplicate": trial.duplicate_tools,
            "unusedResults": trial.pending_tool_results.len(),
            "effective": trial.consumed_tool_results,
        },
        "orchestration": {
            "modelCalls": trial.model_calls,
            "modelRetries": trial.model_retries,
            "failovers": trial.failovers,
            "loopIterations": trial.loop_iterations,
            "replans": trial.replans,
            "checkpoints": trial.checkpoints,
            "resumes": trial.resumes,
            "spawnedAgents": trial.spawned_agents,
            "maxAgentDepth": trial.max_agent_depth,
            "maxConcurrency": trial.max_concurrency,
            "handoffs": trial.handoffs,
            "coordinationTokens": Value::Null,
            "childActiveMs": trial.child_active_ms,
            "asyncJobs": trial.async_jobs,
            "duplicateInjections": trial.duplicate_injections,
            "orphanedChildren": if trace_closed { 0 } else { trial.active_work },
        },
        "trace": {
            "traceId": trial.context.trace_id,
            "rootSpanId": trial.context.root_span_id,
            "spanCount": span_count,
            "orphanSpanCount": if trace_closed { 0 } else { trial.active_work },
            "closed": trace_closed,
        },
        "milestones": [],
        "invariants": [],
        "warnings": warnings,
        "observedSignals": trial.observed_signals,
        "events": trial.events,
    }))
}

/// Stable Session IDs created under a trial. Used only by the authenticated,
/// eval-mode cleanup endpoint to cancel and delete synthetic state after every
/// attempt; no prompt or user data is returned.
pub fn session_ids_for_trial(trial_id: &str) -> Option<Vec<String>> {
    registry()
        .lock()
        .ok()?
        .trials
        .get(trial_id)
        .map(|trial| trial.all_sessions.iter().cloned().collect())
}

fn configured_eval_cost(event: &ModelUsageEvent) -> Option<(f64, String)> {
    let provider_id = event.provider_id.as_deref()?;
    let model_id = event.model_id.as_deref()?;
    let input_tokens = event.input_tokens?;
    let output_tokens = event.output_tokens?;
    let config = crate::config::cached_config();
    let provider = config
        .providers
        .iter()
        .find(|provider| provider.id == provider_id)?;
    let model = provider.models.iter().find(|model| model.id == model_id)?;
    if model.cost_input.is_none() && model.cost_output.is_none() {
        return None;
    }
    let mut input_price = model.cost_input.unwrap_or(0.0);
    let mut output_price = model.cost_output.unwrap_or(0.0);
    if !input_price.is_finite()
        || !output_price.is_finite()
        || input_price < 0.0
        || output_price < 0.0
    {
        return None;
    }
    if matches!(provider.currency, Some(crate::provider::Currency::Cny)) {
        input_price /= crate::dashboard::CNY_PER_USD;
        output_price /= crate::dashboard::CNY_PER_USD;
    }
    let cost =
        (input_tokens as f64 * input_price + output_tokens as f64 * output_price) / 1_000_000.0;
    let component = format!(
        "usd-per-million-v1\0{provider_id}\0{model_id}\0{:.12}\0{:.12}",
        input_price, output_price
    );
    Some((cost, component))
}

fn with_trial_for_session(session_id: &str, update: impl FnOnce(&mut TrialAccumulator, String)) {
    let Ok(mut state) = registry().lock() else {
        return;
    };
    let Some((trial_id, parent_span_id)) = state
        .sessions
        .get(session_id)
        .map(|entry| (entry.context.trial_id.clone(), entry.causal_span_id.clone()))
    else {
        return;
    };
    if let Some(trial) = state.trials.get_mut(&trial_id) {
        update(trial, parent_span_id);
    }
}

fn add_exact_usage(target: &mut Option<u64>, value: Option<u64>) {
    *target = match (*target, value) {
        (Some(total), Some(value)) => Some(total.saturating_add(value)),
        _ => None,
    };
}

fn mark_tool_signal(trial: &mut TrialAccumulator, tool_name: &str) {
    let normalized = tool_name.to_ascii_lowercase();
    if normalized.contains("goal") {
        trial.observed_signals.insert("goal".to_string());
    }
    if normalized.contains("loop") {
        trial.observed_signals.insert("loop".to_string());
    }
    if normalized.contains("workflow") {
        trial.observed_signals.insert("workflow".to_string());
    }
    if normalized.contains("job") || normalized.contains("wakeup") {
        trial.observed_signals.insert("async_jobs".to_string());
    }
    if normalized.contains("subagent") {
        trial.observed_signals.insert("subagent".to_string());
    }
    if normalized.contains("team") {
        trial.observed_signals.insert("team".to_string());
    }
}

fn prune_trials(state: &mut Registry) {
    let mut inspected = 0;
    while state.trials.len() > MAX_RETAINED_TRIALS && inspected <= state.order.len() {
        let Some(candidate) = state.order.pop_front() else {
            break;
        };
        let active = state
            .trials
            .get(&candidate)
            .is_some_and(|trial| !trial.active_sessions.is_empty());
        if active {
            state.order.push_back(candidate);
            inspected += 1;
        } else {
            state.trials.remove(&candidate);
            state
                .sessions
                .retain(|_, entry| entry.context.trial_id != candidate);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> EvalRunContext {
        EvalRunContext {
            evidence_kind: "model_campaign".to_string(),
            campaign_id: "mcampaign_test".to_string(),
            plan_digest: "1".repeat(64),
            suite_id: "hope-core".to_string(),
            suite_version: "1.0.0".to_string(),
            suite_digest: "2".repeat(64),
            case_id: "HA-TEST-001".to_string(),
            case_digest: "3".repeat(64),
            trial_id: "mtrial_test".to_string(),
            trial_index: 0,
            arm: "control".to_string(),
            fault_profile: "clean".to_string(),
            orchestration_profile: None,
            trace_id: "trace_test".to_string(),
            root_span_id: "span_test".to_string(),
            model_role: "anchor".to_string(),
            seed: 1,
            source: "local_cli".to_string(),
            commit_sha: "4".repeat(40),
            dirty: true,
            app_version: "0.17.0".to_string(),
            required_signals: vec!["model".to_string(), "tool".to_string()],
            faults: Vec::new(),
            budget: EvalBudgetLimits::default(),
        }
    }

    #[test]
    fn snapshot_is_complete_only_after_required_signals_and_close() {
        let guard = register_root_session("session-eval-test", context()).unwrap();
        record_tool_result(
            Some("session-eval-test"),
            "read",
            "call-1",
            "args-a",
            EvalToolOutcome::Succeeded,
            2,
        );
        let event = ModelUsageEvent {
            session_id: Some("session-eval-test".to_string()),
            operation: Some("chat_round".to_string()),
            input_tokens: Some(10),
            output_tokens: Some(5),
            ..ModelUsageEvent::new("chat")
        };
        record_model_usage(&event);
        drop(guard);
        let snapshot = telemetry_snapshot("mtrial_test").unwrap();
        assert_eq!(snapshot["attribution"], "complete");
        assert_eq!(snapshot["tokens"]["input"], 10);
        assert_eq!(snapshot["tools"]["attempted"], 1);
        assert_eq!(snapshot["tools"]["effective"], 1);
        assert_eq!(snapshot["tools"]["unusedResults"], 0);
    }

    #[test]
    fn concurrency_counts_background_work_and_nested_children() {
        let mut context = context();
        context.trial_id = "mtrial_concurrency".to_string();
        context.required_signals.clear();
        let root = register_root_session("session-eval-root", context).unwrap();
        let background_one = retain_session("session-eval-root").unwrap();
        let background_two = retain_session("session-eval-root").unwrap();
        let child_one = inherit_session("session-eval-root", "session-eval-child-1").unwrap();
        let child_two = inherit_session("session-eval-child-1", "session-eval-child-2").unwrap();

        let active = telemetry_snapshot("mtrial_concurrency").unwrap();
        assert_eq!(active["orchestration"]["maxConcurrency"], 5);
        assert_eq!(active["orchestration"]["maxAgentDepth"], 2);
        assert!(!active["trace"]["closed"].as_bool().unwrap());

        drop((child_two, child_one, background_two, background_one, root));
        let closed = telemetry_snapshot("mtrial_concurrency").unwrap();
        assert_eq!(closed["trace"]["orphanSpanCount"], 0);
        assert!(closed["trace"]["closed"].as_bool().unwrap());
    }

    #[test]
    fn product_background_model_work_stays_attributed_until_it_finishes() {
        let mut context = context();
        context.trial_id = "mtrial_background_model".to_string();
        context.required_signals.clear();
        let root = register_root_session("session-background-model", context).unwrap();
        let background = retain_model_automation("session-background-model")
            .unwrap()
            .unwrap();
        let usage = ModelUsageEvent {
            session_id: Some("session-background-model".to_string()),
            operation: Some("session_title".to_string()),
            input_tokens: Some(20),
            output_tokens: Some(4),
            ..ModelUsageEvent::new("automation")
        };
        record_model_usage(&usage);
        drop(root);

        let active = telemetry_snapshot("mtrial_background_model").unwrap();
        assert_eq!(active["backgroundModelWork"], 1);
        assert_eq!(active["orchestration"]["modelCalls"], 1);
        assert_eq!(active["tokens"]["input"], 20);
        assert!(!active["trace"]["closed"].as_bool().unwrap());

        drop(background);
        let closed = telemetry_snapshot("mtrial_background_model").unwrap();
        assert_eq!(closed["backgroundModelWork"], 0);
        assert!(closed["trace"]["closed"].as_bool().unwrap());
    }

    #[test]
    fn trace_preserves_parent_child_spans_and_paired_lifecycles() {
        let mut context = context();
        context.trial_id = "mtrial_causal_trace".to_string();
        context.trace_id = "trace_causal_trace".to_string();
        context.root_span_id = "span_causal_trace".to_string();
        context.required_signals.clear();
        let root = register_root_session("session-causal-root", context).unwrap();
        let background = retain_session("session-causal-root").unwrap();
        let child = inherit_session("session-causal-root", "session-causal-child").unwrap();
        record_tool_result(
            Some("session-causal-child"),
            "read",
            "call-causal",
            "args-causal",
            EvalToolOutcome::Succeeded,
            3,
        );
        drop(child);
        drop(background);
        drop(root);

        let snapshot = telemetry_snapshot("mtrial_causal_trace").unwrap();
        let trace: ha_eval_spec::model::TraceSummary =
            serde_json::from_value(snapshot["trace"].clone()).unwrap();
        let events: Vec<ha_eval_spec::model::ModelCampaignEvent> =
            serde_json::from_value(snapshot["events"].clone()).unwrap();
        ha_eval_spec::model::validate_model_trace(&trace, &events, "causal test").unwrap();

        let root_span = events
            .iter()
            .find(|event| event.event == "session.root" && event.status == "started")
            .unwrap();
        let async_span = events
            .iter()
            .find(|event| event.event == "async_job.run" && event.status == "started")
            .unwrap();
        let child_span = events
            .iter()
            .find(|event| event.event == "subagent.run" && event.status == "started")
            .unwrap();
        let tool_span = events
            .iter()
            .find(|event| event.event == "tool.read")
            .unwrap();
        assert_eq!(async_span.parent_span_id, root_span.span_id);
        assert_eq!(child_span.parent_span_id, root_span.span_id);
        assert_eq!(tool_span.parent_span_id, child_span.span_id);
    }

    #[test]
    fn model_retry_and_failover_are_counted_as_real_attempts() {
        let mut context = context();
        context.trial_id = "mtrial_model_recovery".to_string();
        context.required_signals.clear();
        let root = register_root_session("session-model-recovery", context).unwrap();
        let failed_attempt = |request_key: &str| ModelUsageEvent {
            request_key: Some(request_key.to_string()),
            session_id: Some("session-model-recovery".to_string()),
            success: false,
            ..ModelUsageEvent::new("chat")
        };
        record_model_usage(&failed_attempt("round-1"));
        record_model_retry("session-model-recovery", false, "timeout", 10);
        record_model_usage(&failed_attempt("round-2"));
        record_model_retry("session-model-recovery", true, "rate_limit", 0);
        record_model_usage(&ModelUsageEvent {
            request_key: Some("round-3".to_string()),
            session_id: Some("session-model-recovery".to_string()),
            ..ModelUsageEvent::new("chat")
        });
        drop(root);

        let snapshot = telemetry_snapshot("mtrial_model_recovery").unwrap();
        assert_eq!(snapshot["orchestration"]["modelCalls"], 3);
        assert_eq!(snapshot["orchestration"]["modelRetries"], 2);
        assert_eq!(snapshot["orchestration"]["failovers"], 1);
    }

    #[test]
    fn http_turns_accumulate_until_harness_finishes_trial_root() {
        let mut context = context();
        context.trial_id = "mtrial_http_turns".to_string();
        context.required_signals = vec!["tool".to_string()];
        let first = register_http_turn_session("session-http-turns", context.clone()).unwrap();
        record_tool_result(
            Some("session-http-turns"),
            "read",
            "call-http-1",
            "args-http-1",
            EvalToolOutcome::Succeeded,
            1,
        );
        drop(first);
        let between = telemetry_snapshot("mtrial_http_turns").unwrap();
        assert_eq!(between["tools"]["attempted"], 1);
        assert_eq!(between["trace"]["closed"], false);

        let second = register_http_turn_session("session-http-turns", context).unwrap();
        record_tool_result(
            Some("session-http-turns"),
            "write",
            "call-http-2",
            "args-http-2",
            EvalToolOutcome::Succeeded,
            1,
        );
        drop(second);
        finish_trial_root("mtrial_http_turns").unwrap();
        let finished = telemetry_snapshot("mtrial_http_turns").unwrap();
        assert_eq!(finished["tools"]["attempted"], 2);
        assert_eq!(finished["trace"]["closed"], true);
        assert_eq!(
            finished["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| event["event"] == "session.root.closed")
                .count(),
            1
        );
    }

    #[test]
    fn dormant_http_root_attributes_parent_injection_and_deduplicates_handoff() {
        let mut context = context();
        context.trial_id = "mtrial_parent_injection".to_string();
        context.required_signals.clear();
        let turn = register_http_turn_session("session-parent-injection", context).unwrap();
        drop(turn);

        assert!(context_for_session("session-parent-injection").is_some());
        let injection = retain_session("session-parent-injection").unwrap();
        record_lifecycle_event(
            Some("session-parent-injection"),
            "handoff",
            "agent.result_injected",
            Some("run-handoff"),
            "completed",
            0,
        );
        record_lifecycle_event(
            Some("session-parent-injection"),
            "handoff",
            "agent.result_injected",
            Some("run-handoff"),
            "completed",
            0,
        );
        drop(injection);
        finish_trial_root("mtrial_parent_injection").unwrap();

        let snapshot = telemetry_snapshot("mtrial_parent_injection").unwrap();
        assert_eq!(snapshot["orchestration"]["handoffs"], 1);
        assert_eq!(snapshot["orchestration"]["duplicateInjections"], 1);
        assert_eq!(snapshot["trace"]["closed"], true);
        let events: Vec<ha_eval_spec::model::ModelCampaignEvent> =
            serde_json::from_value(snapshot["events"].clone()).unwrap();
        let trace: ha_eval_spec::model::TraceSummary =
            serde_json::from_value(snapshot["trace"].clone()).unwrap();
        ha_eval_spec::model::validate_model_trace(&trace, &events, "parent injection").unwrap();
    }

    #[test]
    fn tool_duplicates_and_unconsumed_results_are_measured() {
        let mut context = context();
        context.trial_id = "mtrial_tool_quality".to_string();
        context.required_signals.clear();
        let root = register_root_session("session-tool-quality", context).unwrap();
        record_tool_result(
            Some("session-tool-quality"),
            "read",
            "call-quality-1",
            "same-args",
            EvalToolOutcome::Succeeded,
            1,
        );
        record_tool_result(
            Some("session-tool-quality"),
            "read",
            "call-quality-2",
            "same-args",
            EvalToolOutcome::Succeeded,
            1,
        );
        let before_consumption = telemetry_snapshot("mtrial_tool_quality").unwrap();
        assert_eq!(before_consumption["tools"]["duplicate"], 1);
        assert_eq!(before_consumption["tools"]["unusedResults"], 2);
        assert_eq!(before_consumption["tools"]["effective"], 0);

        record_model_usage(&ModelUsageEvent {
            session_id: Some("session-tool-quality".to_string()),
            operation: Some("chat_round".to_string()),
            ..ModelUsageEvent::new("chat")
        });
        drop(root);
        let consumed = telemetry_snapshot("mtrial_tool_quality").unwrap();
        assert_eq!(consumed["tools"]["unusedResults"], 0);
        assert_eq!(consumed["tools"]["effective"], 2);
    }

    #[test]
    fn background_and_failed_model_calls_do_not_consume_tool_results() {
        let mut context = context();
        context.trial_id = "mtrial_tool_consumption_boundary".to_string();
        context.required_signals.clear();
        let root = register_root_session("session-tool-consumption", context).unwrap();
        record_tool_result(
            Some("session-tool-consumption"),
            "read",
            "call-consumption-1",
            "args-consumption",
            EvalToolOutcome::Succeeded,
            1,
        );

        record_model_usage(&ModelUsageEvent {
            session_id: Some("session-tool-consumption".to_string()),
            operation: Some("session_title".to_string()),
            ..ModelUsageEvent::new("side_query")
        });
        record_model_usage(&ModelUsageEvent {
            session_id: Some("session-tool-consumption".to_string()),
            operation: Some("chat_round".to_string()),
            success: false,
            ..ModelUsageEvent::new("chat")
        });
        let pending = telemetry_snapshot("mtrial_tool_consumption_boundary").unwrap();
        assert_eq!(pending["tools"]["unusedResults"], 1);
        assert_eq!(pending["tools"]["effective"], 0);

        record_model_usage(&ModelUsageEvent {
            session_id: Some("session-tool-consumption".to_string()),
            operation: Some("chat_round".to_string()),
            ..ModelUsageEvent::new("chat")
        });
        drop(root);
        let consumed = telemetry_snapshot("mtrial_tool_consumption_boundary").unwrap();
        assert_eq!(consumed["tools"]["unusedResults"], 0);
        assert_eq!(consumed["tools"]["effective"], 1);
    }

    #[test]
    fn trace_attributes_capture_metrics_and_digests_without_payloads() {
        let mut context = context();
        context.trial_id = "mtrial_redacted_attributes".to_string();
        context.required_signals.clear();
        let root = register_root_session("session-redacted-attributes", context).unwrap();
        let arguments_digest = "a".repeat(64);
        let result_digest = "b".repeat(64);
        record_tool_result_with_digest(
            Some("session-redacted-attributes"),
            "read",
            "call-redacted",
            &arguments_digest,
            Some(&result_digest),
            EvalToolOutcome::Succeeded,
            7,
        );
        record_model_usage(&ModelUsageEvent {
            request_key: Some("round-redacted".to_string()),
            session_id: Some("session-redacted-attributes".to_string()),
            operation: Some("chat_round".to_string()),
            provider_id: Some("provider-test".to_string()),
            provider_name: Some("Provider\nName".to_string()),
            model_id: Some("model-test".to_string()),
            input_tokens: Some(17),
            output_tokens: Some(9),
            ttft_ms: Some(3),
            duration_ms: Some(11),
            ..ModelUsageEvent::new("chat")
        });
        drop(root);

        let snapshot = telemetry_snapshot("mtrial_redacted_attributes").unwrap();
        let events: Vec<ha_eval_spec::model::ModelCampaignEvent> =
            serde_json::from_value(snapshot["events"].clone()).unwrap();
        let tool = events
            .iter()
            .find(|event| event.event == "tool.read")
            .unwrap();
        assert_eq!(tool.attributes["argumentsDigest"], arguments_digest);
        assert_eq!(tool.attributes["resultDigest"], result_digest);
        assert_eq!(tool.attributes["toolName"], "read");
        let model = events
            .iter()
            .find(|event| event.event == "model.call")
            .unwrap();
        assert_eq!(model.attributes["providerId"], "provider-test");
        assert_eq!(model.attributes["providerName"], "ProviderName");
        assert_eq!(model.attributes["modelId"], "model-test");
        assert_eq!(model.attributes["inputTokens"], 17);
        assert_eq!(model.attributes["outputTokens"], 9);
        assert_eq!(model.attributes["ttftMs"], 3);
        let encoded = serde_json::to_string(&events).unwrap();
        assert!(!encoded.contains("prompt"));
        assert!(!encoded.contains("tool result payload"));
    }

    #[test]
    fn runtime_budgets_block_new_model_tool_and_agent_work() {
        let mut context = context();
        context.trial_id = "mtrial_runtime_budget".to_string();
        context.required_signals.clear();
        context.budget = EvalBudgetLimits {
            max_model_calls: Some(1),
            max_output_tokens: Some(100),
            max_tool_calls: Some(1),
            max_agents: Some(1),
            max_concurrency: Some(4),
            ..Default::default()
        };
        let root = register_root_session("session-runtime-budget", context.clone()).unwrap();

        ensure_model_budget(Some("session-runtime-budget")).unwrap();
        record_model_usage(&ModelUsageEvent {
            session_id: Some("session-runtime-budget".to_string()),
            operation: Some("chat_round".to_string()),
            output_tokens: Some(10),
            ..ModelUsageEvent::new("chat")
        });
        assert!(ensure_model_budget(Some("session-runtime-budget")).is_err());

        ensure_tool_budget(Some("session-runtime-budget")).unwrap();
        record_tool_result(
            Some("session-runtime-budget"),
            "read",
            "call-budget",
            "args-budget",
            EvalToolOutcome::Succeeded,
            1,
        );
        assert!(ensure_tool_budget(Some("session-runtime-budget")).is_err());

        let child = register_child_session_from_parent(
            "session-runtime-budget",
            "session-runtime-budget-child",
            context.clone(),
        )
        .unwrap();
        assert!(register_child_session_from_parent(
            "session-runtime-budget",
            "session-runtime-budget-child-two",
            context,
        )
        .is_err());
        drop(child);
        drop(root);

        let snapshot = telemetry_snapshot("mtrial_runtime_budget").unwrap();
        assert_eq!(snapshot["budgetExhausted"], true);
        let reasons = snapshot["budgetExhaustionReasons"].as_array().unwrap();
        assert!(reasons.iter().any(|value| value == "model_calls"));
        assert!(reasons.iter().any(|value| value == "tool_calls"));
        assert!(reasons.iter().any(|value| value == "agents"));
        assert_eq!(
            snapshot["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| event["event"] == "budget.exhausted")
                .count(),
            3
        );
    }

    #[test]
    fn zero_remaining_budget_is_valid_and_blocks_continuation_immediately() {
        let mut context = context();
        context.trial_id = "mtrial_zero_remaining_budget".to_string();
        context.required_signals.clear();
        context.budget = EvalBudgetLimits {
            max_model_calls: Some(0),
            max_tool_calls: Some(0),
            max_agents: Some(0),
            max_concurrency: Some(0),
            ..Default::default()
        };
        context.validate().unwrap();
        let root = register_root_session("session-zero-remaining-budget", context.clone()).unwrap();
        assert!(ensure_model_budget(Some("session-zero-remaining-budget")).is_err());
        assert!(ensure_tool_budget(Some("session-zero-remaining-budget")).is_err());
        assert!(register_child_session_from_parent(
            "session-zero-remaining-budget",
            "session-zero-remaining-budget-child",
            context,
        )
        .is_err());
        drop(root);
    }

    #[test]
    fn registered_fault_activates_once_and_is_auditable() {
        let mut context = context();
        context.trial_id = "mtrial_fault_once".to_string();
        context.required_signals = vec!["fault".to_string()];
        context.faults = vec![EvalFaultDirective {
            id: "fault_read_once".into(),
            kind: EvalFaultKind::ToolResponse,
            trigger: "first_hidden_test_failure".into(),
            params: BTreeMap::new(),
            max_activations: 1,
        }];
        let guard = register_root_session("session-fault-once", context).unwrap();
        assert!(tool_fault_action(Some("session-fault-once"), "exec").is_some());
        assert!(tool_fault_action(Some("session-fault-once"), "exec").is_none());
        drop(guard);

        let snapshot = telemetry_snapshot("mtrial_fault_once").unwrap();
        assert_eq!(snapshot["attribution"], "complete");
        assert_eq!(
            snapshot["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|event| event["event"] == "fault.activated")
                .count(),
            1
        );
    }
}
