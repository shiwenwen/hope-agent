use serde_json::Value;
use std::time::Duration;
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

use super::exec;
use super::{
    approval, TOOL_APPLY_PATCH, TOOL_EDIT, TOOL_EXEC, TOOL_LS, TOOL_MAC_CONTROL, TOOL_READ,
    TOOL_READ_CONTEXT_RESOURCE, TOOL_WORKFLOW, TOOL_WRITE,
};
use crate::agent_config::AsyncToolPolicy;
use crate::async_jobs::{self, JobOrigin};
// 执行上下文契约在 tool_defs（crate-split 阶段 4）；本文件只剩分发逻辑
// 与下方 dispatch 耦合的第二个 `impl ToolExecContext` 块。`pub(crate)`
// 再导入保住 tools/ 内部既有 `super::execution::ToolExecContext` 路径。
pub(crate) use crate::tool_defs::ToolExecContext;

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
pub async fn resolve_tool_permission(
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
    // back to the canonical hard-deny list (including cross-session
    // delegation, which could otherwise execute mutations in another regular
    // session) so the user-sovereignty contract holds within the same turn.
    // Full PlanAgent restrictions kick in automatically on the next user
    // message when the agent rebuilds.
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
        bound_context_resource_read: is_bound_context_resource_read(tool_name, args, ctx),
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

// `ToolExecContext` 本体在 `crate::tool_defs::context`；这里只保留与
// 分发注册表耦合的可见性裁决方法（fate / workflow / experiment 门），
// 它们随分发层走、不进契约层。
impl ToolExecContext {
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
        // 执行门统一先归一规范名（注册表驱动，别名不得以「无 definition
        // 的名字」滑过任何一道门）；`builtin_fate_error` /
        // `workflow_visibility_error` 内部各自也走同一入口。
        let mcp_canonical = canonical_mcp_execution_name(name);
        let canonical = canonical_builtin_tool_name(mcp_canonical.as_ref());
        self.tool_visibility_error_for_canonical(name, canonical)
            .await
    }

    async fn tool_visibility_error_for_canonical(
        &self,
        submitted_name: &str,
        canonical: &str,
    ) -> Option<String> {
        if !crate::eval_context::tool_allowed_for_experiment(self.session_id.as_deref(), canonical)
        {
            return Some(format!(
                "Evaluation experiment restriction: tool '{submitted_name}' is disabled in the compute-matched single-Agent arm."
            ));
        }
        if let Some(err) = self.builtin_fate_error(canonical) {
            return Some(err);
        }
        if let Some(err) = self.workflow_visibility_error(canonical).await {
            return Some(err);
        }
        // A durable Stop blocks every autonomous entry. The only way a model
        // can interpret a new foreground user's natural-language “continue” is
        // through this current-session harness primitive, so skill/plan/agent
        // filters must not hide or reject it.
        if canonical == crate::tool_defs::TOOL_SESSION_CONTINUE {
            return None;
        }
        // 名单类门按「原名或规范名」双判：deny 了 `read` 的策略不能被
        // `read_file` 别名绕开；allowlist 侧则保持写别名或规范名均命中
        // （只收紧、不放松——两名皆不在 allowlist 才拒）。
        if self.denied_tools.iter().any(|t| t == submitted_name)
            || crate::mcp::tool_filter_contains(&self.denied_tools, canonical)
        {
            return Some(format!(
                "Tool policy restriction: tool '{}' is denied in the current agent context.",
                submitted_name
            ));
        }
        if canonical != TOOL_READ_CONTEXT_RESOURCE
            && !self.skill_allowed_tools.is_empty()
            && !self.skill_allowed_tools.iter().any(|t| t == submitted_name)
            && !crate::mcp::tool_filter_contains(&self.skill_allowed_tools, canonical)
        {
            return Some(format!(
                "Skill restriction: tool '{}' is not allowed by the active skill.",
                submitted_name
            ));
        }
        if canonical != TOOL_READ_CONTEXT_RESOURCE
            && !self.plan_mode_allowed_tools.is_empty()
            && !self
                .plan_mode_allowed_tools
                .iter()
                .any(|t| t == submitted_name)
            && !crate::mcp::tool_filter_contains(&self.plan_mode_allowed_tools, canonical)
        {
            return Some(format!(
                "Plan Mode restriction: tool '{}' is not allowed during planning. Allowed: {}",
                submitted_name,
                self.plan_mode_allowed_tools.join(", ")
            ));
        }
        None
    }
}

/// 别名 → 规范名，事实源是分发注册表（别名与规范名共 handler 的唯一登记
/// 处）。此前这里硬编码 3 个别名，漏了 `list_dir` / `note_move`——fate
/// 兜底对漏网别名是 no-op；注册表驱动后阶段 3 外部注册的别名也自动归一。
/// 未注册的名字（MCP / 未知）原样返回。
fn canonical_builtin_tool_name(name: &str) -> &str {
    super::registry::canonical_name(name).unwrap_or(name)
}

/// Resolve a historical MCP catalog alias before it reaches any execution
/// gate. The caller keeps the submitted name in protocol/history state; this
/// runtime name is the one visibility, hooks, permissions, async policy, and
/// dispatch must agree on.
fn canonical_mcp_execution_name(name: &str) -> std::borrow::Cow<'_, str> {
    canonical_mcp_execution_name_from(name, crate::mcp::canonical_tool_name(name))
}

fn canonical_mcp_execution_name_from(
    name: &str,
    resolved: Option<String>,
) -> std::borrow::Cow<'_, str> {
    match resolved {
        Some(canonical) => std::borrow::Cow::Owned(canonical),
        None => std::borrow::Cow::Borrowed(name),
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
    if explicit_async_job_requested(args) {
        match super::background_policy_for_tool(name) {
            Some(super::BackgroundPolicy::GenericJob) => {}
            Some(super::BackgroundPolicy::SelfManaged { work_kind }) => {
                anyhow::bail!(
                    "tool '{}' manages its own {:?} lifecycle and already returns a durable handle; remove `run_in_background` to avoid a nested async job",
                    name,
                    work_kind
                );
            }
            Some(super::BackgroundPolicy::ForegroundOnly) | None => {
                anyhow::bail!(
                    "tool '{}' does not support generic `run_in_background` execution",
                    name
                );
            }
        }
    }
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
    if !super::is_generic_job_capable(name) {
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

async fn mcp_tool_auto_approves(name: &str) -> bool {
    if !crate::mcp::catalog::is_mcp_tool_name(name) {
        return false;
    }
    // 运行时查表在 ha-mcp（未接线 None → 恒 false）；信任谓词
    // `server_auto_approves_config` 留 kernel（安全语义单一来源）。
    let Some(cfg) = crate::mcp::tool_server_config(name).await else {
        return false;
    };
    crate::mcp::server_auto_approves_config(&cfg)
}

fn needs_permission_engine(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    effective_auto_approve: bool,
) -> bool {
    // Turn-local immutable reads are deterministically classified inside the
    // engine. Never bypass that single entrance, including for auto-approved
    // surfaces; the engine receives whether this exact ref is actually bound.
    if name == TOOL_READ_CONTEXT_RESOURCE {
        return true;
    }
    let plan_mode_active = !ctx.plan_mode_allowed_tools.is_empty();
    let plan_requires_ask =
        plan_mode_active && crate::mcp::tool_filter_contains(&ctx.plan_mode_ask_tools, name);
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

fn is_bound_context_resource_read(name: &str, args: &Value, ctx: &ToolExecContext) -> bool {
    if name != TOOL_READ_CONTEXT_RESOURCE {
        return false;
    }
    let Some(resource_ref) = args
        .get("resource_ref")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    ctx.context_resource_refs.iter().any(|resource| {
        resource.resource_ref == resource_ref
            && ctx.session_id.as_deref() == Some(resource.parent_session_id.as_str())
            && ctx.turn_id.as_deref() == resource.parent_turn_id.as_deref()
            && ctx.agent_id.as_deref() == Some(resource.principal_agent_id.as_str())
    })
}

async fn capture_mac_control_approval_focus_anchor(
    name: &str,
) -> Option<super::MacControlFocusAnchor> {
    // 特征 crate 钩子：未 wire（无 ha-mac）恒 None——此时 mac_control 也
    // 不可分发，焦点保护无对象。
    if name == TOOL_MAC_CONTROL {
        let hooks = super::mac_control_exec_hooks()?;
        (hooks.capture_focus)().await
    } else {
        None
    }
}

async fn restore_mac_control_approval_focus_anchor(anchor: Option<super::MacControlFocusAnchor>) {
    let Some(anchor) = anchor else {
        return;
    };
    // anchor 只在钩子已注册时产生，此处必有钩子；防御式再取一次。
    let Some(hooks) = super::mac_control_exec_hooks() else {
        return;
    };
    if let Err(error) = (hooks.restore_focus)(anchor).await {
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
    // have run it.
    //
    // Read it off the context rather than re-querying: this fires per TOOL CALL,
    // and `effective_session_working_dir` is a synchronous `SessionDB::get_session`
    // on the exclusive writer connection. `ctx.session_working_dir` already holds
    // the same value (the `PostToolUse` twin in `streaming_loop` uses it), so the
    // re-query was one blocking DB round-trip per call for nothing.
    if !crate::hooks::scopes::any_handlers_for(
        HookEvent::PreToolUse,
        ctx.session_working_dir.as_deref().map(std::path::Path::new),
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
pub async fn run_tool_approval(
    name: &str,
    args: &Value,
    ctx: &ToolExecContext,
    reason_payload: Option<approval::ApprovalReasonPayload>,
    allow_always_forbidden: bool,
    desc_override: Option<String>,
) -> anyhow::Result<approval::ApprovalOrigin> {
    // `allow_always_forbidden` 由调用方传入，而本函数自阶段 5 起是 `pub`
    // （迁出的 adapter 如 ha-cron 的 `manage_cron` 要用）。参数因此**只允许
    // 收紧、不允许放松**：与 payload 自带的 strict 位取 `||`，否则 crate 外
    // 的调用方对一个 strict reason 传 `false`，就能给它开出 AllowAlways 持久
    // 化——而 AGENTS 明写判定源是 `AskReason::forbids_allow_always`
    // （`ApprovalReasonKind::is_strict` 是它的镜像，两者有断言守着）。
    let allow_always_forbidden = allow_always_forbidden
        || reason_payload
            .as_ref()
            .is_some_and(|payload| payload.kind.is_strict());
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
        Some(name),
        Some(args),
        ctx.tool_call_id.as_deref(),
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

    // MCP catalogs retain historical names as protocol aliases. Normalize the
    // runtime call before *every* gate so a deny/allow/hook/permission rule on
    // the current canonical tool cannot be bypassed through an old identifier.
    // The orchestrator still owns the submitted name in persisted history.
    let canonical_mcp_name = canonical_mcp_execution_name(name);
    let name = canonical_mcp_name.as_ref();

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
                    ctx.emit_effective_args(ui.clone()).await?;
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
        // 特征 crate 钩子：未 wire 时直通——mac_control 彼时不可分发，
        // sanitize/preflight 防御无对象。
        if let Some(hooks) = super::mac_control_exec_hooks() {
            sanitized_args = (hooks.sanitize_args)(args);
            if let Some(error) = (hooks.preflight_args)(&sanitized_args) {
                return Err(anyhow::anyhow!(error));
            }
            &sanitized_args
        } else {
            args
        }
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
        ctx.emit_effective_args(migrated.clone()).await?;
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
                    Some(name),
                    // The fully-shadowed args: PreToolUse `updatedInput`, then
                    // mac_control sanitize, then the exec legacy-background
                    // migrate. This is what the permission engine actually
                    // evaluated and denied, and it matches what PostToolUse and
                    // history report (`emit_effective_args` pushes the same
                    // value). It deliberately does NOT match the PreToolUse
                    // payload: that hook is the *producer* of `updatedInput`, so
                    // it necessarily ran before the rewrite. A script
                    // reconciling the two on `tool_use_id` will see different
                    // `tool_input` whenever a PreToolUse hook rewrote the call.
                    Some(args),
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
        let encoded_args = args.to_string();
        let args_fingerprint: String = crate::cache_routing::audit_fingerprint(
            "tool-execution-arguments",
            encoded_args.as_bytes(),
        )
        .chars()
        .take(16)
        .collect();
        logger.log(
            "info",
            "tool",
            &format!("tools::{}", name),
            &format!("Tool '{}' started", name),
            Some(
                serde_json::json!({
                    "arguments_size_bytes": encoded_args.len(),
                    "arguments_fingerprint": args_fingerprint,
                })
                .to_string(),
            ),
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
        // The foreground orchestrator admits ordinary text only after
        // PostToolUse. This execution-layer step now materializes media only.
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
        // 阶段 2.5：静态 match 已反转为注册表查表（builtin_registry.rs 持有
        // 全部内置条目，特征 crate 经 registry::register_external_tools 在
        // 装配期追加）。MCP 逃逸口保持原位：`mcp__<server>__<tool>` 前缀走
        // 专属子系统，不进注册表。
        if let Some(tool) = super::registry::lookup(name) {
            (tool.handler)(args, dispatch_ctx).await
        } else if crate::mcp::catalog::is_mcp_tool_name(name) {
            crate::mcp::invoke::call_tool(name, args, dispatch_ctx).await
        } else {
            Err(anyhow::anyhow!("Unknown tool: {}", name))
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

    // Log content-free execution diagnostics only. The hook-before body and
    // provider error text may contain credentials or private paths.
    if let Some(logger) = crate::get_logger() {
        match &result {
            Ok(output) => {
                let fingerprint: String = crate::cache_routing::audit_fingerprint(
                    "tool-execution-output",
                    output.as_bytes(),
                )
                .chars()
                .take(16)
                .collect();
                logger.log(
                    "info",
                    "tool",
                    &format!("tools::{}", name),
                    &format!("Tool '{}' completed in {}ms", name, duration_ms),
                    Some(
                        serde_json::json!({
                            "duration_ms": duration_ms,
                            "output_size_bytes": output.len(),
                            "output_fingerprint": fingerprint,
                        })
                        .to_string(),
                    ),
                    None,
                    None,
                );
            }
            Err(e) => {
                let error_text = e.to_string();
                let fingerprint: String = crate::cache_routing::audit_fingerprint(
                    "tool-execution-error",
                    error_text.as_bytes(),
                )
                .chars()
                .take(16)
                .collect();
                logger.log(
                    "error",
                    "tool",
                    &format!("tools::{}", name),
                    &format!("Tool '{}' failed in {}ms", name, duration_ms),
                    Some(
                        serde_json::json!({
                            "duration_ms": duration_ms,
                            "error_size_bytes": error_text.len(),
                            "error_fingerprint": fingerprint,
                        })
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

// ── Media materialization ─────────────────────────────────────────
// Ordinary text is deliberately returned unchanged. Its only admission point
// is the PostToolUse-effective ResultStore writer in streaming_loop; keeping a
// text spill here would reintroduce a hook-before raw/path bypass.
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
    Ok(output)
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

    let catalog =
        crate::skills_hooks::invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);
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
    let catalog =
        crate::skills_hooks::invocable_skills(&cfg.extra_skills_dirs, &cfg.disabled_skills);
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

#[cfg(test)]
mod tests {
    use super::{
        build_persisted_large_result_preview, canonical_mcp_execution_name_from,
        decide_async_path_with_config, exec_process_background_mode, execute_tool_with_context,
        is_bound_context_resource_read, maybe_persist_large_tool_result,
        migrate_exec_process_mode_to_async_job_args, needs_permission_engine,
        resolve_tool_permission, should_migrate_exec_process_mode_to_async_job_with_config,
        should_run_exec_reorder_gate, tool_timeout, validate_async_background_contract,
        AsyncDecision, JobOrigin, ToolExecContext,
    };
    use crate::agent_config::AsyncToolPolicy;
    use crate::mcp::{McpServerConfig, McpTransportSpec, McpTrustLevel};
    use crate::tool_defs::{EffectiveArgsSink, SessionDbHandle};
    use crate::tools::image_markers::IMAGE_FILE_PREFIX;
    use crate::tools::IMAGE_BASE64_PREFIX;
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
    fn historical_mcp_alias_resolves_to_canonical_runtime_name() {
        let legacy = "mcp__alpha__historical_name";
        let canonical = "mcp__alpha__current_full_name";

        assert_eq!(
            canonical_mcp_execution_name_from(legacy, Some(canonical.to_string())),
            canonical
        );
        assert_eq!(canonical_mcp_execution_name_from(legacy, None), legacy);
    }

    #[tokio::test]
    async fn canonical_mcp_deny_applies_to_historical_alias() {
        let legacy = "mcp__alpha__historical_name";
        let canonical = "mcp__alpha__current_full_name";
        let ctx = ToolExecContext {
            denied_tools: vec![canonical.to_string()],
            ..ToolExecContext::default()
        };

        let error = ctx
            .tool_visibility_error_for_canonical(legacy, canonical)
            .await
            .expect("canonical deny must block a historical MCP alias");
        assert!(error.contains("denied"), "unexpected error: {error}");
    }

    #[tokio::test]
    async fn session_continue_bypasses_context_tool_filters() {
        let ctx = ToolExecContext {
            denied_tools: vec![crate::tools::TOOL_SESSION_CONTINUE.to_string()],
            skill_allowed_tools: vec![crate::tools::TOOL_READ.to_string()],
            plan_mode_allowed_tools: vec![crate::tools::TOOL_WRITE.to_string()],
            ..ToolExecContext::default()
        };

        assert!(ctx
            .tool_visibility_error(crate::tools::TOOL_SESSION_CONTINUE)
            .await
            .is_none());
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
            crate::session::SessionDB::open_ephemeral_for_test(&dir.path().join("sessions.db"))
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
    fn explicit_async_job_cannot_wrap_self_managed_work() {
        for (tool, action) in [
            (crate::tools::TOOL_SUBAGENT, "spawn"),
            (crate::tools::TOOL_WORKFLOW, "create"),
        ] {
            let err = validate_async_background_contract(
                tool,
                &json!({
                    "action": action,
                    "task": "inspect the repository",
                    "run_in_background": true
                }),
            )
            .expect_err("self-managed work must not be nested inside a generic job");

            let message = err.to_string();
            assert!(message.contains("manages its own"));
            assert!(message.contains("durable handle"));
            assert!(message.contains("remove `run_in_background`"));
        }
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
    fn ordinary_large_text_is_not_spilled_before_post_tool_use() {
        let output = "x".repeat(100_000);
        let result =
            maybe_persist_large_tool_result("read", output.clone(), &ToolExecContext::default())
                .expect("plain text passthrough");
        assert_eq!(result, output);
        assert!(!result.contains("saved to:"));
    }

    #[test]
    fn trusted_mcp_auto_approve_config_skips_regular_approval() {
        let cfg = mcp_cfg(true, McpTrustLevel::Trusted);
        assert!(crate::mcp::server_auto_approves_config(&cfg));
    }

    #[test]
    fn untrusted_mcp_auto_approve_is_rejected_and_not_honored() {
        // 保存期校验（Untrusted+auto_approve 拒绝）随 validate_server_config
        // 迁 ha-mcp，其 config 测试覆盖；这里守执行层第二道防线。
        let cfg = mcp_cfg(true, McpTrustLevel::Untrusted);
        assert!(!crate::mcp::server_auto_approves_config(&cfg));
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
    fn bound_context_resource_read_always_enters_permission_engine() {
        let mut ctx = ToolExecContext {
            session_id: Some("session-1".into()),
            turn_id: Some("turn-1".into()),
            agent_id: Some("agent-1".into()),
            context_resource_refs: vec![crate::prompt_context::ContextResourceRef {
                resource_ref: "ctxref-1".into(),
                mention_id: "mention-1".into(),
                target_id: "notes/demo.txt".into(),
                file_name: "demo.txt".into(),
                mime_type: "text/plain".into(),
                parent_session_id: "session-1".into(),
                parent_turn_id: Some("turn-1".into()),
                principal_agent_id: "agent-1".into(),
                bytes: Arc::from(&b"frozen"[..]),
                turn_budget: Arc::new(crate::prompt_context::ContextResourceTurnBudget::default()),
            }],
            auto_approve_tools: true,
            ..ToolExecContext::default()
        };
        let args = json!({"resource_ref": "ctxref-1"});

        assert!(is_bound_context_resource_read(
            crate::tool_defs::TOOL_READ_CONTEXT_RESOURCE,
            &args,
            &ctx,
        ));
        assert!(needs_permission_engine(
            crate::tool_defs::TOOL_READ_CONTEXT_RESOURCE,
            &args,
            &ctx,
            ctx.local_auto_approve(),
        ));

        ctx.turn_id = Some("other-turn".into());
        assert!(!is_bound_context_resource_read(
            crate::tool_defs::TOOL_READ_CONTEXT_RESOURCE,
            &args,
            &ctx,
        ));
        assert!(needs_permission_engine(
            crate::tool_defs::TOOL_READ_CONTEXT_RESOURCE,
            &args,
            &ctx,
            ctx.local_auto_approve(),
        ));
    }

    #[test]
    fn auto_approved_connector_action_still_runs_engine() {
        let ctx = ToolExecContext {
            auto_approve_tools: true,
            ..ToolExecContext::default()
        };
        assert!(needs_permission_engine(
            crate::tool_defs::feishu_names::TOOL_CALENDAR_CREATE_EVENT,
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
            crate::tool_defs::feishu_names::TOOL_CALENDAR_CREATE_EVENT,
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

    #[tokio::test]
    async fn live_plan_mode_blocks_same_round_cross_session_delegation() {
        let session_id = format!("plan-cross-session-{}", uuid::Uuid::new_v4());
        crate::plan::set_plan_state(&session_id, crate::plan::PlanModeState::Planning).await;
        let ctx = ToolExecContext {
            session_id: Some(session_id.clone()),
            // An empty snapshot models a turn that entered Plan Mode after its
            // tool schema had already been built.
            plan_mode_allowed_tools: Vec::new(),
            ..ToolExecContext::default()
        };

        for tool_name in [
            crate::tool_defs::TOOL_SESSIONS_CREATE,
            crate::tool_defs::TOOL_SESSIONS_SEND,
        ] {
            let decision = resolve_tool_permission(tool_name, &json!({}), &ctx, false).await;
            assert!(
                matches!(decision, crate::permission::Decision::Deny { .. }),
                "{tool_name} must not escape a same-round Plan Mode transition"
            );
        }

        crate::plan::set_plan_state(&session_id, crate::plan::PlanModeState::Off).await;
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

        // Wired sink: publishing waits until the orchestrator durably
        // acknowledges the rewritten input.
        let sink = Arc::new(EffectiveArgsSink::default());
        let ctx = ToolExecContext {
            effective_args_sink: Some(sink.clone()),
            ..ToolExecContext::default()
        };
        let publish = tokio::spawn(async move {
            ctx.emit_effective_args(json!({ "command": "echo safe" }))
                .await
        });
        let update = sink.next().await;
        assert!(
            !publish.is_finished(),
            "dispatch must remain paused before the durability acknowledgement"
        );
        assert_eq!(update.value.get("command"), Some(&json!("echo safe")),);
        let _ = update.acknowledged.send(Ok(()));
        publish.await.unwrap().unwrap();

        // No sink: emit is a no-op (no panic, nothing observable changes).
        let bare = ToolExecContext::default();
        bare.emit_effective_args(json!({ "ignored": true }))
            .await
            .unwrap();
        // No assertion needed beyond "did not panic" — the bare context has
        // no sink to inspect.
    }
}
