//! [`ToolExecContext`] 与其卫星类型——kernel 调用方（agent 循环 /
//! async_jobs / workflow / slash_commands）与工具 adapter 之间的执行
//! 上下文契约。
//!
//! 只承载「上下文数据 + 与分发无关的纯方法」；耦合分发注册表的可见性
//! 裁决（`builtin_fate_error` / `tool_visibility_error` 等）留在
//! `tools::execution` 的第二个 `impl ToolExecContext` 块里，随分发层走。

use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex as AsyncMutex, Notify};
use tokio_util::sync::CancellationToken;

use crate::agent_config::AsyncToolPolicy;

/// Whether the model turn that owns a tool call represents live user intent.
///
/// This is deliberately separate from the knowledge-base access source: ACP
/// and autonomous parent injections both map to the conservative KB `Other`
/// bucket, but only ACP is a foreground user turn. Capability-like tools such
/// as `session_continue` must fail closed when provenance was not threaded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ToolTurnProvenance {
    /// A live user initiated this turn through GUI, HTTP, IM, or ACP.
    ForegroundUser,
    /// The turn was initiated by cron, a subagent, or a parent injection.
    Autonomous,
    /// A legacy or non-chat caller did not bind turn provenance.
    #[default]
    Unknown,
}

#[doc(hidden)]
pub struct EffectiveArgsUpdate {
    pub value: Value,
    pub acknowledged: oneshot::Sender<std::result::Result<(), String>>,
}

/// Per-dispatch rendezvous used to stop a tool between `PreToolUse` argument
/// rewriting and the first permission/side-effecting operation. The streaming
/// orchestrator journals the effective arguments, crosses a durability barrier,
/// and only then acknowledges the update so dispatch may continue.
#[derive(Default)]
#[doc(hidden)]
pub struct EffectiveArgsSink {
    pending: AsyncMutex<Option<EffectiveArgsUpdate>>,
    changed: Notify,
}

impl std::fmt::Debug for EffectiveArgsSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EffectiveArgsSink").finish_non_exhaustive()
    }
}

impl EffectiveArgsSink {
    async fn publish_and_wait(&self, value: Value) -> anyhow::Result<()> {
        let (acknowledged, wait) = oneshot::channel();
        *self.pending.lock().await = Some(EffectiveArgsUpdate {
            value,
            acknowledged,
        });
        self.changed.notify_one();
        match wait.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => anyhow::bail!(error),
            Err(_) => anyhow::bail!("effective tool arguments were not acknowledged"),
        }
    }

    #[doc(hidden)]
    pub async fn next(&self) -> EffectiveArgsUpdate {
        loop {
            let changed = self.changed.notified();
            if let Some(update) = self.pending.lock().await.take() {
                return update;
            }
            changed.await;
        }
    }
}

/// How a backgrounded tool call got authorized to run — the persistent audit
/// counterpart to `ApprovalResolutionSource` (transient broadcast), sharing
/// the same snake_case word table. Stored in the async-job `approval_origin`
/// column so audits can tell a real human grant apart from a weaker
/// timeout-proceed (TIMEOUT-2). Written by the exec async approval-reorder; the
/// sync exec path / other origins are wired by later subtasks (F6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalOrigin {
    /// User clicked Approve (once or always), or a prior AllowAlways prefix
    /// matched — a real human grant.
    User,
    /// Approval dialog timed out and `approval_timeout_action=proceed` — a
    /// weaker authorization than an explicit click.
    TimeoutProceed,
    /// An unattended surface (cron / headless-no-client / ACP-no-capability /
    /// subagent-no-parent-surface) auto-proceeded because
    /// `unattendedApprovalAction=proceed` — a weaker, non-human authorization,
    /// recorded distinctly from a real `User` grant. A strict reason can never
    /// reach here (it is force-denied). Epic D / F (TIMEOUT-1).
    UnattendedProceed,
    /// A YOLO session or global dangerous-skip bypassed the gate.
    Yolo,
    /// IM auto-approve account / slash-skill execution skipped all gates.
    AutoApprove,
    /// Async-job re-entry pre-approved at the outer engine gate.
    ExternalPreApproved,
    /// The permission engine allowed the command without prompting (safe for
    /// the current session preset, not via YOLO).
    PolicyAllow,
}

impl ApprovalOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::TimeoutProceed => "timeout_proceed",
            Self::UnattendedProceed => "unattended_proceed",
            Self::Yolo => "yolo",
            Self::AutoApprove => "auto_approve",
            Self::ExternalPreApproved => "external_pre_approved",
            Self::PolicyAllow => "policy_allow",
        }
    }
}

// ── Tool Execution Context ────────────────────────────────────────

/// Optional bound session database for non-global agent/runtime paths. A
/// newtype with a hand-written `Debug` keeps SQLite connection internals out of
/// logs while letting `ToolExecContext` remain debuggable.
#[derive(Clone)]
pub struct SessionDbHandle(pub Arc<crate::session::SessionDB>);

impl std::fmt::Debug for SessionDbHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionDbHandle(..)")
    }
}

/// Context passed to tool execution for dynamic behavior.
///
/// # Concurrency contract
///
/// The tool loop runs concurrent-safe tools in parallel via `join_all`,
/// `clone()`-ing this struct once per concurrent task (see
/// `crates/ha-agent-runtime/src/provider_adapters/` and its round driver,
/// look for `let tool_ctx = tool_ctx.clone();`). Most fields are value types or
/// owned `Vec`s, so the clone is independent and a tool only ever observes its
/// own snapshot. `ContextResourceRef` deliberately carries a turn-owned ledger
/// Arc alongside its immutable byte Arc: profile rebuilds must share cumulative
/// continuation accounting, while dropping the turn releases both.
///
/// **Do not** add `Mutex`/`RwLock` directly to this struct without defining its
/// owner and reset boundary. Process-global coordination belongs in a
/// process-global `OnceLock<TokioMutex<...>>` (see
/// [`crate::tools::pending_approvals_per_session`] for the canonical pattern).
#[derive(Debug, Clone, Default)]
pub struct ToolExecContext {
    /// Model context window in tokens (for dynamic output truncation)
    pub context_window_tokens: Option<u32>,
    /// Estimated tokens currently used by system prompt + messages + max_output.
    /// Used by the read tool to compute remaining context budget for adaptive sizing.
    pub used_tokens: Option<u32>,
    /// Agent home directory — per-agent scratch/home directory.
    pub home_dir: Option<String>,
    /// User-selected working directory for the current session.
    /// Path-aware tools prefer this over the agent home when no explicit
    /// absolute path/cwd is provided.
    pub session_working_dir: Option<String>,
    /// The parent Project's effective primary root when a session-level
    /// working directory overrides it. Relative paths still resolve from the
    /// session root, but absolute file operations may target this root.
    pub project_primary_dir: Option<String>,
    /// Supplementary absolute roots inherited from the current project. They
    /// never affect relative-path or cwd resolution, but path-aware tools may
    /// accept explicit absolute paths beneath them.
    pub project_linked_dirs: Vec<String>,
    /// Current session ID (for sub-agent spawning context)
    pub session_id: Option<String>,
    /// Durable chat-turn identity for turn-scoped opaque references.
    pub turn_id: Option<String>,
    /// Opaque Agent references resolved from typed composer gestures for this
    /// turn. These are selectors only, never authorization or spawn commands.
    pub agent_binding_refs: Vec<crate::prompt_context::AgentBindingRef>,
    /// Exact frozen `@file` / `@plan` resources addressable only through their opaque,
    /// turn-scoped handles. This enables pagination without reopening the
    /// mutable source path, including for incognito turns.
    pub context_resource_refs: Vec<crate::prompt_context::ContextResourceRef>,
    /// Durable Workflow owner identity for tools invoked by the Workflow host.
    ///
    /// This is an execution-context capability, not a model argument. Internal
    /// Workflow fields such as `__hope_workflow_run_id` must match it before a
    /// tool may control Workflow-owned resources.
    pub workflow_run_id: Option<String>,
    /// Session DB bound to this agent/runtime path. When absent, tools fall
    /// back to the process-global session DB for legacy callers.
    pub session_db: Option<SessionDbHandle>,
    /// Provider tool-call id for the currently executing tool. Async jobs
    /// persist this so completion notifications can point back to the exact
    /// original call.
    pub tool_call_id: Option<String>,
    /// Current agent ID
    pub agent_id: Option<String>,
    /// Sub-agent nesting depth (0 = top-level)
    pub subagent_depth: u32,
    /// Agent-level non-Core tool switch overrides from `agent.json`
    /// `capabilities.tools`.
    pub agent_tool_filter: crate::agent_config::FilterConfig,
    /// Tools removed by sub-agent depth policy or other schema-level denies.
    pub denied_tools: Vec<String>,
    /// Active skill-level tool whitelist. When non-empty, only these tools are allowed.
    pub skill_allowed_tools: Vec<String>,
    /// Whether the agent forces Docker sandbox mode for all exec commands.
    pub force_sandbox: bool,
    /// Per-session sandbox mode. `force_sandbox` is retained as a compatibility
    /// bit for legacy contexts; when it is true and this field is `Off`, callers
    /// should treat it as `Standard`.
    pub sandbox_mode: crate::permission::SandboxMode,
    /// Plan mode file-pattern allow rules: when set, write/edit tools targeting these
    /// glob patterns are allowed even if the tool is in the denied list.
    /// Format: list of glob patterns (e.g. ["~/.hope-agent/plans/*.md"])
    pub plan_mode_allow_paths: Vec<String>,
    /// Plan mode tool whitelist: when non-empty, only these tools can execute.
    /// Enforced at execution layer as defense-in-depth (supplements schema-level filtering).
    pub plan_mode_allowed_tools: Vec<String>,
    /// Plan mode tools that are whitelisted but still need explicit per-call
    /// approval (`ask_tools` from the plan agent config). Defaults to `exec`
    /// for the bundled plan agent so a planning subagent can't run shell
    /// commands without confirmation.
    pub plan_mode_ask_tools: Vec<String>,
    /// When true, automatically approve all tool calls — skips BOTH the
    /// permission-engine gate AND the `exec` command-level gate. Set by the
    /// IM channel auto-approve account flag and by skill-triggered slash
    /// commands (the user has out-of-band authorized everything that path
    /// will run). **Do not** set this for internal re-entries that only mean
    /// "the engine already ran at the outer dispatch" — use
    /// [`Self::external_pre_approved`] instead, otherwise `exec` will
    /// silently bypass its dangerous/edit-command audits.
    pub auto_approve_tools: bool,
    /// Set by the async-job spawner / auto-bg helper to mark that the
    /// permission engine gate (see `needs_permission_engine`) was already
    /// satisfied at the outer dispatch. Inner re-entries skip the engine
    /// gate but **still run command-level gates** (notably `exec`'s
    /// dangerous/edit-command + AllowAlways audit), because for the `exec`
    /// tool those gates are intentionally bypassed at the outer engine layer
    /// (`needs_permission_engine` excludes `TOOL_EXEC`) and `exec` is
    /// expected to run them itself.
    ///
    /// Differs from [`Self::auto_approve_tools`], which means "skip ALL
    /// approval gates including command-level" and is reserved for explicit
    /// owner-controlled unattended surfaces such as an IM auto-approve
    /// account. Merely selecting or invoking a Skill never sets it.
    pub external_pre_approved: bool,
    /// Set ONLY by the async approval-reorder path
    /// (`execute_tool_with_context`) after it has already run `exec`'s
    /// command-level gate (`exec::resolve_exec_command_approval`) and the
    /// user approved — *before* detaching the call into a background job. The
    /// spawned re-dispatch reads this via [`Self::should_run_exec_command_gate`]
    /// to skip the inner gate, so the command is approved exactly once and the
    /// model never sees a synthetic "started" job id ahead of the prompt
    /// (ASYNC-1 / HOOKS-2).
    ///
    /// Physically separate from [`Self::external_pre_approved`], which silences
    /// only the *engine* gate and must NEVER suppress the command-level audit.
    /// This flag may suppress the command gate precisely because it is set only
    /// once that gate has already passed for this exact call.
    pub exec_pre_approved: bool,
    /// How a backgrounded call was authorized, for the async-job
    /// `approval_origin` audit column (TIMEOUT-2). Set by the exec async
    /// approval-reorder alongside [`Self::exec_pre_approved`] and read by
    /// [`crate::async_jobs::spawn::record_running_job`]. `None` for synchronous
    /// dispatch and for jobs that skipped the gate (auto-approve / external
    /// pre-approved — wired separately by F6).
    pub approval_origin: Option<ApprovalOrigin>,
    /// Per-session permission mode (Default / Smart / Yolo). Resolved from the
    /// `sessions.permission_mode` column at agent build time. The engine
    /// consumes this together with `global_yolo` to decide approval behavior.
    pub session_mode: crate::permission::SessionMode,
    /// Agent-level "custom tool approval" toggle from `agent.json`.
    /// When false, `agent_custom_approval_tools` is ignored.
    pub agent_custom_approval_enabled: bool,
    /// Agent-level extra approval list. Only consumed in Default mode.
    pub agent_custom_approval_tools: Vec<String>,
    /// Project id (if any) for AllowAlways scope resolution.
    pub project_id: Option<String>,
    /// Turn source for knowledge-base access scoping (design D10). `None` =
    /// unknown (treated as owner/GUI). Set by the chat engine; IM turns set
    /// `Im` so KB access is denied even on project-attached sessions (Phase 1).
    pub chat_source: Option<crate::knowledge::KbAccessSource>,
    /// Call-chain origin for KB access scoping (design D10). `None` = same as
    /// `chat_source` (top-level turn). A subagent carries its parent turn's
    /// origin so an IM-origin chain can't reacquire KB access through the
    /// neutral `Subagent` source. Consumed by `effective_kb_access`.
    pub origin_chat_source: Option<crate::knowledge::KbAccessSource>,
    /// Execution provenance for tools that require fresh, live user intent.
    /// Defaults to [`ToolTurnProvenance::Unknown`] so unthreaded callers cannot
    /// acquire such capabilities accidentally.
    pub turn_provenance: ToolTurnProvenance,
    /// Durable Stop generation observed when this model turn was admitted.
    /// `session_continue` compares it with the live lineage generation so an
    /// older foreground turn cannot undo a newer user Stop.
    pub turn_admitted_stop_epoch: Option<u64>,
    /// Session-free global Stop generation observed at admission.
    pub turn_admitted_global_stop_epoch: Option<u64>,
    /// Receipts from this or earlier global generations visible in the lineage.
    pub turn_admitted_global_stop_receipt_count: Option<u64>,
    /// IM identity of the lineage origin, for the WS8 KB-access opt-in gate.
    /// `Some` only when the lineage contains an IM hop (top-level IM turn or an
    /// IM-origin subagent, which carries the origin's identity). `None` for
    /// GUI/HTTP/cron. Consumed by `effective_kb_access` via `KnowledgeAccessContext`.
    pub channel_kb_context: Option<crate::knowledge::ChannelKbContext>,
    /// Per-agent async tool backgrounding policy (mirrors AgentConfig.capabilities.async_tool_policy).
    pub async_tool_policy: AsyncToolPolicy,
    /// Optional caller-preallocated async job id. Durable parent runtimes set
    /// this before dispatching an explicit `run_in_background` tool so they can
    /// persist the child handle before the side effect starts. Ignored unless
    /// this dispatch actually takes the immediate async-job path.
    pub async_job_id_override: Option<String>,
    /// Internal flag set by the async-job spawner when re-dispatching an
    /// async-capable tool inside a background runtime. Prevents infinite
    /// recursion: even if the tool is async-capable and the policy is
    /// `always-background`, this single re-dispatch runs synchronously.
    pub bypass_async_dispatch: bool,
    /// Internal flag set for async tool jobs that already have their own
    /// background runtime cap (`asyncTools.maxJobSecs`). This prevents the
    /// global foreground safety net (`toolTimeout`) from shortening long
    /// background work unexpectedly.
    pub suppress_global_tool_timeout: bool,
    /// Internal flag for async tool jobs. They persist the final result through
    /// `async_jobs::spawn::persist_result`, so the generic result layer must
    /// not wrap the output first, materialize image markers, or turn the async
    /// output-file into a pointer to a second file.
    pub suppress_result_disk_persistence: bool,
    /// Internal flag for workflow-owned async jobs whose result is surfaced by
    /// their parent workflow UI instead of by a chat `<task-notification>`.
    /// Terminal state, hooks, events, and Background Jobs rows still update; the
    /// row is simply marked injected so replay does not synthesize a chat turn.
    pub suppress_completion_injection: bool,
    /// Whether the owning session is incognito (`sessions.incognito`). Resolved
    /// once per ctx build from the session row. Incognito sessions must leave no
    /// disk trace, so this gates large-tool-result spooling
    /// (`maybe_persist_large_tool_result`) and async-job persistence
    /// ([`crate::async_jobs::spawn::record_running_job`] /
    /// `persist_result`), and forces AllowAlways grants to in-memory session
    /// scope ([`Self::allowlist_grant_context`]). Epic E (INCOG-2/5/6).
    pub incognito: bool,
    /// Best-effort cancellation signal for the currently executing tool.
    /// The chat turn, async-job timeout, or runtime_cancel path can trip this
    /// token; resource-owning tools such as `exec` use it to clean up process
    /// trees instead of merely returning a cancelled tool result.
    pub cancellation_token: Option<CancellationToken>,
    /// Per-dispatch sink for structured tool metadata (e.g. file change
    /// before/after snapshots, line deltas). The orchestrator constructs a
    /// fresh `Arc<Mutex<None>>` for **each** tool dispatch, attaches the same
    /// `Arc` clone to the `ToolExecContext` clone passed into the tool, and
    /// drains the value after the tool returns. Tools call
    /// [`ToolExecContext::emit_metadata`] to push their JSON.
    ///
    /// Why an `Arc<Mutex<...>>` despite the "no Mutex on this struct" rule
    /// above: the rule prevents *cross-dispatch* sharing where each `clone()`
    /// would silently get its own lock. Here every dispatch independently
    /// constructs a single sink and shares it only with the helpers it spawns
    /// for that dispatch — exactly the pattern the rule allows.
    pub metadata_sink: Option<Arc<AsyncMutex<Option<Value>>>>,
    /// Per-dispatch handshake for *effective* tool arguments. When
    /// `PreToolUse` (or exec migration) rewrites input, dispatch pauses here
    /// until the streaming orchestrator has journaled the rewrite and crossed
    /// a durability barrier. `None` keeps non-chat/direct callers unchanged.
    #[doc(hidden)]
    pub effective_args_sink: Option<Arc<EffectiveArgsSink>>,
    /// Callback to record the OS pid of a tool's spawned child process (e.g.
    /// `exec`'s shell child) into the owning async-job row, so a crash/restart
    /// can detect and terminate orphaned process trees (I3). Set by
    /// [`crate::async_jobs::spawn::spawn_explicit_job`] for backgrounded jobs;
    /// `None` for foreground dispatch (no job row to annotate). Invoked via
    /// [`Self::emit_pid`].
    pub pid_sink: Option<PidSink>,
    /// Job id whose running output should be teed into a bounded tail buffer
    /// (`async_jobs::output_tail`, R3 ①) so `job_status` can show a *running*
    /// job's latest output. Set by
    /// [`crate::async_jobs::spawn::spawn_explicit_job`] for backgrounded,
    /// non-incognito jobs only; `None` for foreground dispatch (which returns
    /// its full output immediately, so there is no running window to tail) and
    /// for incognito jobs (close-and-burn — no tail buffer).
    pub output_tail_job_id: Option<String>,
}

/// Wrapper around the [`ToolExecContext::pid_sink`] callback. A newtype with a
/// hand-written `Debug` because `ToolExecContext` derives `Debug` and a bare
/// `Arc<dyn Fn>` is not `Debug`.
#[derive(Clone)]
pub struct PidSink(pub Arc<dyn Fn(u32) + Send + Sync>);

impl std::fmt::Debug for PidSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PidSink(..)")
    }
}

impl ToolExecContext {
    /// True when either local gate-skip flag is set (`auto_approve_tools`
    /// from IM auto-approve accounts / slash-skill execution, or
    /// `external_pre_approved` from async-job re-entry). Callers that need
    /// the full effective verdict still need to OR in
    /// `mcp_tool_auto_approves(name).await` — that one is async-only and
    /// can't fold into a sync method.
    #[inline]
    pub fn local_auto_approve(&self) -> bool {
        self.auto_approve_tools || self.external_pre_approved
    }

    /// True when `exec` must run its command-level audit (dangerous-commands
    /// + edit-commands + AllowAlways prefix). Two flags bypass it:
    ///   - `auto_approve_tools` — "skip ALL approval" (IM auto-approve /
    ///     slash-skill execution); and
    ///   - `exec_pre_approved` — the async approval-reorder already ran this
    ///     exact gate and the user approved, before detaching.
    ///
    /// `external_pre_approved` deliberately does NOT bypass it: it silences
    /// only the engine gate (which excludes `TOOL_EXEC` anyway), and this audit
    /// is `exec`'s only safeguard against dangerous patterns when the call is
    /// re-dispatched through the async-job spawner / auto-bg helper.
    ///
    /// Changing this read site without also updating the
    /// [`Self::auto_approve_tools`] / [`Self::external_pre_approved`] /
    /// [`Self::exec_pre_approved`] docs is a security regression.
    #[inline]
    pub fn should_run_exec_command_gate(&self) -> bool {
        !self.auto_approve_tools && !self.exec_pre_approved
    }

    /// Returns the default path for path-aware tools: session working dir,
    /// then agent home, then ".".
    pub fn default_path(&self) -> &str {
        self.session_working_dir
            .as_deref()
            .or(self.home_dir.as_deref())
            .unwrap_or(".")
    }

    /// Canonical project roots available to file tools for this turn. The
    /// session working directory is first, an overridden Project primary is
    /// next when present, and persisted linked roots follow.
    pub fn project_file_roots(&self) -> impl Iterator<Item = &str> {
        self.session_working_dir
            .as_deref()
            .into_iter()
            .chain(self.project_primary_dir.as_deref())
            .chain(self.project_linked_dirs.iter().map(String::as_str))
    }

    pub fn allowlist_grant_context(&self) -> crate::permission::allowlist::GrantContext<'_> {
        crate::permission::allowlist::GrantContext {
            session_id: self.session_id.as_deref(),
            project_id: self.project_id.as_deref(),
            agent_id: self.agent_id.as_deref(),
            default_path: Some(self.default_path()),
            home_dir: self.home_dir.as_deref(),
            incognito: self.incognito,
        }
    }

    /// Returns the default cwd for process tools: session working dir, then
    /// agent home, then the user's home directory, then ".".
    pub fn default_cwd(&self) -> String {
        self.session_working_dir
            .clone()
            .or_else(|| self.home_dir.clone())
            .or_else(|| dirs::home_dir().map(|p| p.to_string_lossy().to_string()))
            .unwrap_or_else(|| ".".to_string())
    }

    /// Build the shared hook-input fields for a tool-event hook (design §5.4).
    ///
    /// `permission_mode` reflects the live posture so a policy hook can see the
    /// most dangerous state: global dangerous-skip or a YOLO session →
    /// `BypassPermissions`; a non-empty plan allow-list → `Plan`; Smart →
    /// `Other`; else `Default`. The ctx lacks the full `PlanModeState`, so plan
    /// detection is allow-list-based.
    pub fn common_hook_input(&self, event: &str) -> crate::hooks::CommonHookInput {
        let session_id = self.session_id.clone().unwrap_or_default();
        // Empty session_id → no transcript path, rather than a bogus shared
        // `sessions/transcript.jsonl` (mirrors hooks::observation_common).
        let transcript_path = if session_id.is_empty() {
            std::path::PathBuf::default()
        } else {
            crate::paths::session_dir(&session_id)
                .map(|d| d.join("transcript.jsonl"))
                .unwrap_or_default()
        };
        let permission_mode = if crate::security::dangerous::is_dangerous_skip_active()
            || matches!(self.session_mode, crate::permission::SessionMode::Yolo)
        {
            crate::hooks::PermissionMode::BypassPermissions
        } else if !self.plan_mode_allowed_tools.is_empty() {
            crate::hooks::PermissionMode::Plan
        } else if matches!(self.session_mode, crate::permission::SessionMode::Smart) {
            crate::hooks::PermissionMode::Other
        } else {
            crate::hooks::PermissionMode::Default
        };
        crate::hooks::CommonHookInput {
            prompt_id: crate::hooks::resolve_prompt_id(&session_id),
            session_id,
            transcript_path,
            cwd: std::path::PathBuf::from(self.default_cwd()),
            permission_mode,
            effort: crate::hooks::resolve_effort(),
            hook_event_name: event.to_string(),
            agent_id: self.agent_id.clone(),
            // `agent_type` is the agent's *type/role*, which the exec context
            // doesn't carry — leave it unset rather than duplicating agent_id.
            // (A real subagent-type field lands with the subagent hook phase.)
            agent_type: None,
        }
    }

    /// Resolve a user/model supplied file path against the current tool
    /// default. Absolute paths and `~` stay anchored where the caller asked;
    /// relative paths are rooted at the session working dir when one exists.
    pub fn resolve_path(&self, raw_path: &str) -> String {
        let expanded = super::expand_tilde(raw_path);
        let path = std::path::Path::new(&expanded);
        if path.is_absolute() {
            return expanded;
        }
        std::path::Path::new(self.default_path())
            .join(path)
            .to_string_lossy()
            .to_string()
    }

    /// Whether the tool is visible under the current combined restrictions.
    pub fn is_tool_visible(&self, name: &str) -> bool {
        super::tool_visible_with_filters(
            name,
            &self.agent_tool_filter,
            &self.denied_tools,
            &self.skill_allowed_tools,
            &self.plan_mode_allowed_tools,
        )
    }

    /// Push tool-emitted metadata into the per-dispatch sink. No-op when no
    /// sink is wired up (the common case for `execute_tool` direct callers
    /// that don't care about structured side outputs).
    pub async fn emit_metadata(&self, value: Value) {
        if let Some(sink) = &self.metadata_sink {
            *sink.lock().await = Some(value);
        }
    }

    /// Record a spawned child-process pid into the owning async-job row for
    /// restart orphan cleanup (I3). No-op unless a [`PidSink`] is wired (only
    /// backgrounded jobs set one). Synchronous + cheap (a single guarded DB
    /// UPDATE behind the closure).
    pub fn emit_pid(&self, pid: u32) {
        if let Some(sink) = &self.pid_sink {
            (sink.0)(pid);
        }
    }

    /// Push the effective (post-`PreToolUse` rewrite) tool arguments into the
    /// per-dispatch sink. Called once at most, only when `updatedInput`
    /// shadowed the model's args. No-op when no sink is wired up.
    pub(crate) async fn emit_effective_args(&self, value: Value) -> anyhow::Result<()> {
        if let Some(sink) = &self.effective_args_sink {
            sink.publish_and_wait(value).await?;
        }
        Ok(())
    }

    /// Best-effort: tell every matching session/project file-browser view that
    /// a file under the primary or a linked project root changed (agent
    /// `write` / `edit` / `apply_patch`). This uses the same scope identities
    /// as the browser's own CRUD so trees and previews reconcile without a
    /// manual reload. No-op when there is no event bus or no matching root.
    pub fn notify_workspace_file_changed(&self, abs_path: &str) {
        let Some(bus) = crate::globals::get_event_bus() else {
            return;
        };
        for payload in self.workspace_file_change_events(abs_path) {
            bus.emit("project:fs_changed", payload);
        }
    }

    fn workspace_file_change_events(&self, abs_path: &str) -> Vec<Value> {
        // The file may have just been created, so canonicalize its parent dir.
        let Some(parent) = std::path::Path::new(abs_path).parent() else {
            return Vec::new();
        };
        let Ok(parent) = parent.canonicalize() else {
            return Vec::new();
        };

        let mut events = Vec::new();
        if let Some(primary) = self.session_working_dir.as_deref() {
            if let Some(dir) = relative_changed_dir(&parent, primary) {
                if let Some(session_id) = self.session_id.as_deref() {
                    events.push(workspace_changed_payload(
                        "session",
                        session_id.to_string(),
                        self.project_id.as_deref(),
                        &dir,
                    ));
                }
                if self.project_primary_dir.is_none() {
                    if let Some(project_id) = self.project_id.as_deref() {
                        events.push(workspace_changed_payload(
                            "project",
                            project_id.to_string(),
                            Some(project_id),
                            &dir,
                        ));
                    }
                }
            }
        }

        if let Some(project_primary) = self.project_primary_dir.as_deref() {
            if let Some(dir) = relative_changed_dir(&parent, project_primary) {
                if let Some(project_id) = self.project_id.as_deref() {
                    events.push(workspace_changed_payload(
                        "project",
                        project_id.to_string(),
                        Some(project_id),
                        &dir,
                    ));
                }
                let virtual_index = self.project_linked_dirs.len();
                if let Some(session_id) = self.session_id.as_deref() {
                    events.push(workspace_changed_payload(
                        "project_folder",
                        project_folder_scope_id(
                            "session",
                            session_id,
                            virtual_index,
                            project_primary,
                        ),
                        self.project_id.as_deref(),
                        &dir,
                    ));
                }
            }
        }

        for (index, linked_root) in self.project_linked_dirs.iter().enumerate() {
            let Some(dir) = relative_changed_dir(&parent, linked_root) else {
                continue;
            };
            if let Some(session_id) = self.session_id.as_deref() {
                events.push(workspace_changed_payload(
                    "project_folder",
                    project_folder_scope_id("session", session_id, index, linked_root),
                    self.project_id.as_deref(),
                    &dir,
                ));
            }
            if let Some(project_id) = self.project_id.as_deref() {
                events.push(workspace_changed_payload(
                    "project_folder",
                    project_folder_scope_id("project", project_id, index, linked_root),
                    Some(project_id),
                    &dir,
                ));
            }
        }
        events
    }
}

fn relative_changed_dir(parent: &std::path::Path, root: &str) -> Option<String> {
    let root = std::path::Path::new(root).canonicalize().ok()?;
    let relative = parent.strip_prefix(root).ok()?;
    Some(relative.to_string_lossy().replace('\\', "/"))
}

fn project_folder_scope_id(
    base_scope: &str,
    base_id: &str,
    linked_index: usize,
    expected_path: &str,
) -> String {
    format!("{base_scope}:{base_id}:{linked_index}:{expected_path}")
}

fn workspace_changed_payload(
    scope: &str,
    scope_id: String,
    project_id: Option<&str>,
    dir: &str,
) -> Value {
    json!({
        "scope": scope,
        "scopeId": scope_id,
        "projectId": project_id,
        "dir": dir,
    })
}

#[cfg(all(test, unix))]
mod workspace_file_change_tests {
    use super::ToolExecContext;
    use serde_json::json;

    #[test]
    fn linked_root_changes_target_session_and_project_browser_scopes() {
        let base = std::path::Path::new("/tmp").join(format!(
            "ha-workspace-change-events-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        let primary = base.join("primary");
        let linked = base.join("linked");
        std::fs::create_dir_all(primary.join("src")).expect("create primary root");
        std::fs::create_dir_all(linked.join("docs")).expect("create linked root");

        let ctx = ToolExecContext {
            session_id: Some("session-1".into()),
            project_id: Some("project-1".into()),
            session_working_dir: Some(primary.to_string_lossy().into_owned()),
            project_linked_dirs: vec![linked.to_string_lossy().into_owned()],
            ..ToolExecContext::default()
        };

        assert_eq!(
            ctx.workspace_file_change_events(&linked.join("docs/note.md").to_string_lossy()),
            vec![
                json!({
                    "scope": "project_folder",
                    "scopeId": format!("session:session-1:0:{}", linked.display()),
                    "projectId": "project-1",
                    "dir": "docs",
                }),
                json!({
                    "scope": "project_folder",
                    "scopeId": format!("project:project-1:0:{}", linked.display()),
                    "projectId": "project-1",
                    "dir": "docs",
                }),
            ]
        );
        assert_eq!(
            ctx.workspace_file_change_events(&primary.join("src/lib.rs").to_string_lossy()),
            vec![
                json!({
                    "scope": "session",
                    "scopeId": "session-1",
                    "projectId": "project-1",
                    "dir": "src",
                }),
                json!({
                    "scope": "project",
                    "scopeId": "project-1",
                    "projectId": "project-1",
                    "dir": "src",
                }),
            ]
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn linked_root_changes_keep_database_indices_when_primary_duplicates_a_linked_root() {
        let base = std::path::Path::new("/tmp").join(format!(
            "ha-workspace-change-indices-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        let active_linked = base.join("active-linked");
        let changed_linked = base.join("changed-linked");
        std::fs::create_dir_all(&active_linked).expect("create active linked root");
        std::fs::create_dir_all(changed_linked.join("docs")).expect("create changed linked root");

        let ctx = ToolExecContext {
            session_id: Some("session-1".into()),
            project_id: Some("project-1".into()),
            session_working_dir: Some(active_linked.to_string_lossy().into_owned()),
            project_linked_dirs: vec![
                active_linked.to_string_lossy().into_owned(),
                changed_linked.to_string_lossy().into_owned(),
            ],
            ..ToolExecContext::default()
        };

        assert_eq!(
            ctx.workspace_file_change_events(
                &changed_linked.join("docs/note.md").to_string_lossy()
            ),
            vec![
                json!({
                    "scope": "project_folder",
                    "scopeId": format!("session:session-1:1:{}", changed_linked.display()),
                    "projectId": "project-1",
                    "dir": "docs",
                }),
                json!({
                    "scope": "project_folder",
                    "scopeId": format!("project:project-1:1:{}", changed_linked.display()),
                    "projectId": "project-1",
                    "dir": "docs",
                }),
            ]
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn overridden_session_keeps_project_primary_available_and_scopes_events_correctly() {
        let base = std::path::Path::new("/tmp").join(format!(
            "ha-workspace-change-primary-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        let session_root = base.join("session-root");
        let project_primary = base.join("project-primary");
        let linked = base.join("linked");
        std::fs::create_dir_all(session_root.join("src")).expect("create session root");
        std::fs::create_dir_all(project_primary.join("docs")).expect("create project primary");
        std::fs::create_dir_all(&linked).expect("create linked root");

        let ctx = ToolExecContext {
            session_id: Some("session-1".into()),
            project_id: Some("project-1".into()),
            session_working_dir: Some(session_root.to_string_lossy().into_owned()),
            project_primary_dir: Some(project_primary.to_string_lossy().into_owned()),
            project_linked_dirs: vec![linked.to_string_lossy().into_owned()],
            ..ToolExecContext::default()
        };

        assert_eq!(
            ctx.project_file_roots().collect::<Vec<_>>(),
            vec![
                session_root.to_str().unwrap(),
                project_primary.to_str().unwrap(),
                linked.to_str().unwrap(),
            ]
        );
        assert_eq!(
            ctx.workspace_file_change_events(
                &project_primary.join("docs/note.md").to_string_lossy()
            ),
            vec![
                json!({
                    "scope": "project",
                    "scopeId": "project-1",
                    "projectId": "project-1",
                    "dir": "docs",
                }),
                json!({
                    "scope": "project_folder",
                    "scopeId": format!("session:session-1:1:{}", project_primary.display()),
                    "projectId": "project-1",
                    "dir": "docs",
                }),
            ]
        );
        assert_eq!(
            ctx.workspace_file_change_events(&session_root.join("src/lib.rs").to_string_lossy()),
            vec![json!({
                "scope": "session",
                "scopeId": "session-1",
                "projectId": "project-1",
                "dir": "src",
            })]
        );

        let _ = std::fs::remove_dir_all(base);
    }
}

// ── Runtime Timeout Policy Helpers ───────────────────────────────

pub fn should_ignore_model_runtime_timeout_when_user_unlimited(user_limit_secs: u64) -> bool {
    matches!(
        crate::config::cached_config()
            .timeout_policy
            .model_runtime_overrides,
        crate::config::ModelRuntimeTimeoutOverrides::IgnoreWhenUserUnlimited
    ) && user_limit_secs == 0
}

pub fn audit_model_runtime_timeout_override(
    ctx: Option<&ToolExecContext>,
    tool: &str,
    parameter: &str,
    requested_secs: u64,
    effective_secs: u64,
    user_limit_secs: Option<u64>,
    ignored: bool,
    reason: &str,
) {
    let mode = crate::config::cached_config()
        .timeout_policy
        .model_runtime_overrides;
    if matches!(mode, crate::config::ModelRuntimeTimeoutOverrides::Allow) && !ignored {
        return;
    }

    let details = json!({
        "tool": tool,
        "parameter": parameter,
        "requestedSecs": requested_secs,
        "effectiveSecs": effective_secs,
        "userLimitSecs": user_limit_secs,
        "ignored": ignored,
        "reason": reason,
        "policy": mode,
    });
    let level = if ignored { "warn" } else { "info" };
    let message = if ignored {
        format!(
            "Ignored model runtime timeout override for {tool}.{parameter}: requested {requested_secs}s, effective {effective_secs}s ({reason})"
        )
    } else {
        format!(
            "Model runtime timeout override for {tool}.{parameter}: requested {requested_secs}s, effective {effective_secs}s ({reason})"
        )
    };

    if let Some(logger) = crate::get_logger() {
        logger.log(
            level,
            "tool",
            "timeout_policy::model_runtime_override",
            &message,
            Some(details.to_string()),
            ctx.and_then(|c| c.session_id.clone()),
            ctx.and_then(|c| c.agent_id.clone()),
        );
    }
}

pub async fn emit_model_runtime_timeout_metadata(
    ctx: &ToolExecContext,
    tool: &str,
    parameter: &str,
    requested_secs: u64,
    effective_secs: u64,
    user_limit_secs: Option<u64>,
    ignored: bool,
    reason: &str,
) {
    let mode = crate::config::cached_config()
        .timeout_policy
        .model_runtime_overrides;
    if matches!(mode, crate::config::ModelRuntimeTimeoutOverrides::Allow) && !ignored {
        return;
    }

    ctx.emit_metadata(json!({
        "kind": "runtime_timeout_override",
        "tool": tool,
        "parameter": parameter,
        "requestedSecs": requested_secs,
        "effectiveSecs": effective_secs,
        "userLimitSecs": user_limit_secs,
        "ignored": ignored,
        "reason": reason,
        "policy": mode,
    }))
    .await;
}
