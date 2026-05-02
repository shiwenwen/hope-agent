use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

use super::project_read_file;
use super::send_attachment;
use super::skill;
use super::{
    acp_spawn, browser, cron, memory, notification, settings, subagent, team, weather, web_fetch,
    web_search,
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
    TOOL_JOB_STATUS, TOOL_LIST_SETTINGS_BACKUPS, TOOL_LS, TOOL_MANAGE_CRON, TOOL_MEMORY_GET,
    TOOL_PDF, TOOL_PROCESS, TOOL_PROJECT_READ_FILE, TOOL_READ, TOOL_RECALL_MEMORY,
    TOOL_RESTORE_SETTINGS_BACKUP, TOOL_RUNTIME_CANCEL, TOOL_SAVE_MEMORY, TOOL_SEND_ATTACHMENT,
    TOOL_SEND_NOTIFICATION, TOOL_SESSIONS_HISTORY, TOOL_SESSIONS_LIST, TOOL_SESSIONS_SEND,
    TOOL_SESSION_STATUS, TOOL_SUBAGENT, TOOL_SUBMIT_PLAN, TOOL_TASK_CREATE, TOOL_TASK_LIST,
    TOOL_TASK_UPDATE, TOOL_TEAM, TOOL_UPDATE_CORE_MEMORY, TOOL_UPDATE_MEMORY, TOOL_UPDATE_SETTINGS,
    TOOL_WEB_FETCH, TOOL_WEB_SEARCH, TOOL_WRITE,
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
        is_internal_tool,
        smart_config: app_cfg.as_deref().map(|c| &c.permission.smart),
    };
    crate::permission::engine::resolve_async(&resolve_ctx).await
}

/// Load the user-configured tool timeout from config.json. Returns `None`
/// when the user explicitly set 0 (disabled). The serde default in
/// [`AppConfig`] provides the 300s fallback when the field is missing.
fn tool_timeout() -> Option<Duration> {
    let secs = crate::config::cached_config().tool_timeout;
    if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    }
}

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
    /// Current agent ID
    pub agent_id: Option<String>,
    /// Sub-agent nesting depth (0 = top-level)
    pub subagent_depth: u32,
    /// Agent-level tool filter from `agent.json` capabilities.tools.
    /// Internal system tools are exempt at this layer to preserve existing UI semantics.
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
    /// When true, automatically approve all tool calls (IM channel auto-approve mode).
    pub auto_approve_tools: bool,
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
}

impl ToolExecContext {
    /// Returns the default path for path-aware tools: session working dir,
    /// then agent home, then ".".
    pub fn default_path(&self) -> &str {
        self.session_working_dir
            .as_deref()
            .or(self.home_dir.as_deref())
            .unwrap_or(".")
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

    /// Human-readable reason when a tool is blocked by the current restrictions.
    pub fn tool_visibility_error(&self, name: &str) -> Option<String> {
        if !super::agent_tool_filter_allows(name, &self.agent_tool_filter) {
            return Some(format!(
                "Agent tool filter: tool '{}' is disabled for this agent.",
                name
            ));
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

/// Execute a tool with additional context (model info, etc.)
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
    let needs_engine = !ctx.auto_approve_tools && !is_skill_read(name, args) && name != TOOL_EXEC;
    if needs_engine {
        let decision =
            resolve_tool_permission(name, args, ctx, super::is_internal_tool(name)).await;
        match decision {
            crate::permission::Decision::Allow => {}
            crate::permission::Decision::Deny { reason } => {
                return Err(anyhow::anyhow!("Tool '{}' denied: {}", name, reason));
            }
            crate::permission::Decision::Ask { reason } => {
                let desc = format!("tool: {} {}", name, {
                    let s = args.to_string();
                    if s.len() > 200 {
                        format!("{}...", crate::truncate_utf8(&s, 200))
                    } else {
                        s
                    }
                });
                let cwd = ctx.default_path();
                let reason_payload = Some(approval::ApprovalReasonPayload::from(&reason));
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
                    }
                    Ok(approval::ApprovalResponse::AllowAlways) => {
                        if reason.forbids_allow_always() {
                            app_info!(
                                "tool",
                                "approval",
                                "Tool '{}' approved once (AllowAlways unavailable: {:?})",
                                name,
                                reason
                            );
                        } else {
                            // Multi-scope (project / session / agent_home /
                            // global) AllowAlways persistence is wired in by
                            // the approval dialog upgrade. For now `exec`
                            // still uses the legacy command-prefix store
                            // inside `tool_exec`.
                            app_info!("tool", "approval", "Tool '{}' approved (always)", name);
                        }
                    }
                    Ok(approval::ApprovalResponse::Deny) => {
                        return Err(anyhow::anyhow!("Tool '{}' execution denied by user", name));
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
                                return Err(anyhow::anyhow!(
                                    "Tool '{}' execution denied: approval timed out after {}s",
                                    name,
                                    timeout_secs
                                ));
                            }
                            crate::config::ApprovalTimeoutAction::Proceed => {
                                app_warn!(
                                    "tool",
                                    "approval",
                                    "Tool '{}' approval timed out after {}s; proceeding by config",
                                    name,
                                    timeout_secs
                                );
                            }
                        }
                    }
                    Err(e) => {
                        app_warn!(
                            "tool",
                            "approval",
                            "Tool approval check failed for '{}' ({}), proceeding",
                            name,
                            e
                        );
                    }
                }
            }
        }
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
        // Approval already ran at this outer layer — silence the inner re-entry.
        inner_ctx.auto_approve_tools = true;
        let raw =
            async_jobs::dispatch_with_auto_background(name, args, &inner_ctx, auto_bg_secs).await?;
        // The auto-bg helper already routed the result through this function
        // recursively (inner_ctx.bypass_async_dispatch=true) so disk-persist
        // and logging fired inside that nested call. Return as-is.
        return Ok(raw);
    }

    // ── Conditional skill activation (`paths:` frontmatter) ──────
    // Scan args for file paths the tool is about to touch, then light up
    // any `paths:` skills whose patterns match. The skill catalog in the
    // *next* system-prompt build will include them; we bump skill_version
    // so the 30s skill cache doesn't swallow this change.
    if ctx.session_id.is_some() {
        maybe_activate_conditional_skills(name, args, ctx);
    }

    let dispatch = async {
        match name {
            TOOL_EXEC => exec::tool_exec(args, ctx).await,
            TOOL_PROCESS => process::tool_process(args).await,
            TOOL_READ | "read_file" => read::tool_read_file(args, ctx).await,
            TOOL_PROJECT_READ_FILE => project_read_file::tool_project_read_file(args, ctx).await,
            TOOL_WRITE | "write_file" => write::tool_write_file(args, ctx).await,
            TOOL_EDIT | "patch_file" => edit::tool_edit(args, ctx).await,
            TOOL_LS | "list_dir" => ls::tool_ls(args, ctx).await,
            TOOL_GREP => grep::tool_grep(args, ctx).await,
            TOOL_FIND => find::tool_find(args, ctx).await,
            TOOL_APPLY_PATCH => apply_patch::tool_apply_patch(args, ctx).await,
            TOOL_WEB_SEARCH => web_search::tool_web_search(args).await,
            TOOL_WEB_FETCH => web_fetch::tool_web_fetch(args).await,
            TOOL_SAVE_MEMORY => memory::tool_save_memory(args, ctx).await,
            TOOL_RECALL_MEMORY => memory::tool_recall_memory(args).await,
            TOOL_UPDATE_MEMORY => memory::tool_update_memory(args).await,
            TOOL_DELETE_MEMORY => memory::tool_delete_memory(args).await,
            TOOL_UPDATE_CORE_MEMORY => {
                memory::tool_update_core_memory(args, ctx.agent_id.as_deref().unwrap_or("default"))
                    .await
            }
            TOOL_MANAGE_CRON => cron::tool_manage_cron(args, ctx.session_id.as_deref()).await,
            TOOL_BROWSER => browser::tool_browser(args, ctx.session_id.as_deref()).await,
            TOOL_SEND_NOTIFICATION => notification::tool_send_notification(args, ctx).await,
            TOOL_SUBAGENT => subagent::tool_subagent(args, ctx).await,
            TOOL_TEAM => team::tool_team(args, ctx).await,
            TOOL_ACP_SPAWN => acp_spawn::tool_acp_spawn(args, ctx).await,
            TOOL_MEMORY_GET => memory::tool_memory_get(args).await,
            TOOL_AGENTS_LIST => agents::tool_agents_list(args).await,
            TOOL_SESSIONS_LIST => sessions::tool_sessions_list(args).await,
            TOOL_SESSION_STATUS => sessions::tool_session_status(args).await,
            TOOL_SESSIONS_HISTORY => sessions::tool_sessions_history(args).await,
            TOOL_SESSIONS_SEND => Box::pin(sessions::tool_sessions_send(args, ctx)).await,
            TOOL_IMAGE => image::tool_image(args).await,
            TOOL_IMAGE_GENERATE => image_generate::tool_image_generate(args, ctx).await,
            TOOL_PDF => pdf::tool_pdf(args).await,
            TOOL_CANVAS => canvas::tool_canvas(args, ctx).await,
            TOOL_GET_WEATHER => weather::tool_get_weather(args).await,
            TOOL_ASK_USER_QUESTION => {
                Ok(ask_user_question::execute(args, ctx.session_id.as_deref()).await)
            }
            TOOL_ENTER_PLAN_MODE => {
                Ok(enter_plan_mode::execute(args, ctx.session_id.as_deref()).await)
            }
            TOOL_SUBMIT_PLAN => Ok(submit_plan::execute(args, ctx.session_id.as_deref()).await),
            TOOL_TASK_CREATE => Ok(task::tool_task_create(args, ctx.session_id.as_deref()).await),
            TOOL_TASK_UPDATE => Ok(task::tool_task_update(args, ctx.session_id.as_deref()).await),
            TOOL_TASK_LIST => Ok(task::tool_task_list(args, ctx.session_id.as_deref()).await),
            TOOL_JOB_STATUS => job_status::tool_job_status(args).await,
            TOOL_RUNTIME_CANCEL => runtime_cancel::tool_runtime_cancel(args).await,
            super::TOOL_TOOL_SEARCH => super::tool_search::tool_search(args, ctx).await,
            super::TOOL_PEEK_SESSIONS => {
                crate::awareness::run_peek_sessions(args, ctx.session_id.as_deref())
                    .map_err(|e| anyhow::anyhow!(e))
            }
            TOOL_GET_SETTINGS => settings::tool_get_settings(args).await,
            TOOL_UPDATE_SETTINGS => settings::tool_update_settings(args).await,
            TOOL_LIST_SETTINGS_BACKUPS => settings::tool_list_settings_backups(args).await,
            TOOL_RESTORE_SETTINGS_BACKUP => settings::tool_restore_settings_backup(args).await,
            TOOL_SEND_ATTACHMENT => send_attachment::tool_send_attachment(args, ctx).await,
            super::TOOL_SKILL => skill::tool_skill(args, ctx).await,
            super::TOOL_MCP_RESOURCE => crate::mcp::resources::tool_mcp_resource(args).await,
            super::TOOL_MCP_PROMPT => crate::mcp::prompts::tool_mcp_prompt(args).await,
            // MCP-sourced tools all share the `mcp__<server>__<tool>`
            // prefix; dispatch them through the dedicated subsystem.
            n if crate::mcp::catalog::is_mcp_tool_name(n) => {
                crate::mcp::invoke::call_tool(n, args, ctx).await
            }
            _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
        }
    };

    let result = if let Some(hard_timeout) = tool_timeout() {
        match timeout(hard_timeout, dispatch).await {
            Ok(inner) => inner,
            Err(_elapsed) => {
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

    // ── Large result disk persistence ────────────────────────────────
    // If the result exceeds the threshold, write it to disk and return
    // a preview with a path reference so the model can `read` the full file.
    match result {
        Ok(output) if should_persist_large_result(&output) => {
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
                    // Fall back to returning the full result if persistence fails
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
        other => other,
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
    use super::{build_persisted_large_result_preview, ToolExecContext};
    use crate::tools::browser::IMAGE_BASE64_PREFIX;
    use std::path::Path;

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
}
