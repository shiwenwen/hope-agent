use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::app_update;
use super::issue_report;
use super::send_attachment;
use super::skill;
use super::{
    acp_spawn, browser, core_memory, cron, goal, loop_tool, mac_control, memory, note,
    notification, project_memory, settings, subagent, team, weather, web_fetch, web_search,
    workflow_tool,
};
use super::{
    agents, artifact, ask_user_question, canvas, design, enter_plan_mode, image, image_generate,
    job_status, pdf, runtime_cancel, schedule_wakeup, sessions, submit_plan, task,
};
use super::{apply_patch, edit, exec, find, grep, ls, lsp, process, read, write};
use super::{
    approval, TOOL_ACP_SPAWN, TOOL_AGENTS_LIST, TOOL_APPLY_PATCH, TOOL_ARTIFACT,
    TOOL_ASK_USER_QUESTION, TOOL_BROWSER, TOOL_CANVAS, TOOL_CORE_MEMORY, TOOL_DELETE_MEMORY,
    TOOL_DESIGN, TOOL_EDIT, TOOL_ENTER_PLAN_MODE, TOOL_EXEC, TOOL_FIND, TOOL_GET_SETTINGS,
    TOOL_GET_WEATHER, TOOL_GOAL_BLOCK_REQUEST, TOOL_GOAL_CHECKPOINT, TOOL_GOAL_EVALUATE,
    TOOL_GOAL_FINISH_REQUEST, TOOL_GOAL_PREPARE_CONTRACT, TOOL_GOAL_RECORD_EVIDENCE,
    TOOL_GOAL_STATUS, TOOL_GREP, TOOL_IMAGE, TOOL_IMAGE_GENERATE, TOOL_ISSUE_REPORT,
    TOOL_JOB_STATUS, TOOL_LIST_SETTINGS_BACKUPS, TOOL_LOOP_RECORD_PROGRESS, TOOL_LOOP_RESCHEDULE,
    TOOL_LOOP_STATUS, TOOL_LOOP_STOP, TOOL_LOOP_UNWATCH, TOOL_LOOP_WATCH, TOOL_LS, TOOL_LSP,
    TOOL_MAC_CONTROL, TOOL_MANAGE_CRON, TOOL_MEMORY_GET, TOOL_PDF, TOOL_PROCESS,
    TOOL_PROJECT_MEMORY, TOOL_READ, TOOL_RECALL_MEMORY, TOOL_RESTORE_SETTINGS_BACKUP,
    TOOL_RUNTIME_CANCEL, TOOL_SAVE_MEMORY, TOOL_SEND_ATTACHMENT, TOOL_SEND_NOTIFICATION,
    TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST, TOOL_SESSIONS_SEARCH, TOOL_SESSIONS_SEND,
    TOOL_SESSION_STATUS, TOOL_SUBAGENT, TOOL_SUBMIT_PLAN, TOOL_TASK_CREATE, TOOL_TASK_LIST,
    TOOL_TASK_UPDATE, TOOL_TEAM, TOOL_UPDATE_CORE_MEMORY, TOOL_UPDATE_MEMORY, TOOL_UPDATE_SETTINGS,
    TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WORKFLOW, TOOL_WRITE,
};
use super::{
    TOOL_KNOWLEDGE_RECALL, TOOL_NOTE_APPEND, TOOL_NOTE_ASSIGN_BLOCK, TOOL_NOTE_BACKLINKS,
    TOOL_NOTE_BROKEN_LINKS, TOOL_NOTE_BY_TAG, TOOL_NOTE_CREATE, TOOL_NOTE_DELETE,
    TOOL_NOTE_DISTILL, TOOL_NOTE_GRAPH, TOOL_NOTE_LINK, TOOL_NOTE_MOC, TOOL_NOTE_MOVE,
    TOOL_NOTE_ORPHANS, TOOL_NOTE_PATCH, TOOL_NOTE_READ, TOOL_NOTE_RELATED, TOOL_NOTE_RENAME,
    TOOL_NOTE_SEARCH, TOOL_NOTE_SET_FRONTMATTER, TOOL_NOTE_SIMILAR, TOOL_NOTE_SUGGEST_LINKS,
    TOOL_NOTE_TAGS, TOOL_NOTE_UPDATE, TOOL_SESSION_TO_NOTE,
};
use crate::agent_config::AsyncToolPolicy;
use crate::async_jobs::{self, JobOrigin};

/// Single entry point that builds a [`permission::engine::ResolveContext`]
/// from a [`ToolExecContext`] and runs `engine::resolve_async`. Both
/// `execute_tool_with_context` (engine gate) and `tools::exec::tool_exec`
/// (command-level gate) call this so the 14-field context struct lives in
/// exactly one place — adding a new permission input only touches here.
///
/// Smart sessions are the only mode that consumes
/// `AppConfig.permission.smart`; non-Smart skips the config load to keep
/// the per-dispatch hot path at one ArcSwap::load() (or zero, for the
/// Default/YOLO majority).
pub(super) async fn resolve_tool_permission(
    tool_name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    is_internal_tool: bool,
) -> crate::permission::Decision {
    // Mid-turn Plan Mode entry guard: `ctx.plan_mode_allowed_tools` is a
    // snapshot taken when the AssistantAgent was built at turn start. If the
    // model called `enter_plan_mode` mid-turn (user accepted) the live state
    // is now Planning/Review while the snapshot still says Off, so the
    // permission engine would happily run write/edit/apply_patch/canvas. Fall
    // back to a hard deny on those four mutation tools so the user-sovereignty
    // contract holds within the same turn — full PlanAgent restrictions kick
    // in automatically on the next user message when the agent rebuilds.
    if !is_internal_tool && ctx.plan_mode_allowed_tools.is_empty() {
        if let Some(sid) = ctx.session_id.as_deref() {
            let live = crate::plan::get_plan_state(sid).await;
            if matches!(
                live,
                crate::plan::PlanModeState::Planning | crate::plan::PlanModeState::Review
            ) && crate::plan::PLAN_MODE_DENIED_TOOLS.contains(&tool_name)
            {
                return crate::permission::Decision::Deny {
                    reason: format!(
                        "Plan Mode (state: {}) just entered this turn — '{}' is denied. \
                         Use read/grep/glob/web_search/web_fetch/ask_user_question/submit_plan \
                         until the plan is approved.",
                        live.as_str(),
                        tool_name
                    ),
                };
            }
        }
    }

    let app_cfg = (ctx.session_mode == crate::permission::SessionMode::Smart)
        .then(crate::config::cached_config);
    // Smart-judge calibration for unattended runs: when no human can approve, the
    // judge is told so (and given the pre-authorized task intent, for cron) so it
    // allows in-scope actions and denies out-of-scope / injected ones. Reuse the
    // canonical `evaluate_approval_surface` — the single source of truth for "no
    // one can approve" (cron, cron-lineage subagents via C03, headless-no-client,
    // ACP-no-capability) — instead of re-deriving from chat_source (which would
    // miss cron-spawned subagents). Gated to Smart sessions because only the judge
    // consumes it, keeping the surface lookup off the hot path for default/yolo.
    // The intent String must outlive the borrow in `resolve_ctx` → local binding.
    let unattended = ctx.session_mode == crate::permission::SessionMode::Smart
        && matches!(
            crate::permission::evaluate_approval_surface(ctx.session_id.as_deref()),
            crate::permission::ApprovalSurface::Unattended(_)
        );
    let cron_intent: Option<String> = if unattended {
        ctx.session_id
            .as_deref()
            .and_then(crate::permission::task_intent::get)
    } else {
        None
    };
    let resolve_ctx = crate::permission::engine::ResolveContext {
        tool_name,
        args,
        session_mode: ctx.session_mode,
        sandbox_mode: ctx.sandbox_mode,
        global_yolo: crate::security::dangerous::is_dangerous_skip_active(),
        plan_mode: !ctx.plan_mode_allowed_tools.is_empty(),
        plan_mode_allowed_tools: &ctx.plan_mode_allowed_tools,
        plan_mode_ask_tools: &ctx.plan_mode_ask_tools,
        agent_custom_approval_enabled: ctx.agent_custom_approval_enabled,
        agent_custom_approval_tools: &ctx.agent_custom_approval_tools,
        session_id: ctx.session_id.as_deref(),
        project_id: ctx.project_id.as_deref(),
        agent_id: ctx.agent_id.as_deref(),
        default_path: Some(ctx.default_path()),
        is_internal_tool,
        smart_config: app_cfg.as_deref().map(|c| &c.permission.smart),
        unattended,
        task_intent: cron_intent.as_deref(),
    };
    crate::permission::engine::resolve_async(&resolve_ctx).await
}

/// Record the target path(s) of a `write` / `edit` / `apply_patch` call into
/// the session-edit tracker so Smart mode won't re-prompt on later edits to the
/// same file. No-op for non-edit tools (empty target list) and sessionless
/// calls. Paths use the same canonical resolution as the permission engine.
fn record_smart_session_edits(name: &str, args: &Value, ctx: &ToolExecContext) {
    let Some(session_id) = ctx.session_id.as_deref() else {
        return;
    };
    for path in crate::permission::rules::resolved_edit_target_paths(
        name,
        args,
        Some(std::path::Path::new(ctx.default_path())),
    ) {
        crate::permission::session_edits::record(session_id, &path);
    }
}

/// Load the user-configured tool timeout from config.json. Returns `None`
/// when the user explicitly set 0 (disabled). The serde default in
/// [`AppConfig`] also defaults missing values to 0 (disabled).
fn tool_timeout(ctx: &ToolExecContext) -> Option<Duration> {
    if ctx.suppress_global_tool_timeout {
        return None;
    }
    let secs = crate::config::cached_config().tool_timeout;
    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

const TOOL_TIMEOUT_CLEANUP_GRACE: Duration = Duration::from_secs(5);

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
/// `crates/ha-core/src/agent/providers/{anthropic,openai_chat,openai_responses,codex}.rs`,
/// look for `let tool_ctx = tool_ctx.clone();`). All current fields are value
/// types or owned `Vec`s, so the clone is independent and a tool only ever
/// observes its own snapshot.
///
/// **Do not** add `Mutex`/`RwLock` directly to this struct. Each concurrent
/// branch holds an independent clone, so writes through such a lock would be
/// invisible to peers and to subsequent rounds. State that must be shared
/// across concurrent tools belongs in a process-global
/// `OnceLock<TokioMutex<...>>` (see
/// [`super::approval::pending_approvals_per_session`] for the canonical
/// pattern).
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
    /// Current session ID (for sub-agent spawning context)
    pub session_id: Option<String>,
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
    /// permission engine gate (see [`needs_permission_engine`]) was already
    /// satisfied at the outer dispatch. Inner re-entries skip the engine
    /// gate but **still run command-level gates** (notably `exec`'s
    /// dangerous/edit-command + AllowAlways audit), because for the `exec`
    /// tool those gates are intentionally bypassed at the outer engine layer
    /// (`needs_permission_engine` excludes `TOOL_EXEC`) and `exec` is
    /// expected to run them itself.
    ///
    /// Differs from [`Self::auto_approve_tools`], which means "skip ALL
    /// approval gates including command-level" and is set only by IM
    /// auto-approve accounts or slash-skill execution.
    pub external_pre_approved: bool,
    /// Set ONLY by the async approval-reorder path
    /// ([`execute_tool_with_context`]) after it has already run `exec`'s
    /// command-level gate ([`exec::resolve_exec_command_approval`]) and the
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
    pub approval_origin: Option<approval::ApprovalOrigin>,
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
    /// ([`maybe_persist_large_tool_result`]) and async-job persistence
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
    /// Per-dispatch sink for the *effective* tool arguments — populated by
    /// [`execute_tool_with_context`] when a `PreToolUse` hook rewrites the
    /// input via `updatedInput`. Same lifecycle as `metadata_sink`: the
    /// orchestrator constructs the `Arc<Mutex<None>>` per dispatch, clones it
    /// into the local context, and drains it after the tool returns so the
    /// caller can surface the rewrite through the UI / persisted history /
    /// `PostToolUse` hook input. None ⇒ no rewrite occurred (or the caller
    /// doesn't care; the regular non-orchestrator callers of
    /// `execute_tool_with_context` leave it `None`).
    pub effective_args_sink: Option<Arc<AsyncMutex<Option<Value>>>>,
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
            session_id,
            transcript_path,
            cwd: std::path::PathBuf::from(self.default_cwd()),
            permission_mode,
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

    fn builtin_fate_error(&self, name: &str) -> Option<String> {
        let canonical = canonical_builtin_tool_name(name);
        let agent_id = self
            .agent_id
            .as_deref()
            .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID);
        let agent_def = crate::agent_loader::load_agent(agent_id).ok();
        let default_cfg = crate::agent_config::AgentConfig::default();
        let agent_cfg = agent_def
            .as_ref()
            .map(|d| &d.config)
            .unwrap_or(&default_cfg);

        if crate::mcp::catalog::is_mcp_tool_name(canonical) && !agent_cfg.capabilities.mcp_enabled {
            return Some(format!(
                "Agent tool switch: MCP tools are disabled for this agent, so '{}' cannot execute.",
                name
            ));
        }

        let def = super::dispatch::all_dispatchable_tools()
            .iter()
            .find(|def| def.name == canonical)?;

        // Plan-mode tools are injected by `AssistantAgent::apply_plan_tools`
        // according to live session state rather than by static tool fate.
        // Their handlers also validate the active plan state, so the generic
        // dispatcher verdict (`Hidden`) must not block legitimate calls.
        if matches!(
            def.tier,
            super::ToolTier::Core {
                subclass: super::CoreSubclass::PlanMode
            }
        ) {
            return None;
        }

        let app_config = crate::config::cached_config();
        let session_access = crate::memory::effective_session_memory_access(
            self.session_id.as_deref(),
            self.session_db.as_ref().map(|handle| handle.0.as_ref()),
        );
        let dispatch_ctx = super::dispatch::DispatchContext {
            agent_id,
            incognito: self.incognito,
            mcp_enabled: agent_cfg.capabilities.mcp_enabled,
            memory_enabled: agent_cfg.memory.enabled,
            use_memories: session_access.use_memories,
            contribute_to_memories: session_access.contribute_to_memories,
            tools_filter: &agent_cfg.capabilities.tools,
            app_config: &app_config,
        };

        match super::dispatch::resolve_tool_fate(def, &dispatch_ctx) {
            super::dispatch::ToolFate::InjectEager | super::dispatch::ToolFate::InjectDeferred => {
                None
            }
            super::dispatch::ToolFate::HintOnly { config_hint } => Some(format!(
                "Agent tool switch: tool '{}' is enabled but not configured. {}",
                canonical, config_hint
            )),
            super::dispatch::ToolFate::Hidden
                if self.incognito && super::is_memory_tool(canonical) =>
            {
                Some(format!(
                    "Incognito restriction: long-term memory tool '{}' is unavailable in this session.",
                    canonical
                ))
            }
            super::dispatch::ToolFate::Hidden => Some(format!(
                "Agent tool switch: tool '{}' is disabled for this agent.",
                canonical
            )),
        }
    }

    async fn workflow_visibility_error(&self, name: &str) -> Option<String> {
        if canonical_builtin_tool_name(name) != TOOL_WORKFLOW {
            return None;
        }
        let Some(session_id) = self.session_id.as_deref() else {
            return Some(
                "workflow requires an active session with Workflow Mode enabled.".to_string(),
            );
        };
        if self.incognito {
            return Some(
                "workflow is disabled for incognito sessions because workflow runs are durable."
                    .to_string(),
            );
        }
        let Some(db) = self
            .session_db
            .as_ref()
            .map(|handle| handle.0.clone())
            .or_else(|| crate::get_session_db().cloned())
        else {
            return Some("workflow cannot execute because Session DB is unavailable.".into());
        };
        let session_id = session_id.to_string();
        let mode = match db
            .run(move |db| db.get_session_workflow_mode(&session_id))
            .await
        {
            Ok(Some(mode)) => mode,
            Ok(None) => Default::default(),
            Err(e) => {
                return Some(format!(
                    "workflow cannot read the session Workflow Mode: {e}"
                ));
            }
        };
        if !mode.enabled() {
            return Some(
                "Workflow Mode is off for this session. Use `/workflow on` or the GUI toggle before calling workflow."
                    .to_string(),
            );
        }
        None
    }

    /// Human-readable reason when a tool is blocked by the current restrictions.
    pub async fn tool_visibility_error(&self, name: &str) -> Option<String> {
        if let Some(err) = self.builtin_fate_error(name) {
            return Some(err);
        }
        if let Some(err) = self.workflow_visibility_error(name).await {
            return Some(err);
        }
        if self.denied_tools.iter().any(|t| t == name) {
            return Some(format!(
                "Tool policy restriction: tool '{}' is denied in the current agent context.",
                name
            ));
        }
        if !self.skill_allowed_tools.is_empty()
            && !self.skill_allowed_tools.iter().any(|t| t == name)
        {
            return Some(format!(
                "Skill restriction: tool '{}' is not allowed by the active skill.",
                name
            ));
        }
        if !self.plan_mode_allowed_tools.is_empty()
            && !self.plan_mode_allowed_tools.iter().any(|t| t == name)
        {
            return Some(format!(
                "Plan Mode restriction: tool '{}' is not allowed during planning. Allowed: {}",
                name,
                self.plan_mode_allowed_tools.join(", ")
            ));
        }
        None
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
    pub(crate) async fn emit_effective_args(&self, value: Value) {
        if let Some(sink) = &self.effective_args_sink {
            *sink.lock().await = Some(value);
        }
    }

    /// Best-effort: tell any open file-browser view that a file under this
    /// session's working directory just changed (agent `write` / `edit` /
    /// `apply_patch`), so the tree/preview reconcile without a manual reload —
    /// the same `project:fs_changed` event the browser's own CRUD emits. No-op
    /// when there's no session, no working dir, no event bus, or the path falls
    /// outside the working directory.
    pub fn notify_workspace_file_changed(&self, abs_path: &str) {
        let (Some(sid), Some(wd)) = (
            self.session_id.as_deref(),
            self.session_working_dir.as_deref(),
        ) else {
            return;
        };
        let Some(bus) = crate::globals::get_event_bus() else {
            return;
        };
        let Ok(root) = std::path::Path::new(wd).canonicalize() else {
            return;
        };
        // The file may have just been created, so canonicalize its parent dir.
        let Some(parent) = std::path::Path::new(abs_path).parent() else {
            return;
        };
        let Ok(parent) = parent.canonicalize() else {
            return;
        };
        let Ok(rel) = parent.strip_prefix(&root) else {
            return; // outside the working dir — not a browseable change
        };
        let dir = rel.to_string_lossy().replace('\\', "/");
        bus.emit(
            "project:fs_changed",
            serde_json::json!({
                "scope": "session",
                "scopeId": sid,
                "projectId": self.project_id.as_deref(),
                "dir": dir,
            }),
        );
    }
}

fn canonical_builtin_tool_name(name: &str) -> &str {
    match name {
        "read_file" => TOOL_READ,
        "write_file" => TOOL_WRITE,
        "patch_file" => TOOL_EDIT,
        _ => name,
    }
}

// ── Tool Execution (provider-agnostic) ────────────────────────────

/// Execute a tool by name with the given JSON arguments.
#[allow(dead_code)]
pub async fn execute_tool(name: &str, args: &Value) -> anyhow::Result<String> {
    execute_tool_with_context(name, args, &ToolExecContext::default()).await
}

/// Outcome of the async-tool dispatch decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AsyncDecision {
    /// Tool is sync-only — run through the normal dispatch + tool_timeout path.
    Sync,
    /// Tool is async-capable but the model didn't opt in and the policy is
    /// `model-decide`. Race the dispatch against `auto_background_secs`.
    AutoBackgroundEligible,
    /// Tool must be detached immediately (explicit `run_in_background: true`
    /// or policy `always-background`).
    ImmediateBackground(JobOrigin),
}

/// Which exec-native process lifecycle the call requested, if any.
///
/// `exec(background=true)` and `exec(yield_ms=...)` return a process
/// `session_id` and are later observed through `process(action=...)`. That is a
/// separate lifecycle from async tool jobs. The execution entry migrates
/// ordinary uses to async_jobs when available, leaving this detector for
/// compatibility paths that still need the process-session surface.
fn exec_process_background_mode(args: &Value) -> Option<&'static str> {
    let background = args
        .get("background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_yield_ms = args.get("yield_ms").is_some();

    match (background, has_yield_ms) {
        (true, true) => Some("background/yield_ms"),
        (true, false) => Some("background"),
        (false, true) => Some("yield_ms"),
        (false, false) => None,
    }
}

fn explicit_async_job_requested(args: &Value) -> bool {
    args.get("run_in_background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn should_migrate_exec_process_mode_to_async_job(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
) -> bool {
    should_migrate_exec_process_mode_to_async_job_with_config(
        name,
        args,
        ctx,
        crate::config::cached_config().async_tools.enabled,
    )
}

fn should_migrate_exec_process_mode_to_async_job_with_config(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    async_enabled: bool,
) -> bool {
    if name != TOOL_EXEC || ctx.bypass_async_dispatch {
        return false;
    }
    if exec_process_background_mode(args).is_none() {
        return false;
    }
    if matches!(ctx.async_tool_policy, AsyncToolPolicy::NeverBackground) {
        return false;
    }
    async_enabled
}

fn migrate_exec_process_mode_to_async_job_args(args: &Value) -> Option<Value> {
    let mut migrated = args.clone();
    let obj = migrated.as_object_mut()?;
    obj.remove("background");
    obj.remove("yield_ms");
    obj.insert("run_in_background".to_string(), Value::Bool(true));
    Some(migrated)
}

fn validate_async_background_contract(name: &str, args: &Value) -> anyhow::Result<()> {
    if name == TOOL_EXEC {
        if let (true, Some(process_mode)) = (
            explicit_async_job_requested(args),
            exec_process_background_mode(args),
        ) {
            anyhow::bail!(
                "exec background conflict: do not combine `run_in_background` with \
                 exec `{}` mode. Choose one lifecycle: use `run_in_background` for \
                 an async job whose result is delivered through `job_status` / \
                 task notification, or use `background` / `yield_ms` for an exec \
                 process session managed with `process(action=\"poll\"|\"log\"|\"kill\")`.",
                process_mode
            );
        }
    }
    Ok(())
}

/// Inspect tool metadata, args, and agent policy to decide whether this call
/// should detach immediately, become eligible for auto-background, or run
/// purely synchronously. Recursion-safe via `bypass_async_dispatch`.
fn decide_async_path(name: &str, args: &Value, ctx: &ToolExecContext) -> AsyncDecision {
    let cfg = crate::config::cached_config();
    decide_async_path_with_config(
        name,
        args,
        ctx,
        cfg.async_tools.enabled,
        cfg.async_tools.auto_background_secs,
    )
}

fn decide_async_path_with_config(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    async_enabled: bool,
    auto_background_secs: u64,
) -> AsyncDecision {
    if ctx.bypass_async_dispatch {
        return AsyncDecision::Sync;
    }
    if !super::is_async_capable(name) {
        return AsyncDecision::Sync;
    }
    if !async_enabled {
        return AsyncDecision::Sync;
    }
    if matches!(ctx.async_tool_policy, AsyncToolPolicy::NeverBackground) {
        return AsyncDecision::Sync;
    }
    if explicit_async_job_requested(args) {
        return AsyncDecision::ImmediateBackground(JobOrigin::Explicit);
    }

    // Exec has its own process-session backgrounding surface:
    // `background=true` and `yield_ms` return a session id and are controlled
    // by the `process` tool. The default path migrates legacy requests to
    // `run_in_background` before this decision is computed; the remaining
    // process-session requests are explicit compatibility paths and must not be
    // wrapped in async_jobs too.
    if name == TOOL_EXEC && exec_process_background_mode(args).is_some() {
        return AsyncDecision::Sync;
    }

    if matches!(ctx.async_tool_policy, AsyncToolPolicy::AlwaysBackground) {
        return AsyncDecision::ImmediateBackground(JobOrigin::PolicyForced);
    }
    if auto_background_secs > 0 {
        return AsyncDecision::AutoBackgroundEligible;
    }
    AsyncDecision::Sync
}

/// Whether the exec async approval-reorder should run `exec`'s command gate
/// *before* detaching the call into a background job (B5/B6). It runs only when
/// all hold:
///   - the tool is `exec` (the only tool excluded from the outer engine gate);
///   - the call is **auto-background-eligible** — a plain exec that backgrounds
///     only if it outlives the foreground budget. For these the approval must
///     resolve up front so the wait stays out of the `auto_background_secs` /
///     `max_job_secs` budgets (ASYNC-2). **Explicit `ImmediateBackground`**
///     (`run_in_background:true` / policy AlwaysBackground) is deliberately
///     EXCLUDED (R8): its command gate is deferred to the background job thread,
///     where an attended approval parks the job at `AwaitingApproval` and the
///     decision resolves asynchronously — the model gets the job id immediately
///     and a denial settles the job terminal instead of blocking the turn. See
///     `async_jobs::approval_bridge`.
///   - exec was NOT already approved at the outer engine gate this turn
///     (`already_approved`) — set by the Plan-Mode-ask path so the reorder
///     doesn't re-prompt for the identical command (review#3); and
///   - the command gate isn't globally bypassed
///     (`ctx.should_run_exec_command_gate()` = `!auto_approve_tools &&
///     !exec_pre_approved` on the ctx).
fn should_run_exec_reorder_gate(
    name: &str,
    async_decision: AsyncDecision,
    already_approved: bool,
    ctx: &ToolExecContext,
) -> bool {
    name == TOOL_EXEC
        && matches!(async_decision, AsyncDecision::AutoBackgroundEligible)
        && !already_approved
        && ctx.should_run_exec_command_gate()
}

/// Check if a read tool call targets a SKILL.md file (pre-authorized by skill system).
fn is_skill_read(name: &str, args: &Value) -> bool {
    if name != TOOL_READ {
        return false;
    }
    args.get("path")
        .and_then(|v| v.as_str())
        .map(|p| p.ends_with("/SKILL.md") || p.ends_with("\\SKILL.md"))
        .unwrap_or(false)
}

fn mcp_server_auto_approves_config(cfg: &crate::mcp::McpServerConfig) -> bool {
    cfg.auto_approve && matches!(cfg.trust_level, crate::mcp::McpTrustLevel::Trusted)
}

async fn mcp_tool_auto_approves(name: &str) -> bool {
    if !crate::mcp::catalog::is_mcp_tool_name(name) {
        return false;
    }
    let Some(manager) = crate::mcp::McpManager::global() else {
        return false;
    };
    let Some(entry) = manager.lookup_tool(name).await else {
        return false;
    };
    let Some(handle) = manager.get_by_id(&entry.server_id).await else {
        return false;
    };
    let cfg = handle.config.read().await;
    mcp_server_auto_approves_config(&cfg)
}

fn needs_permission_engine(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    effective_auto_approve: bool,
) -> bool {
    let plan_mode_active = !ctx.plan_mode_allowed_tools.is_empty();
    let plan_requires_ask = plan_mode_active && ctx.plan_mode_ask_tools.iter().any(|t| t == name);
    let auto_approve_blocked_by_plan = effective_auto_approve && plan_requires_ask;
    let exec_skip_blocked_by_plan = name == TOOL_EXEC && plan_requires_ask;
    let connector_action_requires_approval = !ctx.external_pre_approved
        && crate::permission::engine::classify_external_connector_action(name, args).is_some();
    if connector_action_requires_approval && !is_skill_read(name, args) {
        return true;
    }
    (!effective_auto_approve || auto_approve_blocked_by_plan)
        && !is_skill_read(name, args)
        && (name != TOOL_EXEC || exec_skip_blocked_by_plan)
}

async fn capture_mac_control_approval_focus_anchor(
    name: &str,
) -> Option<crate::mac_control::MacControlFocusAnchor> {
    if name == TOOL_MAC_CONTROL {
        crate::mac_control::capture_focus_anchor().await
    } else {
        None
    }
}

async fn restore_mac_control_approval_focus_anchor(
    anchor: Option<crate::mac_control::MacControlFocusAnchor>,
) {
    let Some(anchor) = anchor else {
        return;
    };
    if let Err(error) = crate::mac_control::restore_focus_anchor(&anchor).await {
        app_warn!(
            "tool",
            "approval_focus",
            "Failed to restore macOS focus after approval: {}",
            error
        );
    }
}

/// Execute a tool with additional context (model info, etc.)
/// Outcome of the `PreToolUse` hook gate (design §9.3/§9.4). Fires after the
/// name-based visibility gate and before the permission engine.
enum PreToolGate {
    /// A hook denied/blocked the call — short-circuit (no downstream gate can
    /// rescue a hook deny; it's a top-level block).
    Deny(String),
    /// Proceed. `updated_input` patches the tool args (the engine then re-checks
    /// the patched values, so an arg-rewrite can't dodge a path/command gate).
    /// `skip_user_prompt` (explicit `permissionDecision:"allow"`) downgrades a
    /// *soft* engine `Ask` to allow — never a hard Deny, and never a strict
    /// prompt (protected path / dangerous command / Plan ask). `force_prompt`
    /// (`ask`/`defer`) forces the approval prompt even when the engine would
    /// allow, so a hook's request for confirmation can't silently fail open.
    Proceed {
        updated_input: Option<Value>,
        skip_user_prompt: bool,
        force_prompt: bool,
    },
}

/// Run the `PreToolUse` hook for this call. No-op fast path when no hook listens.
async fn fire_pre_tool_use_hook(name: &str, args: &Value, ctx: &ToolExecContext) -> PreToolGate {
    use crate::hooks::{HookDispatcher, HookEvent, HookInput};
    // Resolve the same per-cwd scope the dispatcher will: project/local hooks
    // live under the session working dir, so this fast-path gate must use
    // `any_handlers_for(event, cwd)` (not the global-only registry) or a
    // project-only `PreToolUse` hook is silently skipped while `dispatch` would
    // have run it. Mirrors `hooks::session_working_dir` (empty sid → no cwd).
    let wd = ctx
        .session_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .and_then(|sid| crate::session::effective_session_working_dir(Some(sid)));
    if !crate::hooks::scopes::any_handlers_for(
        HookEvent::PreToolUse,
        wd.as_deref().map(std::path::Path::new),
    ) {
        return PreToolGate::Proceed {
            updated_input: None,
            skip_user_prompt: false,
            force_prompt: false,
        };
    }
    let input = HookInput::PreToolUse {
        common: ctx.common_hook_input("PreToolUse"),
        tool_name: name.to_string(),
        tool_input: args.clone(),
        tool_use_id: ctx.tool_call_id.clone().unwrap_or_default(),
    };
    let outcome = HookDispatcher::dispatch(HookEvent::PreToolUse, input).await;
    pre_tool_gate_from_outcome(outcome)
}

/// Pure mapping from a `PreToolUse` aggregate outcome to a [`PreToolGate`].
///
/// `continue:false` is treated as a top-level block ahead of the `decision`
/// match — a Claude Code-style safety hook returning
/// `{"continue":false,"stopReason":"..."}` (without an explicit
/// `permissionDecision:"deny"`) must halt the call, not silently fall through
/// the `Allow` arm. The `decision` match still wins inside `continue:true` so
/// `Ask` / `Defer` keep their force-prompt semantics.
fn pre_tool_gate_from_outcome(outcome: crate::hooks::HookOutcome) -> PreToolGate {
    use crate::hooks::HookDecision;
    if !outcome.continue_execution {
        let reason = outcome
            .stop_reason
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "Tool blocked by a PreToolUse hook (continue:false).".to_string());
        return PreToolGate::Deny(reason);
    }
    match outcome.decision {
        HookDecision::Deny { reason } | HookDecision::Block { reason } => PreToolGate::Deny(reason),
        // Skip the prompt only when the *aggregate* verdict is an explicit
        // allow. `permission_allow` is OR-folded across hooks, so honoring it
        // under an `Ask` aggregate would let one hook's allow suppress another
        // hook's deliberate `ask` — gate it on the winning decision being Allow.
        HookDecision::Allow => PreToolGate::Proceed {
            updated_input: outcome.updated_input,
            skip_user_prompt: outcome.permission_allow,
            force_prompt: false,
        },
        // Ask / Defer: the hook wants human confirmation — force the prompt.
        HookDecision::Ask | HookDecision::Defer => PreToolGate::Proceed {
            updated_input: outcome.updated_input,
            skip_user_prompt: false,
            force_prompt: true,
        },
    }
}

#[cfg(test)]
mod pre_tool_gate_tests {
    use super::*;
    use crate::hooks::{HookDecision, HookOutcome};

    #[test]
    fn continue_false_with_reason_maps_to_deny() {
        let mut outcome = HookOutcome::noop();
        outcome.continue_execution = false;
        outcome.stop_reason = Some("blocked by safety hook".into());
        match pre_tool_gate_from_outcome(outcome) {
            PreToolGate::Deny(r) => assert_eq!(r, "blocked by safety hook"),
            PreToolGate::Proceed { .. } => panic!("expected Deny on continue:false"),
        }
    }

    #[test]
    fn continue_false_without_reason_uses_default_message() {
        let mut outcome = HookOutcome::noop();
        outcome.continue_execution = false;
        // stop_reason absent (None) or whitespace-only is treated identically.
        match pre_tool_gate_from_outcome(outcome) {
            PreToolGate::Deny(r) => {
                assert!(
                    r.contains("continue:false"),
                    "default reason mentions cause, got {r:?}"
                );
            }
            PreToolGate::Proceed { .. } => panic!("expected Deny on continue:false"),
        }
    }

    #[test]
    fn continue_false_overrides_explicit_allow_decision() {
        // A hook can set `permissionDecision:"allow"` *and* `continue:false` —
        // the loop-terminate signal must win over the auto-approve.
        let mut outcome = HookOutcome::noop();
        outcome.decision = HookDecision::Allow;
        outcome.permission_allow = true;
        outcome.continue_execution = false;
        outcome.stop_reason = Some("halt".into());
        match pre_tool_gate_from_outcome(outcome) {
            PreToolGate::Deny(r) => assert_eq!(r, "halt"),
            PreToolGate::Proceed { .. } => {
                panic!("continue:false must not be overridden by permission_allow")
            }
        }
    }

    #[test]
    fn allow_with_continue_true_proceeds() {
        let mut outcome = HookOutcome::noop();
        outcome.decision = HookDecision::Allow;
        outcome.permission_allow = true;
        match pre_tool_gate_from_outcome(outcome) {
            PreToolGate::Proceed {
                skip_user_prompt,
                force_prompt,
                ..
            } => {
                assert!(skip_user_prompt);
                assert!(!force_prompt);
            }
            PreToolGate::Deny(_) => panic!("expected Proceed on Allow + continue:true"),
        }
    }

    #[test]
    fn ask_forces_prompt() {
        let mut outcome = HookOutcome::noop();
        outcome.decision = HookDecision::Ask;
        match pre_tool_gate_from_outcome(outcome) {
            PreToolGate::Proceed {
                skip_user_prompt,
                force_prompt,
                ..
            } => {
                assert!(!skip_user_prompt);
                assert!(force_prompt);
            }
            PreToolGate::Deny(_) => panic!("Ask must Proceed with force_prompt"),
        }
    }
}

/// Show the user approval prompt and map the response to a result. `Ok(())`
/// means proceed (approved, or timed-out-with-`proceed` policy); `Err` blocks
/// the call. `reason_payload` drives the dialog's reason banner (`None` =
/// no banner, used for a hook-forced prompt); `allow_always_forbidden` reflects
/// whether the reason bars an "Allow Always".
pub(super) async fn run_tool_approval(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    reason_payload: Option<approval::ApprovalReasonPayload>,
    allow_always_forbidden: bool,
    desc_override: Option<String>,
) -> anyhow::Result<approval::ApprovalOrigin> {
    let desc = desc_override.unwrap_or_else(|| {
        format!("tool: {} {}", name, {
            let s = args.to_string();
            if s.len() > 200 {
                format!("{}...", crate::truncate_utf8(&s, 200))
            } else {
                s
            }
        })
    });
    let cwd = ctx.default_path();
    match approval::check_and_request_approval(
        &desc,
        cwd,
        ctx.session_id.as_deref(),
        reason_payload,
    )
    .await
    {
        Ok(approval::ApprovalResponse::AllowOnce) => {
            app_info!("tool", "approval", "Tool '{}' approved (once)", name);
            Ok(approval::ApprovalOrigin::User)
        }
        Ok(approval::ApprovalResponse::AllowAlways) => {
            if allow_always_forbidden {
                app_info!(
                    "tool",
                    "approval",
                    "Tool '{}' approved once (AllowAlways unavailable for this reason)",
                    name
                );
            } else {
                // Persist the multi-scope AllowAlways grant (#244). `exec` still
                // uses the legacy command-prefix store inside `tool_exec`.
                match crate::permission::allowlist::add_allow_always_for_call(
                    name,
                    args,
                    ctx.allowlist_grant_context(),
                ) {
                    Ok(grant) => app_info!(
                        "tool",
                        "approval",
                        "Tool '{}' approved (always, scope={}, rule={:?})",
                        name,
                        grant.scope.as_str(),
                        grant.rule
                    ),
                    Err(e) => app_warn!(
                        "tool",
                        "approval",
                        "Tool '{}' AllowAlways persistence failed; approved for this call only: {}",
                        name,
                        e
                    ),
                }
            }
            Ok(approval::ApprovalOrigin::User)
        }
        Ok(approval::ApprovalResponse::Deny) => {
            Err(super::rejection::ToolRejection::denied_by_user(name))
        }
        Err(approval::ApprovalCheckError::TimedOut {
            timeout_secs,
            strict,
            action,
        }) => {
            // F2 (TIMEOUT-1): a strict reason (protected path / dangerous command
            // / mac-dangerous / plan-ask) must NEVER auto-proceed unattended —
            // force a deny even when `approval_timeout_action=proceed`.
            if strict {
                app_warn!(
                    "permission",
                    "strict_timeout_deny",
                    "Tool '{}' approval timed out after {}s; reason is strict — forcing deny",
                    name,
                    timeout_secs
                );
                return Err(super::rejection::ToolRejection::approval_timeout(
                    name,
                    timeout_secs,
                ));
            }
            match action {
                crate::config::ApprovalTimeoutAction::Deny => {
                    app_warn!(
                        "tool",
                        "approval",
                        "Tool '{}' approval timed out after {}s; blocking execution",
                        name,
                        timeout_secs
                    );
                    Err(super::rejection::ToolRejection::approval_timeout(
                        name,
                        timeout_secs,
                    ))
                }
                crate::config::ApprovalTimeoutAction::Proceed => {
                    app_warn!(
                        "tool",
                        "approval",
                        "Tool '{}' approval timed out after {}s; proceeding by config",
                        name,
                        timeout_secs
                    );
                    // F6: weaker-than-click authorization for the audit column.
                    Ok(approval::ApprovalOrigin::TimeoutProceed)
                }
            }
        }
        Err(approval::ApprovalCheckError::Unattended { reason }) => {
            // Surface check already logged + fired the denied hook. Fail-closed
            // with the structured root cause instead of a generic "check failed".
            Err(super::rejection::ToolRejection::denied_unattended(
                name,
                reason.explain(),
            ))
        }
        Err(approval::ApprovalCheckError::UnattendedProceed { reason }) => {
            // Non-strict reason on an unattended surface with
            // `unattendedApprovalAction=proceed`. Auto-proceed, but record the
            // weaker-than-click origin (a strict reason never reaches here — it
            // is force-denied as `Unattended` above).
            app_warn!(
                "tool",
                "approval",
                "Tool '{}' auto-proceeded on unattended surface ({})",
                name,
                reason.explain()
            );
            Ok(approval::ApprovalOrigin::UnattendedProceed)
        }
        Err(e) => {
            app_warn!(
                "tool",
                "approval",
                "Tool approval check failed for '{}' ({}); blocking execution",
                name,
                e
            );
            Err(super::rejection::ToolRejection::approval_failed(
                name,
                e.to_string(),
            ))
        }
    }
}

pub async fn execute_tool_with_context(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
) -> anyhow::Result<String> {
    let start = std::time::Instant::now();

    // ── Tool visibility / policy gate ─────────────────────────────
    // Defense-in-depth: enforce the same effective visibility rules used for
    // schema generation and tool_search, so a tool cannot execute if it was
    // hidden by Agent filter, denied_tools, skill allowlist, or Plan Mode.
    if let Some(err) = ctx.tool_visibility_error(name).await {
        return Err(anyhow::anyhow!(err));
    }

    // ── PreToolUse hook (blocking; design §9.3/§9.4) ──────────────
    // Runs after the name-based hard-deny gate (visibility) and before the
    // permission engine. A hook deny short-circuits here; `updatedInput`
    // shadows `args` so every downstream gate (engine arg checks, plan-mode
    // path glob) and the tool itself see the patched value.
    //
    // Fire only on the OUTER call. Async-tool re-entry (`bypass_async_dispatch`,
    // set by the auto-background / explicit-background dispatch) already carries
    // the outer call's patched args and pre-approval, so re-firing here would
    // double a hook's side effects and re-apply an arg rewrite to its own output.
    let pre_skip_prompt: bool;
    let pre_force_prompt: bool;
    let patched_args_holder: Option<Value> = if ctx.bypass_async_dispatch {
        pre_skip_prompt = false;
        pre_force_prompt = false;
        None
    } else {
        match fire_pre_tool_use_hook(name, args, ctx).await {
            PreToolGate::Deny(reason) => {
                return Err(super::rejection::ToolRejection::denied_by_policy(
                    name, reason,
                ));
            }
            PreToolGate::Proceed {
                updated_input,
                skip_user_prompt,
                force_prompt,
            } => {
                pre_skip_prompt = skip_user_prompt;
                pre_force_prompt = force_prompt;
                if let Some(ref ui) = updated_input {
                    app_info!(
                        "hooks",
                        "dispatch",
                        "PreToolUse rewrote tool_input for '{}'",
                        name
                    );
                    // Surface the rewrite to the orchestrator so the UI, the
                    // persisted history, and the `PostToolUse` hook see the
                    // effective args — not the model's pre-rewrite ones. The
                    // sink is None for non-orchestrator callers
                    // (`execute_tool` direct path, async-job re-entry, slash
                    // commands), so this is free for them.
                    ctx.emit_effective_args(ui.clone()).await;
                }
                updated_input
            }
        }
    };
    // `args` now points at the patched value (if any) for the rest of the call.
    let args: &Value = patched_args_holder.as_ref().unwrap_or(args);
    // mac_control (#247): sanitize + preflight the (possibly hook-patched) args.
    let sanitized_args;
    let args = if name == TOOL_MAC_CONTROL {
        sanitized_args = crate::mac_control::sanitize_tool_args(args);
        if let Some(error) = crate::mac_control::preflight_tool_args(&sanitized_args) {
            return Err(anyhow::anyhow!(error));
        }
        &sanitized_args
    } else {
        args
    };

    let migrated_exec_args_holder = should_migrate_exec_process_mode_to_async_job(name, args, ctx)
        .then(|| migrate_exec_process_mode_to_async_job_args(args))
        .flatten();
    if let Some(ref migrated) = migrated_exec_args_holder {
        app_info!(
            "tool",
            "exec",
            "Migrating legacy exec background/yield_ms request to async job dispatch"
        );
        ctx.emit_effective_args(migrated.clone()).await;
    }
    let args: &Value = migrated_exec_args_holder.as_ref().unwrap_or(args);

    validate_async_background_contract(name, args)?;

    // Async-tool decision is computed up front but acted on after the
    // approval + plan-mode gates have run (so user-facing safeguards apply
    // once at submission time, then the work detaches).
    let async_decision = decide_async_path(name, args, ctx);

    // ── Tool-level approval gate ─────────────────────────────────
    // Run the unified permission engine. The engine consumes:
    //   plan_mode → YOLO → protected_paths → dangerous_commands → AllowAlways
    //   → session_mode preset → fallback Allow
    // and returns Allow / Ask / Deny. `exec` retains a separate command-level
    // gate further inside `tool_exec` for legacy AllowAlways prefix matching.
    //
    // SKILL.md reads are pre-authorized — skip the engine entirely so the
    // skill bootstrap never blocks on permission state.
    // Plan Mode `ask_tools` (`exec` per PlanAgentConfig) MUST hit the
    // permission engine so the user gets prompted for shell commands
    // during Planning — even when:
    //   - `auto_approve_tools=true` (IM channel auto-approve account
    //     convenience must NOT pierce Plan Mode's user-sovereignty
    //     contract), or
    //   - MCP server `autoApprove=true` + `trustLevel=Trusted` skips the
    //     ordinary tool approval gate, or
    //   - the tool is `exec` (which usually skips the engine for its own
    //     command-level prefix gate; in Plan Mode the engine's
    //     plan-mode-ask path takes precedence).
    // `external_pre_approved` only suppresses re-entry into the engine gate —
    // it does NOT pierce `exec`'s command-level audit (exec.rs reads
    // `auto_approve_tools` directly via `should_run_exec_command_gate`).
    // `auto_approve_tools` continues to mean "skip everything" for IM
    // auto-approve accounts and skill-triggered slash commands.
    let effective_auto_approve = ctx.local_auto_approve() || mcp_tool_auto_approves(name).await;
    let needs_engine = needs_permission_engine(name, args, ctx, effective_auto_approve);

    // F7 (IMYOLO-1 / DELETE-2): an IM auto-approve account / slash-skill skips the
    // engine gate entirely (`auto_approve_tools` → `needs_engine=false`). That
    // convenience stays opt-in, but a *strict* call slipping through silently
    // (dangerous command / protected path / mac-dangerous / plan-ask) must be
    // auditable. Probe the engine WITHOUT enforcing — only when the bypass is
    // specifically `auto_approve_tools` (NOT `external_pre_approved` async
    // re-entry, already gated at the outer dispatch; NOT MCP trust). Audit only:
    // the call still proceeds.
    if ctx.auto_approve_tools && !ctx.external_pre_approved && !needs_engine {
        if let crate::permission::Decision::Ask { reason } =
            resolve_tool_permission(name, args, ctx, super::is_internal_tool(name)).await
        {
            if reason.forbids_allow_always() {
                app_warn!(
                    "permission",
                    "auto_approve_bypass",
                    "Tool '{}' auto-approved (IM/skill), bypassing a STRICT approval ({:?}) — audit only, proceeding",
                    name,
                    reason
                );
            }
        }
    }
    // exec async approval-reorder state (B5/B6). Declared here — above the
    // engine gate — so the Plan-Mode-ask path below can record that exec was
    // already approved at the outer gate and suppress the reorder's second
    // prompt (review#3: plan-ask + async-eligible exec double-prompted).
    let mut exec_pre_approved = false;
    let mut tool_approval_origin: Option<approval::ApprovalOrigin> = None;
    if needs_engine {
        let decision =
            resolve_tool_permission(name, args, ctx, super::is_internal_tool(name)).await;
        match decision {
            crate::permission::Decision::Allow => {
                // Engine would allow without a prompt. A PreToolUse hook that
                // returned `ask`/`defer` still wants human confirmation — force
                // the prompt (no reason banner) so its request can't fail open.
                if pre_force_prompt {
                    tool_approval_origin =
                        Some(run_tool_approval(name, args, ctx, None, false, None).await?);
                }
            }
            crate::permission::Decision::Deny { reason } => {
                // PermissionDenied hook (observation): engine policy auto-denied
                // this tool (no user prompt — that decline path fires from the
                // approval layer instead).
                crate::hooks::fire_permission_denied(
                    ctx.session_id.as_deref(),
                    name,
                    "policy",
                    ctx.tool_call_id.as_deref(),
                );
                return Err(super::rejection::ToolRejection::denied_by_policy(
                    name, reason,
                ));
            }
            crate::permission::Decision::Ask { reason } => {
                // A hook `allow` may skip only a *soft* prompt — never a strict
                // one (protected path / dangerous command / mac-dangerous / Plan
                // ask), which always requires per-call human confirmation and
                // is exactly the boundary a hook must not be able to auto-bypass.
                let strict = reason.forbids_allow_always()
                    || matches!(reason, crate::permission::AskReason::PlanModeAsk);
                if pre_skip_prompt && !strict {
                    app_info!(
                        "hooks",
                        "dispatch",
                        "PreToolUse allow skipped soft approval prompt for '{}' (reason {:?})",
                        name,
                        reason
                    );
                } else {
                    let forbidden = reason.forbids_allow_always();
                    // mac_control (#247): the approval dialog steals focus; capture
                    // the target app before the prompt and restore it after a
                    // proceed (run_tool_approval returns Ok) so the action lands on
                    // the right app. On deny/error `?` returns early, leaving focus
                    // as-is — the same restore-on-proceed behavior #247 had before
                    // the hooks approval refactor.
                    let mac_control_focus_anchor =
                        capture_mac_control_approval_focus_anchor(name).await;
                    // F6: the prompt outcome IS the audit origin (User on approve,
                    // TimeoutProceed on a non-strict timeout-proceed).
                    tool_approval_origin = Some(
                        run_tool_approval(
                            name,
                            args,
                            ctx,
                            Some(approval::ApprovalReasonPayload::from(&reason)),
                            forbidden,
                            None,
                        )
                        .await?,
                    );
                    restore_mac_control_approval_focus_anchor(mac_control_focus_anchor).await;
                    // review#3: `exec` reaches the outer engine gate ONLY via
                    // Plan-Mode `ask_tools` (it is otherwise excluded). The user
                    // just approved that PlanModeAsk prompt; the async
                    // approval-reorder below would re-run the SAME engine →
                    // PlanModeAsk again → a redundant SECOND prompt for the
                    // identical command. Record the approval so the reorder (and
                    // the backgrounded inner gate) skip it — one prompt, not two.
                    // Gated on PlanModeAsk specifically so a future non-plan
                    // route to the engine can't accidentally bypass exec's
                    // command-level dangerous/protected audit.
                    if name == TOOL_EXEC
                        && matches!(reason, crate::permission::AskReason::PlanModeAsk)
                    {
                        // Origin already captured from the prompt above; just mark
                        // the reorder gate as satisfied so exec isn't re-prompted.
                        exec_pre_approved = true;
                    }
                }
            }
        }
    } else if pre_force_prompt && !is_skill_read(name, args) {
        // The engine gate was skipped (auto-approve / exec's own gate), but a
        // PreToolUse hook explicitly asked for confirmation — honor it rather
        // than letting the request through silently. SKILL.md reads are exempt
        // so skill bootstrap never blocks on a prompt.
        tool_approval_origin = Some(run_tool_approval(name, args, ctx, None, false, None).await?);
    }

    // ── exec async approval reorder (B5 / B6) ─────────────────────
    // `exec` is excluded from the outer engine gate above — its command-level
    // approval normally lives inside `tool_exec`. For an **auto-background-
    // eligible** exec call that would otherwise detach mid-flight, run that gate
    // HERE, *before* handing off to the spawner, so the approval wait is excluded
    // from the `auto_background_secs` + `max_job_secs` budgets (ASYNC-2) — those
    // timers only start inside the spawner call below. On approval,
    // `exec_pre_approved` rides into the spawned context so the inner gate in
    // `tool_exec` is skipped (one prompt, not two); on deny the rejection returns
    // WITHOUT spawning (the model gets a STOP, never a phantom job).
    //
    // R8 carve-out: **explicit `ImmediateBackground`** exec (`run_in_background`
    // / policy AlwaysBackground) does NOT reorder here — `should_run_exec_
    // reorder_gate` excludes it. Its command gate runs inside the background job
    // thread instead, so an attended approval parks the job at `AwaitingApproval`
    // and resolves asynchronously: the model receives the job id immediately and
    // a denial settles the job terminal (DeniedByUser→Failed) via injection,
    // rather than blocking the foreground turn. This deliberately supersedes
    // ASYNC-1 for the explicit-background path (the acceptance requires a denied
    // background exec to terminate as a job, not vanish). Non-exec async tools
    // still ran the engine gate above, so they reach the spawn branches approved.
    if should_run_exec_reorder_gate(name, async_decision, exec_pre_approved, ctx) {
        let command = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
        let session_cwd = args
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|raw| ctx.resolve_path(raw))
            .unwrap_or_else(|| ctx.default_cwd());
        let origin = exec::resolve_exec_command_approval(command, args, ctx, &session_cwd).await?;
        exec_pre_approved = true;
        tool_approval_origin = Some(origin);
    }

    // F6 (TIMEOUT-2): every backgrounded job's `approval_origin` column records
    // HOW it was authorized. The engine gate / exec reorder set it for prompted,
    // exec, and policy-allowed-with-force-prompt calls; fill the remaining bypass
    // cases so no spawned job carries a null origin — async re-entry
    // (external_pre_approved), IM/skill auto-approve (effective_auto_approve), or
    // a silent engine Allow (policy/yolo). Only the async spawn branches below
    // consume this; sync execution ignores it.
    if tool_approval_origin.is_none() {
        tool_approval_origin = Some(if ctx.external_pre_approved {
            approval::ApprovalOrigin::ExternalPreApproved
        } else if effective_auto_approve {
            approval::ApprovalOrigin::AutoApprove
        } else {
            exec::policy_allow_origin(ctx)
        });
    }

    // Log tool execution start
    if let Some(logger) = crate::get_logger() {
        let args_preview = {
            let s = args.to_string();
            if s.len() > 500 {
                format!("{}...", crate::truncate_utf8(&s, 500))
            } else {
                s
            }
        };
        logger.log(
            "info",
            "tool",
            &format!("tools::{}", name),
            &format!("Tool '{}' started", name),
            Some(serde_json::json!({"args": args_preview}).to_string()),
            None,
            None,
        );
    }

    // ── Plan Mode path-based permission check ─────────────────────
    // When plan_mode_allow_paths is set, write/edit/apply_patch tools check
    // the target file path and block non-plan-file operations.
    if !ctx.plan_mode_allow_paths.is_empty() {
        let is_path_aware = matches!(name, TOOL_WRITE | TOOL_EDIT | TOOL_APPLY_PATCH);
        if is_path_aware {
            let target_path = args
                .get("file_path")
                .or_else(|| args.get("path"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !target_path.is_empty() && !crate::plan::is_plan_mode_path_allowed(target_path) {
                return Err(anyhow::anyhow!(
                    "Plan Mode restriction: cannot modify '{}'. During planning, only plan files \
                     (under .hope-agent/plans/) can be edited. Use submit_plan to finalize the plan.",
                    target_path
                ));
            }
        }
    }

    // Short-circuit: explicit / policy-forced background spawn. The synthetic
    // job_id is returned to the LLM as the tool result; the real work runs on
    // a dedicated OS thread via `async_jobs::spawn_explicit_job`.
    if let AsyncDecision::ImmediateBackground(origin) = async_decision {
        let mut spawn_ctx = ctx.clone();
        // R8: for explicit background exec the reorder gate above is SKIPPED, so
        // `exec_pre_approved` is normally false here — the command gate runs
        // inside the background runtime, where an attended approval parks the job
        // at `AwaitingApproval` (see `async_jobs::approval_bridge`). It is only
        // true when a prior engine prompt this turn already approved the command
        // (Plan-Mode-ask path), in which case the inner gate is correctly skipped.
        // `approval_origin` is the spawn-time audit value (a placeholder when the
        // gate is deferred); the bridge corrects it to the real decision on resume.
        spawn_ctx.exec_pre_approved = exec_pre_approved;
        spawn_ctx.approval_origin = tool_approval_origin;
        let job_id_override = spawn_ctx.async_job_id_override.take();
        let raw = if let Some(job_id) = job_id_override {
            async_jobs::JobManager::spawn_tool_with_id(
                name,
                args.clone(),
                spawn_ctx,
                origin,
                job_id,
            )?
        } else {
            async_jobs::JobManager::spawn_tool(name, args.clone(), spawn_ctx, origin)?
        };
        // Skip the disk-persist tail since the synthetic JSON is small and
        // mirrors the same shape `job_status` returns later.
        return Ok(raw);
    }

    // Auto-background path: detour through the budget-aware helper which
    // re-enters this function with `bypass_async_dispatch = true`, runs the
    // dispatch on an OS thread, and either returns the inline result or
    // detaches into a job and returns a synthetic.
    if matches!(async_decision, AsyncDecision::AutoBackgroundEligible) {
        let auto_bg_secs = crate::config::cached_config()
            .async_tools
            .auto_background_secs;
        let mut inner_ctx = ctx.clone();
        inner_ctx.bypass_async_dispatch = true;
        inner_ctx.suppress_global_tool_timeout = true;
        // The engine gate either ran (for non-exec tools) or was deliberately
        // skipped (`exec` is always excluded from the outer engine gate and
        // runs its command-level audit instead). Tell the recursive inner
        // dispatch "engine already handled" so it doesn't double-prompt the
        // user — but **do not** flip `auto_approve_tools`, which would also
        // bypass `exec`'s command-level dangerous/edit audit and let any
        // shell command run silently as long as it's async-eligible.
        inner_ctx.external_pre_approved = true;
        // For exec the command gate already ran above (before the budget timer
        // starts); carry the verdict so the inner re-dispatch doesn't prompt
        // again on the background OS thread, plus the audit origin.
        inner_ctx.exec_pre_approved = exec_pre_approved;
        inner_ctx.approval_origin = tool_approval_origin;
        let raw = async_jobs::JobManager::dispatch_tool_with_auto_background(
            name,
            args,
            &inner_ctx,
            auto_bg_secs,
        )
        .await?;
        // The inner worker suppresses generic disk persistence so detached jobs
        // can spool their raw output into the async output-file. If the worker
        // finished within the foreground budget, persist large inline output at
        // this outer layer before returning it to the model.
        return maybe_persist_large_tool_result(name, raw, ctx);
    }

    // ── Conditional skill activation (`paths:` frontmatter) ──────
    // Scan args for file paths the tool is about to touch, then light up
    // any `paths:` skills whose patterns match. The skill catalog in the
    // *next* system-prompt build will include them; we bump skill_version
    // so the 30s skill cache doesn't swallow this change.
    if ctx.session_id.is_some() {
        maybe_activate_conditional_skills(name, args, ctx);
    }

    let hard_timeout = tool_timeout(ctx);
    let timeout_ctx = hard_timeout.map(|_| {
        let mut timeout_ctx = ctx.clone();
        let token = ctx
            .cancellation_token
            .as_ref()
            .map(CancellationToken::child_token)
            .unwrap_or_default();
        timeout_ctx.cancellation_token = Some(token);
        timeout_ctx
    });
    let dispatch_ctx = timeout_ctx.as_ref().unwrap_or(ctx);
    let timeout_cancel_token = dispatch_ctx.cancellation_token.clone();

    let dispatch = async {
        match name {
            TOOL_EXEC => exec::tool_exec(args, dispatch_ctx).await,
            TOOL_PROCESS => process::tool_process(args).await,
            TOOL_READ | "read_file" => read::tool_read_file(args, dispatch_ctx).await,
            TOOL_WRITE | "write_file" => write::tool_write_file(args, dispatch_ctx).await,
            TOOL_EDIT | "patch_file" => edit::tool_edit(args, dispatch_ctx).await,
            TOOL_LS | "list_dir" => ls::tool_ls(args, dispatch_ctx).await,
            TOOL_LSP => lsp::tool_lsp(args, dispatch_ctx).await,
            TOOL_GREP => grep::tool_grep(args, dispatch_ctx).await,
            TOOL_FIND => find::tool_find(args, dispatch_ctx).await,
            TOOL_APPLY_PATCH => apply_patch::tool_apply_patch(args, dispatch_ctx).await,
            TOOL_WEB_SEARCH => web_search::tool_web_search(args, dispatch_ctx).await,
            TOOL_WEB_FETCH => web_fetch::tool_web_fetch(args).await,
            TOOL_SAVE_MEMORY => memory::tool_save_memory(args, dispatch_ctx).await,
            TOOL_RECALL_MEMORY => memory::tool_recall_memory(args, dispatch_ctx).await,
            TOOL_UPDATE_MEMORY => memory::tool_update_memory(args, dispatch_ctx).await,
            TOOL_DELETE_MEMORY => memory::tool_delete_memory(args, dispatch_ctx).await,
            TOOL_UPDATE_CORE_MEMORY => memory::tool_update_core_memory(args, dispatch_ctx).await,
            TOOL_CORE_MEMORY => core_memory::tool_core_memory(args, dispatch_ctx).await,
            TOOL_PROJECT_MEMORY => project_memory::tool_project_memory(args, dispatch_ctx).await,
            TOOL_MANAGE_CRON => cron::tool_manage_cron(args, dispatch_ctx).await,
            TOOL_BROWSER => browser::tool_browser(args, dispatch_ctx).await,
            TOOL_MAC_CONTROL => mac_control::tool_mac_control(args).await,
            TOOL_SEND_NOTIFICATION => {
                notification::tool_send_notification(args, dispatch_ctx).await
            }
            TOOL_SUBAGENT => subagent::tool_subagent(args, dispatch_ctx).await,
            TOOL_TEAM => team::tool_team(args, dispatch_ctx).await,
            TOOL_ACP_SPAWN => acp_spawn::tool_acp_spawn(args, dispatch_ctx).await,
            TOOL_WORKFLOW => workflow_tool::tool_workflow(args, dispatch_ctx).await,
            TOOL_MEMORY_GET => memory::tool_memory_get(args, dispatch_ctx).await,
            // Knowledge base (note_*) tools.
            TOOL_NOTE_CREATE => note::tool_note_create(args, dispatch_ctx).await,
            TOOL_NOTE_READ => note::tool_note_read(args, dispatch_ctx).await,
            TOOL_NOTE_UPDATE => note::tool_note_update(args, dispatch_ctx).await,
            TOOL_NOTE_PATCH => note::tool_note_patch(args, dispatch_ctx).await,
            TOOL_NOTE_APPEND => note::tool_note_append(args, dispatch_ctx).await,
            TOOL_NOTE_DELETE => note::tool_note_delete(args, dispatch_ctx).await,
            TOOL_NOTE_SEARCH => note::tool_note_search(args, dispatch_ctx).await,
            TOOL_NOTE_LINK => note::tool_note_link(args, dispatch_ctx).await,
            TOOL_NOTE_BACKLINKS => note::tool_note_backlinks(args, dispatch_ctx).await,
            TOOL_NOTE_BY_TAG => note::tool_note_by_tag(args, dispatch_ctx).await,
            TOOL_NOTE_TAGS => note::tool_note_tags(args, dispatch_ctx).await,
            TOOL_NOTE_RENAME | TOOL_NOTE_MOVE => note::tool_note_rename(args, dispatch_ctx).await,
            TOOL_NOTE_SET_FRONTMATTER => note::tool_note_set_frontmatter(args, dispatch_ctx).await,
            TOOL_NOTE_ASSIGN_BLOCK => note::tool_note_assign_block(args, dispatch_ctx).await,
            TOOL_NOTE_BROKEN_LINKS => note::tool_note_broken_links(args, dispatch_ctx).await,
            TOOL_NOTE_ORPHANS => note::tool_note_orphans(args, dispatch_ctx).await,
            TOOL_NOTE_GRAPH => note::tool_note_graph(args, dispatch_ctx).await,
            TOOL_NOTE_SIMILAR => note::tool_note_similar(args, dispatch_ctx).await,
            TOOL_NOTE_RELATED => note::tool_note_related(args, dispatch_ctx).await,
            TOOL_NOTE_SUGGEST_LINKS => note::tool_note_suggest_links(args, dispatch_ctx).await,
            TOOL_NOTE_DISTILL => note::tool_note_distill(args, dispatch_ctx).await,
            TOOL_NOTE_MOC => note::tool_note_moc(args, dispatch_ctx).await,
            TOOL_KNOWLEDGE_RECALL => note::tool_knowledge_recall(args, dispatch_ctx).await,
            TOOL_SESSION_TO_NOTE => note::tool_session_to_note(args, dispatch_ctx).await,
            TOOL_AGENTS_LIST => agents::tool_agents_list(args).await,
            TOOL_SESSIONS_LIST => sessions::tool_sessions_list(args).await,
            TOOL_SESSION_STATUS => sessions::tool_session_status(args).await,
            TOOL_SESSIONS_SEARCH => sessions::tool_sessions_search(args, dispatch_ctx).await,
            TOOL_SESSIONS_HISTORY => sessions::tool_sessions_history(args).await,
            TOOL_SESSIONS_SEND => Box::pin(sessions::tool_sessions_send(args, dispatch_ctx)).await,
            TOOL_IMAGE => image::tool_image(args, dispatch_ctx).await,
            TOOL_IMAGE_GENERATE => image_generate::tool_image_generate(args, dispatch_ctx).await,
            TOOL_ISSUE_REPORT => issue_report::tool_issue_report(args, dispatch_ctx).await,
            TOOL_PDF => pdf::tool_pdf(args).await,
            TOOL_CANVAS => canvas::tool_canvas(args, dispatch_ctx).await,
            TOOL_DESIGN => design::tool_design(args, dispatch_ctx).await,
            TOOL_ARTIFACT => artifact::tool_artifact(args, dispatch_ctx).await,
            TOOL_GET_WEATHER => weather::tool_get_weather(args).await,
            TOOL_ASK_USER_QUESTION => {
                Ok(ask_user_question::execute(args, dispatch_ctx.session_id.as_deref()).await)
            }
            TOOL_ENTER_PLAN_MODE => {
                Ok(enter_plan_mode::execute(args, dispatch_ctx.session_id.as_deref()).await)
            }
            TOOL_SUBMIT_PLAN => {
                Ok(submit_plan::execute(args, dispatch_ctx.session_id.as_deref()).await)
            }
            TOOL_TASK_CREATE => {
                Ok(task::tool_task_create(args, dispatch_ctx.session_id.as_deref()).await)
            }
            TOOL_TASK_UPDATE => {
                Ok(task::tool_task_update(args, dispatch_ctx.session_id.as_deref()).await)
            }
            TOOL_TASK_LIST => {
                Ok(task::tool_task_list(args, dispatch_ctx.session_id.as_deref()).await)
            }
            TOOL_GOAL_STATUS => Ok(goal::tool_goal_status(args, dispatch_ctx).await),
            TOOL_GOAL_PREPARE_CONTRACT => {
                Ok(goal::tool_goal_prepare_contract(args, dispatch_ctx).await)
            }
            TOOL_GOAL_CHECKPOINT => Ok(goal::tool_goal_checkpoint(args, dispatch_ctx).await),
            TOOL_GOAL_RECORD_EVIDENCE => {
                Ok(goal::tool_goal_record_evidence(args, dispatch_ctx).await)
            }
            TOOL_GOAL_EVALUATE => Ok(goal::tool_goal_evaluate(args, dispatch_ctx).await),
            TOOL_GOAL_FINISH_REQUEST => {
                Ok(goal::tool_goal_finish_request(args, dispatch_ctx).await)
            }
            TOOL_GOAL_BLOCK_REQUEST => Ok(goal::tool_goal_block_request(args, dispatch_ctx).await),
            TOOL_LOOP_STATUS => Ok(loop_tool::tool_loop_status(args, dispatch_ctx).await),
            TOOL_LOOP_RESCHEDULE => Ok(loop_tool::tool_loop_reschedule(args, dispatch_ctx).await),
            TOOL_LOOP_STOP => Ok(loop_tool::tool_loop_stop(args, dispatch_ctx).await),
            TOOL_LOOP_RECORD_PROGRESS => {
                Ok(loop_tool::tool_loop_record_progress(args, dispatch_ctx).await)
            }
            TOOL_LOOP_WATCH => Ok(loop_tool::tool_loop_watch(args, dispatch_ctx).await),
            TOOL_LOOP_UNWATCH => Ok(loop_tool::tool_loop_unwatch(args, dispatch_ctx).await),
            super::TOOL_APP_UPDATE => app_update::tool_app_update(args, dispatch_ctx).await,
            TOOL_JOB_STATUS => {
                job_status::tool_job_status(args, dispatch_ctx.session_id.as_deref()).await
            }
            super::TOOL_SCHEDULE_WAKEUP => {
                schedule_wakeup::tool_schedule_wakeup(args, dispatch_ctx).await
            }
            TOOL_RUNTIME_CANCEL => runtime_cancel::tool_runtime_cancel(args).await,
            super::TOOL_TOOL_SEARCH => super::tool_search::tool_search(args, dispatch_ctx).await,
            super::TOOL_PEEK_SESSIONS => {
                crate::awareness::run_peek_sessions(args, dispatch_ctx.session_id.as_deref())
                    .map_err(|e| anyhow::anyhow!(e))
            }
            TOOL_GET_SETTINGS => settings::tool_get_settings(args).await,
            TOOL_UPDATE_SETTINGS => settings::tool_update_settings(args).await,
            TOOL_LIST_SETTINGS_BACKUPS => settings::tool_list_settings_backups(args).await,
            TOOL_RESTORE_SETTINGS_BACKUP => settings::tool_restore_settings_backup(args).await,
            TOOL_SEND_ATTACHMENT => send_attachment::tool_send_attachment(args, dispatch_ctx).await,
            super::TOOL_SKILL => skill::tool_skill(args, dispatch_ctx).await,
            super::TOOL_MCP_RESOURCE => crate::mcp::resources::tool_mcp_resource(args).await,
            super::TOOL_MCP_PROMPT => crate::mcp::prompts::tool_mcp_prompt(args).await,
            super::feishu::TOOL_DOCX_CREATE => super::feishu::docx::execute_create(args).await,
            super::feishu::TOOL_DOCX_GET_BLOCKS => {
                super::feishu::docx::execute_get_blocks(args).await
            }
            super::feishu::TOOL_DOCX_APPEND_BLOCK => {
                super::feishu::docx::execute_append_block(args).await
            }
            super::feishu::TOOL_DOCX_UPDATE_BLOCK_TEXT => {
                super::feishu::docx::execute_update_block_text(args).await
            }
            super::feishu::TOOL_BITABLE_LIST_RECORDS => {
                super::feishu::bitable::execute_list_records(args).await
            }
            super::feishu::TOOL_BITABLE_SEARCH_RECORDS => {
                super::feishu::bitable::execute_search_records(args).await
            }
            super::feishu::TOOL_BITABLE_CREATE_RECORD => {
                super::feishu::bitable::execute_create_record(args).await
            }
            super::feishu::TOOL_BITABLE_BATCH_UPDATE_RECORDS => {
                super::feishu::bitable::execute_batch_update_records(args).await
            }
            super::feishu::TOOL_BITABLE_LIST_VIEWS => {
                super::feishu::bitable::execute_list_views(args).await
            }
            super::feishu::TOOL_BITABLE_GET_VIEW => {
                super::feishu::bitable::execute_get_view(args).await
            }
            super::feishu::TOOL_BITABLE_LIST_DASHBOARDS => {
                super::feishu::bitable::execute_list_dashboards(args).await
            }
            super::feishu::TOOL_DRIVE_LIST_FILES => {
                super::feishu::drive::execute_list_files(args).await
            }
            super::feishu::TOOL_DRIVE_UPLOAD_MEDIA => {
                super::feishu::drive::execute_upload_media(args).await
            }
            super::feishu::TOOL_DRIVE_DOWNLOAD_MEDIA => {
                super::feishu::drive::execute_download_media(args).await
            }
            super::feishu::TOOL_WIKI_GET_NODE => super::feishu::wiki::execute_get_node(args).await,
            super::feishu::TOOL_APPROVAL_CREATE_INSTANCE => {
                super::feishu::approval::execute_create_instance(args).await
            }
            super::feishu::TOOL_APPROVAL_GET_INSTANCE => {
                super::feishu::approval::execute_get_instance(args).await
            }
            super::feishu::TOOL_APPROVAL_CANCEL_INSTANCE => {
                super::feishu::approval::execute_cancel_instance(args).await
            }
            super::feishu::TOOL_APPROVAL_LIST_INSTANCES => {
                super::feishu::approval::execute_list_instances(args).await
            }
            super::feishu::TOOL_APPROVAL_SUBSCRIBE => {
                super::feishu::approval::execute_subscribe(args).await
            }
            super::feishu::TOOL_CALENDAR_LIST => super::feishu::calendar::execute_list(args).await,
            super::feishu::TOOL_CALENDAR_CREATE_EVENT => {
                super::feishu::calendar::execute_create_event(args).await
            }
            super::feishu::TOOL_CALENDAR_LIST_EVENTS => {
                super::feishu::calendar::execute_list_events(args).await
            }
            super::feishu::TOOL_CALENDAR_UPDATE_EVENT => {
                super::feishu::calendar::execute_update_event(args).await
            }
            super::feishu::TOOL_CALENDAR_DELETE_EVENT => {
                super::feishu::calendar::execute_delete_event(args).await
            }
            super::feishu::TOOL_CALENDAR_ATTENDEES_CREATE => {
                super::feishu::calendar::execute_attendees_create(args).await
            }
            super::feishu::TOOL_CONTACT_GET_USER => {
                super::feishu::contact::execute_get_user(args).await
            }
            super::feishu::TOOL_CONTACT_BATCH_GET_USERS => {
                super::feishu::contact::execute_batch_get_users(args).await
            }
            super::feishu::TOOL_CONTACT_GET_DEPARTMENT => {
                super::feishu::contact::execute_get_department(args).await
            }
            super::feishu::TOOL_CONTACT_SEARCH_USERS_BY_DEPARTMENT => {
                super::feishu::contact::execute_search_users_by_department(args).await
            }
            super::feishu::TOOL_HIRE_LIST_JOBS => {
                super::feishu::hire::execute_list_jobs(args).await
            }
            super::feishu::TOOL_HIRE_GET_JOB => super::feishu::hire::execute_get_job(args).await,
            super::feishu::TOOL_HIRE_LIST_TALENTS => {
                super::feishu::hire::execute_list_talents(args).await
            }
            super::feishu::TOOL_HIRE_GET_TALENT => {
                super::feishu::hire::execute_get_talent(args).await
            }
            super::feishu::TOOL_HIRE_LIST_APPLICATIONS => {
                super::feishu::hire::execute_list_applications(args).await
            }
            // MCP-sourced tools all share the `mcp__<server>__<tool>`
            // prefix; dispatch them through the dedicated subsystem.
            n if crate::mcp::catalog::is_mcp_tool_name(n) => {
                crate::mcp::invoke::call_tool(n, args, dispatch_ctx).await
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        }
    };

    let mut dispatch = Box::pin(dispatch);
    let result = if let Some(hard_timeout) = hard_timeout {
        match timeout(hard_timeout, &mut dispatch).await {
            Ok(inner) => inner,
            Err(_elapsed) => {
                if let Some(token) = &timeout_cancel_token {
                    token.cancel();
                }
                let _ = timeout(TOOL_TIMEOUT_CLEANUP_GRACE, &mut dispatch).await;
                app_error!(
                    "tool",
                    "execution",
                    "Tool '{}' timed out after {}s — forcefully cancelled",
                    name,
                    hard_timeout.as_secs()
                );
                Err(anyhow::anyhow!(
                    "Tool '{}' execution timed out after {}s. The operation was cancelled. \
                     This may be caused by network issues, an unresponsive API, or a slow provider. \
                     Please check your network connection and provider configuration, \
                     or increase toolTimeout in Settings > System.",
                    name,
                    hard_timeout.as_secs()
                ))
            }
        }
    } else {
        // timeout disabled (toolTimeout = 0)
        dispatch.await
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    // Log tool execution result
    if let Some(logger) = crate::get_logger() {
        match &result {
            Ok(output) => {
                let output_preview = if output.len() > 300 {
                    format!("{}...", crate::truncate_utf8(output, 300))
                } else {
                    output.clone()
                };
                logger.log("info", "tool", &format!("tools::{}", name),
                    &format!("Tool '{}' completed in {}ms", name, duration_ms),
                    Some(serde_json::json!({"duration_ms": duration_ms, "output_preview": output_preview}).to_string()),
                    None, None);
            }
            Err(e) => {
                logger.log(
                    "error",
                    "tool",
                    &format!("tools::{}", name),
                    &format!("Tool '{}' failed in {}ms: {}", name, duration_ms, e),
                    Some(
                        serde_json::json!({"duration_ms": duration_ms, "error": e.to_string()})
                            .to_string(),
                    ),
                    None,
                    None,
                );
            }
        }
    }

    match result {
        Ok(output) => {
            // Smart mode only: remember a file the agent SUCCESSFULLY edited so
            // re-edits in this session skip the prompt. Gated on success (a
            // failed write/edit/apply_patch returns Err and is excluded) and on
            // Smart mode (Default/YOLO/auto-approve edits must NOT leak forward
            // into Smart's trusted set — only edits actually vetted under Smart
            // count). Plan-mode-blocked edits returned Err before dispatch, so
            // they never reach here either.
            if ctx.session_mode == crate::permission::SessionMode::Smart {
                record_smart_session_edits(name, args, ctx);
            }
            maybe_persist_large_tool_result(name, output, ctx)
        }
        other => other,
    }
}

// ── Result disk persistence ──────────────────────────────────────
// In normal sessions, inline image markers are first materialized into managed
// `__IMAGE_FILE__` references. Other large results are written to disk and
// replaced with a preview plus a path the model can `read`.
fn maybe_persist_large_tool_result(
    name: &str,
    output: String,
    ctx: &ToolExecContext,
) -> anyhow::Result<String> {
    // E3 (INCOG-5): incognito sessions never spill tool output to disk — keep it
    // inline (in-memory) so the burn-on-close leaves no `tool_results/` trace.
    if ctx.suppress_result_disk_persistence || ctx.incognito {
        return Ok(output);
    }
    if crate::tools::image_markers::has_valid_image_markers(&output) {
        match crate::tools::image_markers::materialize_base64_image_markers(
            &output,
            ctx.session_id.as_deref(),
        ) {
            Ok(Some(materialized)) => {
                app_info!(
                    "tool",
                    "disk_persist",
                    "Tool '{}' result {}B materialized image markers for provider vision",
                    name,
                    output.len()
                );
                return Ok(materialized);
            }
            Ok(None) => {
                app_info!(
                    "tool",
                    "disk_persist",
                    "Tool '{}' result {}B contains valid image file marker; preserving provider vision",
                    name,
                    output.len()
                );
            }
            Err(e) => {
                app_warn!(
                    "tool",
                    "disk_persist",
                    "Failed to materialize image markers for '{}': {}; preserving inline for provider vision",
                    name,
                    e
                );
            }
        }
        return Ok(output);
    }
    if !should_persist_large_result(&output) {
        return Ok(output);
    }
    match persist_large_result(&output, ctx.session_id.as_deref(), name) {
        Ok(path) => {
            app_info!(
                "tool",
                "disk_persist",
                "Tool '{}' result {}B persisted to {}",
                name,
                output.len(),
                path
            );
            Ok(build_persisted_large_result_preview(&output, &path))
        }
        Err(e) => {
            // Fall back to returning the full result if persistence fails.
            app_warn!(
                "tool",
                "disk_persist",
                "Failed to persist large result for '{}': {}",
                name,
                e
            );
            Ok(output)
        }
    }
}

// ── Disk Persistence Helpers ─────────────────────────────────────

/// Load the disk persistence threshold from config.json, defaulting to 50KB.
/// Returns 0 to disable (never persist).
fn disk_persist_threshold() -> usize {
    crate::config::cached_config()
        .tool_result_disk_threshold
        .unwrap_or(50_000)
}

fn should_persist_large_result(output: &str) -> bool {
    let threshold = disk_persist_threshold();
    threshold > 0 && output.len() > threshold
}

fn build_persisted_large_result_preview(output: &str, path: &str) -> String {
    let (media_header, output_body) = split_media_items_header(output);

    if crate::tools::image_markers::contains_image_marker(output_body) {
        let preview = format!(
            "[Large tool result ({total}B) saved to: {path}]\n\
             [Inline preview omitted because the result contains image marker data that must not be truncated.]\n\
             [Use read tool with this path to access full content]",
            total = output.len(),
        );
        return format!("{media_header}{preview}");
    }

    let head = crate::truncate_utf8(output_body, 2000);
    let tail = crate::util::truncate_utf8_tail(output_body, 1000);
    let omitted = output_body.len().saturating_sub(head.len() + tail.len());
    let preview = format!(
        "{head}\n\n[...{omitted} bytes omitted...]\n\n{tail}\n\n\
         [Full result ({total}B) saved to: {path}]\n\
         [Use read tool with this path to access full content]",
        total = output.len(),
    );
    format!("{media_header}{preview}")
}

fn split_media_items_header(output: &str) -> (&str, &str) {
    let Some(rest) = output.strip_prefix(crate::agent::MEDIA_ITEMS_PREFIX) else {
        return ("", output);
    };
    let Some(newline_idx) = rest.find('\n') else {
        return ("", output);
    };
    let split_at = crate::agent::MEDIA_ITEMS_PREFIX.len() + newline_idx + 1;
    (&output[..split_at], &output[split_at..])
}

/// Write a large tool result to disk and return the file path.
/// Extract file paths from tool args so `paths:` skill activation can see
/// what the session is touching. Only the path-aware tools (read/write/edit/
/// ls/apply_patch) are scanned; other tools return an empty Vec.
fn extract_touched_paths(tool_name: &str, args: &Value) -> Vec<String> {
    fn as_str(v: Option<&Value>) -> Option<String> {
        v.and_then(|x| x.as_str()).map(|s| s.to_string())
    }

    match tool_name {
        TOOL_READ | "read_file" | TOOL_WRITE | "write_file" | TOOL_EDIT | "patch_file"
        | TOOL_LS | "list_dir" => {
            let mut out = Vec::new();
            if let Some(p) = as_str(args.get("path")) {
                out.push(p);
            }
            if let Some(p) = as_str(args.get("file_path")) {
                out.push(p);
            }
            out
        }
        TOOL_APPLY_PATCH => {
            // Patch format uses `*** Update File: <path>` / `*** Add File: <path>`.
            let patch = match args
                .get("input")
                .or_else(|| args.get("patch"))
                .and_then(|v| v.as_str())
            {
                Some(s) => s,
                None => return Vec::new(),
            };
            let mut out = Vec::new();
            for line in patch.lines() {
                let trimmed = line.trim_start();
                for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
                    if let Some(path) = trimmed.strip_prefix(marker) {
                        out.push(path.trim().to_string());
                    }
                }
            }
            out
        }
        _ => Vec::new(),
    }
}

/// Cached answer to "are there any `paths:` skills in the current catalog?"
/// Keyed on `skill_cache_version()` so it invalidates together with the rest
/// of the skill system when discovery changes. The fast-path lets us skip
/// the filesystem-scanning `get_invocable_skills` call on every file op when
/// no skill actually declares `paths:` (the common case).
static HAS_PATHS_SKILLS_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<(u64, bool)>>> =
    std::sync::OnceLock::new();

fn any_paths_skills(cfg: &crate::config::AppConfig) -> bool {
    let current_version = crate::skills::skill_cache_version();
    let cache = HAS_PATHS_SKILLS_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((v, b)) = *guard {
            if v == current_version {
                return b;
            }
        }
    }

    let catalog = crate::skills::get_invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);
    let has_any = catalog
        .iter()
        .any(|s| s.paths.as_ref().map(|p| !p.is_empty()).unwrap_or(false));

    if let Ok(mut guard) = cache.lock() {
        *guard = Some((current_version, has_any));
    }
    has_any
}

fn maybe_activate_conditional_skills(name: &str, args: &Value, ctx: &ToolExecContext) {
    let cfg = crate::config::cached_config();
    if !cfg.conditional_skills_enabled {
        return;
    }
    let session_id = match ctx.session_id.as_deref() {
        Some(s) => s,
        None => return,
    };
    let paths = extract_touched_paths(name, args);
    if paths.is_empty() {
        return;
    }
    // Fast path: if no skill in the catalog declares `paths:`, skip the
    // full discovery pass. Cache invalidates with skill_cache_version.
    if !any_paths_skills(&cfg) {
        return;
    }
    let cwd = ctx.default_path();
    let catalog = crate::skills::get_invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);
    let activated = crate::skills::activate_skills_for_paths(session_id, &paths, cwd, &catalog);
    if !activated.is_empty() {
        crate::skills::bump_skill_version();
        crate::app_info!(
            "skill",
            "activation",
            "Activated conditional skills {:?} in session {}",
            activated,
            session_id
        );
    }
}

/// Recursively delete a session's large-tool-result spill directory
/// (`~/.hope-agent/tool_results/<session_id>/`). Called by the session cleanup
/// watcher on **purge** (incognito burn-on-close) as a backstop — incognito
/// sessions never write here in the first place (E3 keeps results inline), but
/// this clears anything written before the incognito flag was visible or by a
/// prior build. Best-effort: a missing dir or a remove error is logged, never
/// propagated. Epic E (INCOG-5).
pub fn purge_tool_results_for_session(session_id: &str) {
    if session_id.is_empty() {
        return;
    }
    let dir = match crate::paths::root_dir() {
        Ok(root) => root
            .join("tool_results")
            .join(crate::paths::sanitize_path_segment(session_id)),
        Err(_) => return,
    };
    if !dir.exists() {
        return;
    }
    if let Err(e) = std::fs::remove_dir_all(&dir) {
        if e.kind() != std::io::ErrorKind::NotFound {
            app_warn!(
                "tool",
                "purge_tool_results",
                "failed to purge tool_results dir for session {}: {}",
                session_id,
                e
            );
        }
    }
}

fn persist_large_result(
    content: &str,
    session_id: Option<&str>,
    tool_name: &str,
) -> anyhow::Result<String> {
    let base_dir =
        crate::paths::root_dir()?
            .join("tool_results")
            .join(crate::paths::sanitize_path_segment(
                session_id.unwrap_or("_global"),
            ));
    std::fs::create_dir_all(&base_dir)?;

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let filename = format!("{tool_name}_{ts}.txt");
    let path = base_dir.join(&filename);
    std::fs::write(&path, content)?;

    Ok(path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::{
        build_persisted_large_result_preview, decide_async_path_with_config,
        exec_process_background_mode, execute_tool_with_context, maybe_persist_large_tool_result,
        mcp_server_auto_approves_config, migrate_exec_process_mode_to_async_job_args,
        needs_permission_engine, should_migrate_exec_process_mode_to_async_job_with_config,
        should_run_exec_reorder_gate, tool_timeout, validate_async_background_contract,
        AsyncDecision, JobOrigin, SessionDbHandle, ToolExecContext,
    };
    use crate::agent_config::AsyncToolPolicy;
    use crate::mcp::{McpServerConfig, McpTransportSpec, McpTrustLevel};
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    use crate::tools::image_markers::IMAGE_FILE_PREFIX;
    use base64::Engine as _;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::io::Cursor;
    use std::path::Path;
    use std::sync::Arc;

    fn mcp_cfg(auto_approve: bool, trust_level: McpTrustLevel) -> McpServerConfig {
        McpServerConfig {
            id: "id-alpha".into(),
            name: "alpha".into(),
            enabled: true,
            transport: McpTransportSpec::Stdio {
                command: "true".into(),
                args: vec![],
                cwd: None,
            },
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            oauth: None,
            allowed_tools: vec![],
            denied_tools: vec![],
            connect_timeout_secs: 30,
            call_timeout_secs: 120,
            health_check_interval_secs: 60,
            max_concurrent_calls: 4,
            auto_approve,
            trust_level,
            eager: false,
            deferred_tools: false,
            project_paths: vec![],
            description: None,
            icon: None,
            created_at: 0,
            updated_at: 0,
            trust_acknowledged_at: None,
        }
    }

    #[test]
    fn default_path_prefers_session_working_dir_over_agent_home() {
        let ctx = ToolExecContext {
            home_dir: Some("/tmp/hope-agent/coder-home".to_string()),
            session_working_dir: Some("/tmp/projects/demo".to_string()),
            ..ToolExecContext::default()
        };

        assert_eq!(ctx.default_path(), "/tmp/projects/demo");
    }

    #[test]
    fn background_async_jobs_suppress_global_tool_timeout() {
        let ctx = ToolExecContext {
            suppress_global_tool_timeout: true,
            ..ToolExecContext::default()
        };

        assert!(tool_timeout(&ctx).is_none());
    }

    #[tokio::test]
    async fn workflow_execution_uses_bound_session_db_and_mode_gate() {
        let dir = tempfile::tempdir().expect("temp session db dir");
        let db = Arc::new(
            crate::session::SessionDB::open(&dir.path().join("sessions.db"))
                .expect("open session db"),
        );
        let session = db.create_session("ha-main").expect("create session");
        let ctx = ToolExecContext {
            session_id: Some(session.id.clone()),
            session_db: Some(SessionDbHandle(db.clone())),
            ..ToolExecContext::default()
        };
        let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Run bounded smoke workflow" });
  await workflow.trace({ label: "budget", payload: { maxRuntimeSecs: 60, maxOps: 6 } });
  const validation = await workflow.validate({
    label: "validate",
    reason: "bounded smoke validation",
    commands: [{ command: "true", label: "smoke" }]
  });
  await workflow.task.update({ task, status: "completed" });
  await workflow.finish({ summary: "ok", verification: validation, residualRisk: "none" });
}
"#;
        let args = json!({
            "action": "create",
            "script": script,
            "sizeGuideline": "small",
            "runImmediately": false
        });

        let off_err = execute_tool_with_context(crate::tools::TOOL_WORKFLOW, &args, &ctx)
            .await
            .expect_err("workflow should be rejected while Workflow Mode is off");
        assert!(off_err.to_string().contains("Workflow Mode is off"));
        assert!(db
            .list_workflow_runs_for_session(&session.id, 10)
            .expect("list workflow runs")
            .is_empty());

        db.update_session_workflow_mode(&session.id, crate::workflow_mode::WorkflowMode::On)
            .expect("enable workflow mode");
        let raw = execute_tool_with_context(crate::tools::TOOL_WORKFLOW, &args, &ctx)
            .await
            .expect("workflow should create a run when Workflow Mode is on");
        let parsed: serde_json::Value = serde_json::from_str(&raw).expect("parse tool result");
        assert_eq!(parsed["kind"].as_str(), Some("general.workflow"));
        assert_eq!(parsed["initialState"].as_str(), Some("draft"));
        assert_eq!(parsed["expectedNextState"].as_str(), Some("draft"));
        assert_eq!(parsed["sizeGuideline"].as_str(), Some("small"));
        assert_eq!(parsed["startRequested"].as_bool(), Some(false));
        assert_eq!(parsed["launchAccepted"].as_bool(), Some(false));
        assert!(parsed.get("started").is_none());
        assert!(parsed.get("queued").is_none());

        let runs = db
            .list_workflow_runs_for_session(&session.id, 10)
            .expect("list workflow runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].kind, "general.workflow");
        assert_eq!(
            runs[0]
                .budget
                .get("sizeGuideline")
                .and_then(serde_json::Value::as_str),
            Some("small")
        );
        let run_id = runs[0].id.clone();

        let list_raw = execute_tool_with_context(
            crate::tools::TOOL_WORKFLOW,
            &json!({ "action": "list", "scope": "active" }),
            &ctx,
        )
        .await
        .expect("workflow list should return visible runs");
        let list: serde_json::Value =
            serde_json::from_str(&list_raw).expect("parse workflow list result");
        assert_eq!(list["action"].as_str(), Some("list"));
        assert_eq!(list["count"].as_u64(), Some(1));
        assert_eq!(list["runs"][0]["runId"].as_str(), Some(run_id.as_str()));
        assert_eq!(list["runs"][0]["sizeGuideline"].as_str(), Some("small"));

        let status_raw = execute_tool_with_context(
            crate::tools::TOOL_WORKFLOW,
            &json!({ "action": "status" }),
            &ctx,
        )
        .await
        .expect("workflow status should select the visible run");
        let status: serde_json::Value =
            serde_json::from_str(&status_raw).expect("parse workflow status result");
        assert_eq!(status["action"].as_str(), Some("status"));
        assert_eq!(status["run"]["runId"].as_str(), Some(run_id.as_str()));
        assert_eq!(status["run"]["sizeGuideline"].as_str(), Some("small"));
        assert!(status["pendingActions"].is_array());

        db.append_workflow_event(
            &run_id,
            "trace",
            json!({ "label": "checkpoint", "payload": { "summary": "phase done" } }),
        )
        .expect("append trace event");
        let trace_raw = execute_tool_with_context(
            crate::tools::TOOL_WORKFLOW,
            &json!({ "action": "trace", "runId": run_id.as_str(), "includePayload": false }),
            &ctx,
        )
        .await
        .expect("workflow trace should return events");
        let trace: serde_json::Value =
            serde_json::from_str(&trace_raw).expect("parse workflow trace result");
        assert_eq!(trace["action"].as_str(), Some("trace"));
        assert!(trace["count"].as_u64().unwrap_or(0) >= 1);
        assert!(trace["events"][0].get("payloadSummary").is_some());

        let invalid_control = execute_tool_with_context(
            crate::tools::TOOL_WORKFLOW,
            &json!({ "action": "control", "runId": run_id.as_str(), "command": "approve" }),
            &ctx,
        )
        .await
        .expect_err("workflow model tool must not accept approval control");
        assert!(invalid_control.to_string().contains("unknown variant"));

        if !crate::runtime_lock::is_primary() {
            let start_now_err = execute_tool_with_context(
                crate::tools::TOOL_WORKFLOW,
                &json!({ "action": "create", "script": script }),
                &ctx,
            )
            .await
            .expect_err("non-primary workflow should not create an unstartable draft");
            assert!(start_now_err
                .to_string()
                .contains("primary runtime process"));
            let runs = db
                .list_workflow_runs_for_session(&session.id, 10)
                .expect("list workflow runs");
            assert_eq!(
                runs.len(),
                1,
                "default-start failure must not create a draft run"
            );
        }
    }

    #[test]
    fn exec_process_background_mode_detects_exec_native_lifecycle() {
        assert_eq!(
            exec_process_background_mode(&json!({"command": "sleep 1", "background": true})),
            Some("background")
        );
        assert_eq!(
            exec_process_background_mode(&json!({"command": "sleep 1", "yield_ms": 50})),
            Some("yield_ms")
        );
        assert_eq!(
            exec_process_background_mode(&json!({
                "command": "sleep 1",
                "background": true,
                "yield_ms": 50
            })),
            Some("background/yield_ms")
        );
        assert_eq!(
            exec_process_background_mode(&json!({
                "command": "sleep 1",
                "background": false
            })),
            None
        );
    }

    #[test]
    fn explicit_async_job_cannot_wrap_unmigrated_exec_process_background() {
        let err = validate_async_background_contract(
            "exec",
            &json!({
                "command": "sleep 60",
                "background": true,
                "run_in_background": true
            }),
        )
        .expect_err("preserved process lifecycle must not also be an async job");

        let message = err.to_string();
        assert!(message.contains("exec background conflict"));
        assert!(message.contains("do not combine `run_in_background`"));
        assert!(message.contains("process session"));
    }

    #[test]
    fn legacy_exec_process_background_migrates_to_async_job_args() {
        let ctx = ToolExecContext::default();
        let legacy = json!({"command": "sleep 60", "background": true});

        assert!(should_migrate_exec_process_mode_to_async_job_with_config(
            "exec", &legacy, &ctx, true
        ));
        let migrated =
            migrate_exec_process_mode_to_async_job_args(&legacy).expect("object args migrate");
        assert_eq!(migrated.get("background"), None);
        assert_eq!(migrated.get("yield_ms"), None);
        assert_eq!(
            migrated.get("run_in_background").and_then(|v| v.as_bool()),
            Some(true)
        );

        assert_eq!(
            decide_async_path_with_config("exec", &migrated, &ctx, true, 30,),
            AsyncDecision::ImmediateBackground(JobOrigin::Explicit)
        );

        let pty_legacy = json!({"command": "top", "pty": true, "background": true});
        assert!(should_migrate_exec_process_mode_to_async_job_with_config(
            "exec",
            &pty_legacy,
            &ctx,
            true
        ));
    }

    #[test]
    fn preserved_exec_process_background_stays_sync() {
        let ctx = ToolExecContext::default();
        let never = ToolExecContext {
            async_tool_policy: AsyncToolPolicy::NeverBackground,
            ..ToolExecContext::default()
        };
        let legacy = json!({"command": "sleep 60", "yield_ms": 1000});
        assert!(!should_migrate_exec_process_mode_to_async_job_with_config(
            "exec", &legacy, &never, true
        ));
        assert!(!should_migrate_exec_process_mode_to_async_job_with_config(
            "exec", &legacy, &ctx, false
        ));
        assert_eq!(
            decide_async_path_with_config("exec", &legacy, &never, true, 30),
            AsyncDecision::Sync
        );
    }

    #[test]
    fn resolve_path_joins_relative_paths_to_session_working_dir() {
        let ctx = ToolExecContext {
            home_dir: Some("/tmp/hope-agent/coder-home".to_string()),
            session_working_dir: Some("/tmp/projects/demo".to_string()),
            ..ToolExecContext::default()
        };

        let expected = Path::new("/tmp/projects/demo")
            .join("src/main.rs")
            .to_string_lossy()
            .to_string();
        assert_eq!(ctx.resolve_path("src/main.rs"), expected);
        assert_eq!(ctx.resolve_path("/var/tmp/file.txt"), "/var/tmp/file.txt");
    }

    #[test]
    fn preserves_valid_image_marker_results_inline_for_provider_vision() {
        let output = format!(
            "{}image/png__aGVsbG8=__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );

        assert!(crate::tools::image_markers::has_valid_image_markers(
            &output
        ));
    }

    fn large_test_image_marker() -> String {
        let mut image = image::RgbImage::new(512, 512);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            let seed = x
                .wrapping_mul(1_103_515_245)
                .wrapping_add(y.wrapping_mul(12_345));
            *pixel = image::Rgb([
                (seed & 0xff) as u8,
                ((seed >> 8) & 0xff) as u8,
                ((seed >> 16) & 0xff) as u8,
            ]);
        }
        let mut buf = Cursor::new(Vec::new());
        let encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 95);
        image::DynamicImage::ImageRgb8(image)
            .write_with_encoder(encoder)
            .expect("encode test image");
        let jpeg = buf.into_inner();
        assert!(jpeg.len() > 50_000);
        let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
        format!("{IMAGE_BASE64_PREFIX}image/jpeg__{b64}__\nScreenshot captured.")
    }

    #[test]
    fn image_marker_results_materialize_to_file_markers() {
        let root = tempfile::tempdir().expect("tempdir");

        crate::test_support::with_env_vars(&[("HA_DATA_DIR", root.path())], || {
            let output = format!(
                "{}image/png__{}__\nScreenshot captured.",
                IMAGE_BASE64_PREFIX,
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAFgwJ/lT5cWQAAAABJRU5ErkJggg=="
            );
            let ctx = ToolExecContext {
                session_id: Some("session/../x".to_string()),
                ..ToolExecContext::default()
            };

            let result = maybe_persist_large_tool_result("image", output, &ctx)
                .expect("persist large image marker");

            assert!(result.contains(IMAGE_FILE_PREFIX));
            assert!(!result.contains(IMAGE_BASE64_PREFIX));
            assert!(result.contains("Screenshot captured."));
            assert!(result.contains("session____x"));

            let spec_line = result
                .strip_prefix(IMAGE_FILE_PREFIX)
                .and_then(|rest| rest.split_once('\n').map(|(spec, _)| spec))
                .expect("file marker JSON line");
            let spec: serde_json::Value =
                serde_json::from_str(spec_line).expect("file marker JSON");
            let path = spec
                .get("path")
                .and_then(|v| v.as_str())
                .expect("path in marker");
            assert!(Path::new(path).starts_with(root.path().join("tool_results/session____x")));
            assert!(std::fs::metadata(path).expect("materialized file").len() > 0);
        });
    }

    #[test]
    fn incognito_large_image_marker_results_stay_inline() {
        let output = large_test_image_marker();
        let ctx = ToolExecContext {
            incognito: true,
            session_id: Some("secret-session".to_string()),
            ..ToolExecContext::default()
        };

        let result = maybe_persist_large_tool_result("image", output.clone(), &ctx)
            .expect("incognito image marker");

        assert_eq!(result, output);
        assert!(result.contains(IMAGE_BASE64_PREFIX));
        assert!(!result.contains(IMAGE_FILE_PREFIX));
    }

    #[tokio::test]
    async fn incognito_blocks_memory_tier_before_handler() {
        let ctx = ToolExecContext {
            incognito: true,
            ..ToolExecContext::default()
        };

        let err = super::execute_tool_with_context(
            crate::tools::TOOL_RECALL_MEMORY,
            &json!({ "query": "anything" }),
            &ctx,
        )
        .await
        .expect_err("incognito must hide memory-tier tools before handler execution");

        assert!(
            err.to_string().contains("Incognito restriction"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn persisted_preview_omits_image_marker_prefix_for_malformed_image_results() {
        let output = format!(
            "{}image/png__not-base64__\nScreenshot captured.",
            IMAGE_BASE64_PREFIX
        );

        let preview = build_persisted_large_result_preview(&output, "/tmp/browser_1.txt");

        assert!(!preview.contains(IMAGE_BASE64_PREFIX));
        assert!(preview.contains("Large tool result"));
        assert!(preview.contains("/tmp/browser_1.txt"));
    }

    #[test]
    fn persisted_preview_preserves_media_items_header() {
        let output = format!(
            "{}[]\n{}",
            crate::agent::MEDIA_ITEMS_PREFIX,
            "x".repeat(10_000)
        );

        let preview = build_persisted_large_result_preview(&output, "/tmp/tool.txt");

        assert!(preview.starts_with(crate::agent::MEDIA_ITEMS_PREFIX));
        assert!(preview.contains("/tmp/tool.txt"));
    }

    #[test]
    fn trusted_mcp_auto_approve_config_skips_regular_approval() {
        let cfg = mcp_cfg(true, McpTrustLevel::Trusted);
        assert!(mcp_server_auto_approves_config(&cfg));
    }

    #[test]
    fn untrusted_mcp_auto_approve_is_rejected_and_not_honored() {
        let cfg = mcp_cfg(true, McpTrustLevel::Untrusted);
        assert!(cfg.validate().is_err());
        assert!(!mcp_server_auto_approves_config(&cfg));
    }

    #[test]
    fn auto_approved_mcp_tool_skips_engine_outside_plan_mode() {
        let ctx = ToolExecContext::default();
        assert!(!needs_permission_engine(
            "mcp__alpha__read",
            &json!({}),
            &ctx,
            true
        ));
    }

    #[test]
    fn auto_approved_connector_action_still_runs_engine() {
        let ctx = ToolExecContext {
            auto_approve_tools: true,
            ..ToolExecContext::default()
        };
        assert!(needs_permission_engine(
            crate::tools::feishu::TOOL_CALENDAR_CREATE_EVENT,
            &json!({"summary": "Customer call"}),
            &ctx,
            ctx.local_auto_approve()
        ));
        assert!(needs_permission_engine(
            "mcp__gmail__send_email",
            &json!({"to": "user@example.com", "body": "hello"}),
            &ctx,
            true
        ));
    }

    #[test]
    fn external_pre_approved_connector_action_skips_engine_reentry() {
        let ctx = ToolExecContext {
            external_pre_approved: true,
            ..ToolExecContext::default()
        };
        assert!(!needs_permission_engine(
            crate::tools::feishu::TOOL_CALENDAR_CREATE_EVENT,
            &json!({"summary": "Customer call"}),
            &ctx,
            ctx.local_auto_approve()
        ));
    }

    #[test]
    fn plan_ask_tools_keep_engine_for_auto_approved_mcp_tool() {
        let tool = "mcp__alpha__read".to_string();
        let ctx = ToolExecContext {
            plan_mode_allowed_tools: vec![tool.clone()],
            plan_mode_ask_tools: vec![tool.clone()],
            ..ToolExecContext::default()
        };

        assert!(needs_permission_engine(&tool, &json!({}), &ctx, true));
    }

    // ── Regression: `external_pre_approved` vs `auto_approve_tools` split ──
    //
    // Before the split there was a single `auto_approve_tools` flag used both
    // by IM auto-approve accounts ("skip ALL gates") and by async-job
    // re-entry helpers ("engine already ran outside"). For `exec` the
    // re-entry meaning was wrong — the outer engine gate intentionally
    // excludes `TOOL_EXEC` (see `needs_permission_engine`), so flipping
    // `auto_approve_tools=true` on re-entry let `exec` silently bypass its
    // own command-level dangerous/edit audit. These tests pin the new
    // contract: `external_pre_approved` only suppresses the engine gate,
    // never the per-tool command-level gate.

    #[test]
    fn external_pre_approved_skips_engine_for_non_exec() {
        let ctx = ToolExecContext {
            external_pre_approved: true,
            exec_pre_approved: false,
            ..ToolExecContext::default()
        };
        assert!(ctx.local_auto_approve());
        assert!(!needs_permission_engine(
            "read",
            &json!({"path": "/tmp/x"}),
            &ctx,
            ctx.local_auto_approve()
        ));
    }

    #[test]
    fn external_pre_approved_does_not_pierce_exec_command_gate() {
        // Core regression: even with `external_pre_approved=true` the
        // command-level audit (dangerous/edit-commands + AllowAlways prefix)
        // must still run inside `exec::tool_exec`.
        let ctx = ToolExecContext {
            external_pre_approved: true,
            exec_pre_approved: false,
            auto_approve_tools: false,
            ..ToolExecContext::default()
        };
        assert!(
            ctx.should_run_exec_command_gate(),
            "external_pre_approved must NOT bypass exec command-level audit"
        );
    }

    #[test]
    fn auto_approve_tools_pierces_exec_command_gate() {
        // IM auto-approve account / skill-triggered slash command behavior:
        // `auto_approve_tools=true` legitimately bypasses every gate
        // including the exec command-level audit.
        let ctx = ToolExecContext {
            auto_approve_tools: true,
            ..ToolExecContext::default()
        };
        assert!(
            !ctx.should_run_exec_command_gate(),
            "IM auto-approve behavior regression"
        );
    }

    #[test]
    fn plan_mode_ask_tools_pierces_external_pre_approved_for_exec() {
        // Plan Mode `ask_tools` user-sovereignty contract: even if a recursive
        // inner dispatch claims "engine already ran outside", Plan Mode forces
        // the engine to re-prompt because the outer turn's plan agent had
        // already decided this tool must always ask.
        let ctx = ToolExecContext {
            external_pre_approved: true,
            exec_pre_approved: false,
            plan_mode_allowed_tools: vec!["exec".to_string()],
            plan_mode_ask_tools: vec!["exec".to_string()],
            ..ToolExecContext::default()
        };
        assert!(needs_permission_engine(
            "exec",
            &json!({"command": "ls"}),
            &ctx,
            ctx.local_auto_approve()
        ));
    }

    #[test]
    fn plan_mode_ask_tools_pierces_auto_approve_tools_for_exec() {
        let ctx = ToolExecContext {
            auto_approve_tools: true,
            plan_mode_allowed_tools: vec!["exec".to_string()],
            plan_mode_ask_tools: vec!["exec".to_string()],
            ..ToolExecContext::default()
        };
        assert!(needs_permission_engine(
            "exec",
            &json!({"command": "ls"}),
            &ctx,
            ctx.local_auto_approve()
        ));
    }

    #[test]
    fn async_spawn_keeps_exec_command_gate() {
        // Pins the spawn.rs / auto-bg helper contract: when re-dispatching
        // into the OS-thread runtime, only `external_pre_approved` may be
        // flipped to silence the engine re-entry; `auto_approve_tools` must
        // stay false so the command-level audit still catches things like
        // `git push --force` or `rm -rf /`.
        let inner_ctx = ToolExecContext {
            bypass_async_dispatch: true,
            external_pre_approved: true,
            exec_pre_approved: false,
            // auto_approve_tools intentionally NOT touched
            ..ToolExecContext::default()
        };
        assert!(
            inner_ctx.should_run_exec_command_gate(),
            "async spawn must NOT flip auto_approve_tools — that was the original CVE-class bug"
        );
        // Engine gate skipped on re-entry (exec was already excluded from the
        // outer engine gate; the load-bearing guarantee is the command-level
        // audit above still fires).
        assert!(!needs_permission_engine(
            "exec",
            &json!({"command": "rm -rf /"}),
            &inner_ctx,
            inner_ctx.local_auto_approve()
        ));
    }

    #[test]
    fn exec_reorder_gate_skips_when_already_approved_at_outer_gate() {
        // review#3: a Plan-Mode-ask exec that the user already approved at the
        // OUTER engine gate must NOT be re-prompted by the async reorder.
        let ctx = ToolExecContext::default(); // auto_approve=false, exec_pre_approved=false
                                              // The reorder runs only for the AUTO-background tier (approval must
                                              // resolve before the budget timer starts, ASYNC-2).
        let auto_bg = AsyncDecision::AutoBackgroundEligible;
        // Fresh auto-bg exec, not yet approved → reorder runs its gate.
        assert!(should_run_exec_reorder_gate("exec", auto_bg, false, &ctx));
        // Already approved at the outer plan-ask gate → reorder is suppressed
        // (one prompt, not two).
        assert!(!should_run_exec_reorder_gate("exec", auto_bg, true, &ctx));
        // Sync (non-backgrounding) exec → reorder never runs (inner gate handles it).
        assert!(!should_run_exec_reorder_gate(
            "exec",
            AsyncDecision::Sync,
            false,
            &ctx
        ));
        // Non-exec auto-bg tool → already gated by the outer engine, no reorder.
        assert!(!should_run_exec_reorder_gate(
            "web_search",
            auto_bg,
            false,
            &ctx
        ));
    }

    #[test]
    fn exec_reorder_gate_excludes_immediate_background_for_r8_parking() {
        // R8: explicit `run_in_background` / policy AlwaysBackground exec does NOT
        // reorder its approval to the foreground turn. The command gate is
        // deferred to the background job thread so an attended approval parks the
        // job at AwaitingApproval and resolves asynchronously (the model gets the
        // job id immediately; a denial settles the job terminal via injection).
        let ctx = ToolExecContext::default(); // exec_pre_approved=false
        let immediate = AsyncDecision::ImmediateBackground(JobOrigin::Explicit);
        assert!(
            !should_run_exec_reorder_gate("exec", immediate, false, &ctx),
            "ImmediateBackground exec must defer its approval gate to the job thread (R8)"
        );
        // ...unless a prior engine prompt this turn already approved it
        // (Plan-Mode-ask path) — then there is nothing left to gate, parked or not.
        assert!(!should_run_exec_reorder_gate("exec", immediate, true, &ctx));
    }

    #[test]
    fn exec_reorder_gate_respects_global_command_gate_bypass() {
        // auto_approve_tools / exec_pre_approved on the ctx globally bypass the
        // command gate → the reorder must not prompt either.
        let auto = ToolExecContext {
            auto_approve_tools: true,
            ..ToolExecContext::default()
        };
        let pre = ToolExecContext {
            exec_pre_approved: true,
            ..ToolExecContext::default()
        };
        let auto_bg = AsyncDecision::AutoBackgroundEligible;
        assert!(!should_run_exec_reorder_gate("exec", auto_bg, false, &auto));
        assert!(!should_run_exec_reorder_gate("exec", auto_bg, false, &pre));
    }

    #[test]
    fn exec_pre_approved_bypasses_exec_command_gate() {
        // B2: the async approval-reorder sets `exec_pre_approved=true` only
        // AFTER it already ran the command gate at the outer dispatch, so the
        // background re-dispatch must skip the inner gate — one prompt, not
        // two. Physically distinct from `external_pre_approved`, which must
        // NEVER pierce the command gate (see the regression above).
        let ctx = ToolExecContext {
            exec_pre_approved: true,
            external_pre_approved: true,
            auto_approve_tools: false,
            ..ToolExecContext::default()
        };
        assert!(
            !ctx.should_run_exec_command_gate(),
            "exec_pre_approved (set post-approval by the reorder) must bypass the inner gate"
        );
    }

    /// `ToolExecContext::emit_effective_args` is the bridge the streaming
    /// loop uses to surface `PreToolUse` `updatedInput` rewrites to the UI /
    /// history / `PostToolUse` hook input. Verify the sink is populated
    /// exactly when wired up — non-wired contexts (slash commands,
    /// async-job re-entry, the direct `execute_tool` helper) must remain
    /// no-op so they don't pay the lock cost.
    #[tokio::test]
    async fn effective_args_sink_emits_only_when_wired() {
        use std::sync::Arc;
        use tokio::sync::Mutex as AsyncMutex;

        // Wired sink: emit populates it.
        let sink = Arc::new(AsyncMutex::new(None));
        let ctx = ToolExecContext {
            effective_args_sink: Some(sink.clone()),
            ..ToolExecContext::default()
        };
        ctx.emit_effective_args(json!({ "command": "echo safe" }))
            .await;
        let drained = sink.lock().await.take();
        assert_eq!(
            drained.as_ref().and_then(|v| v.get("command")),
            Some(&json!("echo safe")),
        );

        // No sink: emit is a no-op (no panic, nothing observable changes).
        let bare = ToolExecContext::default();
        bare.emit_effective_args(json!({ "ignored": true })).await;
        // No assertion needed beyond "did not panic" — the bare context has
        // no sink to inspect.
    }
}
