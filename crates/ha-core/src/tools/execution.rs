use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::app_update;
use super::issue_report;
use super::project_read_file;
use super::send_attachment;
use super::skill;
use super::{
    acp_spawn, browser, cron, mac_control, memory, notification, settings, subagent, team, weather,
    web_fetch, web_search,
};
use super::{
    agents, ask_user_question, canvas, enter_plan_mode, image, image_generate, job_status, pdf,
    runtime_cancel, sessions, submit_plan, task,
};
use super::{apply_patch, edit, exec, find, grep, ls, process, read, write};
use super::{
    approval, TOOL_ACP_SPAWN, TOOL_AGENTS_LIST, TOOL_APPLY_PATCH, TOOL_ASK_USER_QUESTION,
    TOOL_BROWSER, TOOL_CANVAS, TOOL_DELETE_MEMORY, TOOL_EDIT, TOOL_ENTER_PLAN_MODE, TOOL_EXEC,
    TOOL_FIND, TOOL_GET_SETTINGS, TOOL_GET_WEATHER, TOOL_GREP, TOOL_IMAGE, TOOL_IMAGE_GENERATE,
    TOOL_ISSUE_REPORT, TOOL_JOB_STATUS, TOOL_LIST_SETTINGS_BACKUPS, TOOL_LS, TOOL_MAC_CONTROL,
    TOOL_MANAGE_CRON, TOOL_MEMORY_GET, TOOL_PDF, TOOL_PROCESS, TOOL_PROJECT_READ_FILE, TOOL_READ,
    TOOL_RECALL_MEMORY, TOOL_RESTORE_SETTINGS_BACKUP, TOOL_RUNTIME_CANCEL, TOOL_SAVE_MEMORY,
    TOOL_SEND_ATTACHMENT, TOOL_SEND_NOTIFICATION, TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST,
    TOOL_SESSIONS_SEND, TOOL_SESSION_STATUS, TOOL_SUBAGENT, TOOL_SUBMIT_PLAN, TOOL_TASK_CREATE,
    TOOL_TASK_LIST, TOOL_TASK_UPDATE, TOOL_TEAM, TOOL_UPDATE_CORE_MEMORY, TOOL_UPDATE_MEMORY,
    TOOL_UPDATE_SETTINGS, TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WRITE,
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
    let resolve_ctx = crate::permission::engine::ResolveContext {
        tool_name,
        args,
        session_mode: ctx.session_mode,
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
    };
    crate::permission::engine::resolve_async(&resolve_ctx).await
}

/// Load the user-configured tool timeout from config.json. Returns `None`
/// when the user explicitly set 0 (disabled). The serde default in
/// [`AppConfig`] provides the 300s fallback when the field is missing.
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
    /// Per-agent async tool backgrounding policy (mirrors AgentConfig.capabilities.async_tool_policy).
    pub async_tool_policy: AsyncToolPolicy,
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
    /// `async_jobs::spawn::persist_result`, so the generic large-result
    /// preview layer must not wrap the output first and turn the async
    /// output-file into a pointer to a second file.
    pub suppress_result_disk_persistence: bool,
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
    /// + edit-commands + AllowAlways prefix). Only `auto_approve_tools`
    /// bypasses this — `external_pre_approved` deliberately does NOT,
    /// because the outer engine gate excludes `TOOL_EXEC` and this audit is
    /// `exec`'s only safeguard against dangerous patterns when the call is
    /// re-dispatched through the async-job spawner / auto-bg helper.
    ///
    /// Changing this read site without also updating
    /// [`Self::auto_approve_tools`] / [`Self::external_pre_approved`] doc
    /// is a security regression.
    #[inline]
    pub fn should_run_exec_command_gate(&self) -> bool {
        !self.auto_approve_tools
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
        let dispatch_ctx = super::dispatch::DispatchContext {
            agent_id,
            mcp_enabled: agent_cfg.capabilities.mcp_enabled,
            memory_enabled: agent_cfg.memory.enabled,
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
            super::dispatch::ToolFate::Hidden => Some(format!(
                "Agent tool switch: tool '{}' is disabled for this agent.",
                canonical
            )),
        }
    }

    /// Human-readable reason when a tool is blocked by the current restrictions.
    pub fn tool_visibility_error(&self, name: &str) -> Option<String> {
        if let Some(err) = self.builtin_fate_error(name) {
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

    /// Push the effective (post-`PreToolUse` rewrite) tool arguments into the
    /// per-dispatch sink. Called once at most, only when `updatedInput`
    /// shadowed the model's args. No-op when no sink is wired up.
    pub(crate) async fn emit_effective_args(&self, value: Value) {
        if let Some(sink) = &self.effective_args_sink {
            *sink.lock().await = Some(value);
        }
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
#[derive(Debug, Clone, Copy)]
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

/// Inspect tool metadata, args, and agent policy to decide whether this call
/// should detach immediately, become eligible for auto-background, or run
/// purely synchronously. Recursion-safe via `bypass_async_dispatch`.
fn decide_async_path(name: &str, args: &Value, ctx: &ToolExecContext) -> AsyncDecision {
    if ctx.bypass_async_dispatch {
        return AsyncDecision::Sync;
    }
    if !super::is_async_capable(name) {
        return AsyncDecision::Sync;
    }
    let cfg = crate::config::cached_config();
    if !cfg.async_tools.enabled {
        return AsyncDecision::Sync;
    }
    if matches!(ctx.async_tool_policy, AsyncToolPolicy::NeverBackground) {
        return AsyncDecision::Sync;
    }
    let explicit_bg = args
        .get("run_in_background")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if explicit_bg {
        return AsyncDecision::ImmediateBackground(JobOrigin::Explicit);
    }
    if matches!(ctx.async_tool_policy, AsyncToolPolicy::AlwaysBackground) {
        return AsyncDecision::ImmediateBackground(JobOrigin::PolicyForced);
    }
    if cfg.async_tools.auto_background_secs > 0 {
        return AsyncDecision::AutoBackgroundEligible;
    }
    AsyncDecision::Sync
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
async fn run_tool_approval(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    reason_payload: Option<approval::ApprovalReasonPayload>,
    allow_always_forbidden: bool,
) -> anyhow::Result<()> {
    let desc = format!("tool: {} {}", name, {
        let s = args.to_string();
        if s.len() > 200 {
            format!("{}...", crate::truncate_utf8(&s, 200))
        } else {
            s
        }
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
            Ok(())
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
            Ok(())
        }
        Ok(approval::ApprovalResponse::Deny) => {
            Err(super::rejection::ToolRejection::denied_by_user(name))
        }
        Err(approval::ApprovalCheckError::TimedOut { timeout_secs }) => {
            match approval::approval_timeout_action() {
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
                    Ok(())
                }
            }
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
    if let Some(err) = ctx.tool_visibility_error(name) {
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
    if needs_engine {
        let decision =
            resolve_tool_permission(name, args, ctx, super::is_internal_tool(name)).await;
        match decision {
            crate::permission::Decision::Allow => {
                // Engine would allow without a prompt. A PreToolUse hook that
                // returned `ask`/`defer` still wants human confirmation — force
                // the prompt (no reason banner) so its request can't fail open.
                if pre_force_prompt {
                    run_tool_approval(name, args, ctx, None, false).await?;
                }
            }
            crate::permission::Decision::Deny { reason } => {
                // PermissionDenied hook (observation): engine policy auto-denied
                // this tool (no user prompt — that decline path fires from the
                // approval layer instead).
                crate::hooks::fire_permission_denied(ctx.session_id.as_deref(), name, "policy");
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
                    run_tool_approval(
                        name,
                        args,
                        ctx,
                        Some(approval::ApprovalReasonPayload::from(&reason)),
                        forbidden,
                    )
                    .await?;
                    restore_mac_control_approval_focus_anchor(mac_control_focus_anchor).await;
                }
            }
        }
    } else if pre_force_prompt && !is_skill_read(name, args) {
        // The engine gate was skipped (auto-approve / exec's own gate), but a
        // PreToolUse hook explicitly asked for confirmation — honor it rather
        // than letting the request through silently. SKILL.md reads are exempt
        // so skill bootstrap never blocks on a prompt.
        run_tool_approval(name, args, ctx, None, false).await?;
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
        let raw = async_jobs::spawn_explicit_job(name, args.clone(), ctx.clone(), origin)?;
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
        let raw =
            async_jobs::dispatch_with_auto_background(name, args, &inner_ctx, auto_bg_secs).await?;
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
            TOOL_PROJECT_READ_FILE => {
                project_read_file::tool_project_read_file(args, dispatch_ctx).await
            }
            TOOL_WRITE | "write_file" => write::tool_write_file(args, dispatch_ctx).await,
            TOOL_EDIT | "patch_file" => edit::tool_edit(args, dispatch_ctx).await,
            TOOL_LS | "list_dir" => ls::tool_ls(args, dispatch_ctx).await,
            TOOL_GREP => grep::tool_grep(args, dispatch_ctx).await,
            TOOL_FIND => find::tool_find(args, dispatch_ctx).await,
            TOOL_APPLY_PATCH => apply_patch::tool_apply_patch(args, dispatch_ctx).await,
            TOOL_WEB_SEARCH => web_search::tool_web_search(args).await,
            TOOL_WEB_FETCH => web_fetch::tool_web_fetch(args).await,
            TOOL_SAVE_MEMORY => memory::tool_save_memory(args, dispatch_ctx).await,
            TOOL_RECALL_MEMORY => memory::tool_recall_memory(args).await,
            TOOL_UPDATE_MEMORY => memory::tool_update_memory(args).await,
            TOOL_DELETE_MEMORY => memory::tool_delete_memory(args).await,
            TOOL_UPDATE_CORE_MEMORY => {
                memory::tool_update_core_memory(
                    args,
                    dispatch_ctx
                        .agent_id
                        .as_deref()
                        .unwrap_or(crate::agent_loader::DEFAULT_AGENT_ID),
                )
                .await
            }
            TOOL_MANAGE_CRON => {
                cron::tool_manage_cron(args, dispatch_ctx.session_id.as_deref()).await
            }
            TOOL_BROWSER => browser::tool_browser(args, dispatch_ctx.session_id.as_deref()).await,
            TOOL_MAC_CONTROL => mac_control::tool_mac_control(args).await,
            TOOL_SEND_NOTIFICATION => {
                notification::tool_send_notification(args, dispatch_ctx).await
            }
            TOOL_SUBAGENT => subagent::tool_subagent(args, dispatch_ctx).await,
            TOOL_TEAM => team::tool_team(args, dispatch_ctx).await,
            TOOL_ACP_SPAWN => acp_spawn::tool_acp_spawn(args, dispatch_ctx).await,
            TOOL_MEMORY_GET => memory::tool_memory_get(args).await,
            TOOL_AGENTS_LIST => agents::tool_agents_list(args).await,
            TOOL_SESSIONS_LIST => sessions::tool_sessions_list(args).await,
            TOOL_SESSION_STATUS => sessions::tool_session_status(args).await,
            TOOL_SESSIONS_HISTORY => sessions::tool_sessions_history(args).await,
            TOOL_SESSIONS_SEND => Box::pin(sessions::tool_sessions_send(args, dispatch_ctx)).await,
            TOOL_IMAGE => image::tool_image(args).await,
            TOOL_IMAGE_GENERATE => image_generate::tool_image_generate(args, dispatch_ctx).await,
            TOOL_ISSUE_REPORT => issue_report::tool_issue_report(args, dispatch_ctx).await,
            TOOL_PDF => pdf::tool_pdf(args).await,
            TOOL_CANVAS => canvas::tool_canvas(args, dispatch_ctx).await,
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
            super::TOOL_APP_UPDATE => app_update::tool_app_update(args, dispatch_ctx).await,
            TOOL_JOB_STATUS => job_status::tool_job_status(args).await,
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
                    name, hard_timeout.as_secs()
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
        Ok(output) => maybe_persist_large_tool_result(name, output, ctx),
        other => other,
    }
}

// ── Large result disk persistence ────────────────────────────────
// If the result exceeds the threshold, write it to disk and return a preview
// with a path reference so the model can `read` the full file.
fn maybe_persist_large_tool_result(
    name: &str,
    output: String,
    ctx: &ToolExecContext,
) -> anyhow::Result<String> {
    if ctx.suppress_result_disk_persistence || !should_persist_large_result(&output) {
        return Ok(output);
    }
    if crate::tools::image_markers::has_valid_image_markers(&output) {
        app_info!(
            "tool",
            "disk_persist",
            "Tool '{}' result {}B contains valid image marker; preserving inline for provider vision",
            name,
            output.len()
        );
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

fn persist_large_result(
    content: &str,
    session_id: Option<&str>,
    tool_name: &str,
) -> anyhow::Result<String> {
    let base_dir = crate::paths::root_dir()?
        .join("tool_results")
        .join(session_id.unwrap_or("_global"));
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
        build_persisted_large_result_preview, mcp_server_auto_approves_config,
        needs_permission_engine, tool_timeout, ToolExecContext,
    };
    use crate::mcp::{McpServerConfig, McpTransportSpec, McpTrustLevel};
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    use serde_json::json;
    use std::collections::BTreeMap;
    use std::path::Path;

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
