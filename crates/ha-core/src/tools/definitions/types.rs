use serde::Serialize;
use serde_json::{json, Value};

use super::super::{ToolProvider, ASYNC_JOB_TIMEOUT_ARG};
use super::metadata::ToolMetadata;

// ── Tool Tier (single source of truth for visibility / injection) ──

/// 4 层 + 2 特殊路径的工具分类。
///
/// `internal` / `deferred` / `always_load` 三个旧 bool 已删除，全部由 tier 派生
/// （[`ToolDefinition::is_internal`] / [`ToolDefinition::is_always_load`] /
/// [`ToolDefinition::is_deferred_default`]）。新增工具时只声明 tier，下游所有
/// 注入路径决策由 tier 驱动。
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolTier {
    /// Tier 1: 强制注入，UI 不显示开关。
    /// 子类用于注入路径分发，不影响"是否对用户可见"。
    Core { subclass: CoreSubclass },

    /// Tier 2: Agent 默认开/关，用户可关闭。
    Standard {
        /// 主 agent (`id == "ha-main"`) 的默认开关状态
        default_for_main: bool,
        /// 其他新建 agent 的默认开关状态
        default_for_others: bool,
        /// Legacy recommendation hint retained for config compatibility. V2
        /// permits every Standard tool to move into the deferred pool.
        default_deferred: bool,
    },

    /// Tier 3: 需要全局 provider / capability 配置。即使 agent 开了，没配也不真正注入；
    /// 此时在系统提示词的 `# Unconfigured Capabilities` 段提示用户去配置。
    Configured {
        default_for_main: bool,
        default_for_others: bool,
        /// Legacy recommendation hint retained for config compatibility. V2
        /// permits every Configured tool to move into the deferred pool.
        default_deferred: bool,
        /// 配置入口提示文案（用于 system prompt 的 # Unconfigured Capabilities 段）。
        /// `&'static str` 因为所有提示都是定义时的字面量。
        config_hint: &'static str,
    },

    /// 特殊：记忆工具，由全局 `memory.enabled` 控制注入。
    Memory,

    /// 特殊：MCP 内置元工具（mcp_resource / mcp_prompt），由 agent 的 mcp_enabled 控制。
    Mcp,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreSubclass {
    /// 文件 / shell（exec/process/read/write/edit/...）
    FileSystem,
    /// 交互（ask_user_question / send_attachment / task_*）
    Interaction,
    /// 跨会话（sessions_* / peek_sessions / agents_list）
    SessionAware,
    /// 框架元工具（tool_search / job_status / runtime_cancel / skill）
    Meta,
    /// Plan Mode 工具（enter_plan_mode / submit_plan）—— 由 PlanAgentMode 控制
    PlanMode,
}

impl CoreSubclass {
    pub fn as_str(self) -> &'static str {
        match self {
            CoreSubclass::FileSystem => "file_system",
            CoreSubclass::Interaction => "interaction",
            CoreSubclass::SessionAware => "session_aware",
            CoreSubclass::Meta => "meta",
            CoreSubclass::PlanMode => "plan_mode",
        }
    }
}

// ── Background execution semantics ──────────────────────────────

/// Durable work created and managed outside the generic async tool-job pool.
///
/// These kinds may share a read-only projection in the background task UI, but
/// their native store remains the execution truth and owns cancellation,
/// recovery, delivery, and concurrency limits.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DurableWorkKind {
    SubagentRun,
    WorkflowRun,
    AcpRun,
    AgentTeam,
}

impl DurableWorkKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SubagentRun => "subagent_run",
            Self::WorkflowRun => "workflow_run",
            Self::AcpRun => "acp_run",
            Self::AgentTeam => "agent_team",
        }
    }
}

/// Whether a tool invocation may be wrapped by the generic `async_jobs`
/// executor or already owns a durable asynchronous lifecycle of its own.
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum BackgroundPolicy {
    /// The invocation completes on the normal tool path and cannot opt into a
    /// generic background job.
    #[default]
    ForegroundOnly,
    /// The generic tool-job layer may detach the entire invocation and return a
    /// synthetic `job_id`.
    GenericJob,
    /// The tool creates or controls its own durable work handle. Wrapping it in
    /// `async_jobs` would create two IDs, two lifecycles, and duplicate delivery.
    SelfManaged { work_kind: DurableWorkKind },
}

impl BackgroundPolicy {
    pub const fn supports_generic_job(self) -> bool {
        matches!(self, Self::GenericJob)
    }

    pub const fn work_kind(self) -> Option<DurableWorkKind> {
        match self {
            Self::SelfManaged { work_kind } => Some(work_kind),
            Self::ForegroundOnly | Self::GenericJob => None,
        }
    }
}

/// Action-level behavior for composite self-managed tools.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DurableWorkOperation {
    Dispatch,
    Wait,
    Observe,
    Control,
    Manage,
}

#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum ToolInvocationSemantics {
    Inline,
    GenericJobEligible,
    SelfManaged {
        work_kind: DurableWorkKind,
        operation: DurableWorkOperation,
    },
}

// ── Tool Definition (provider-agnostic) ───────────────────────────

#[derive(Serialize, Clone, Debug)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool parameters
    pub parameters: Value,
    /// Tier classification — single source of truth for visibility/injection logic.
    pub tier: ToolTier,
    /// Internal capability tools never require user approval. Orthogonal to tier:
    /// `exec` / `write` / `read` are Tier 1 Core but NOT internal (they modify the
    /// system, need user consent). `recall_memory` / `sessions_list` / `task_list`
    /// are also Tier 1 Core but ARE internal (autonomous read-only abilities).
    #[serde(default)]
    pub internal: bool,
    /// Whether this tool can be safely executed concurrently with other concurrent-safe
    /// tools within a single tool round (read-only, no side effects). Migrated from
    /// the old `CONCURRENT_SAFE_TOOL_NAMES` static name list.
    #[serde(default)]
    pub concurrent_safe: bool,
    /// Background execution contract. Only `GenericJob` receives the
    /// `run_in_background` / `job_timeout_secs` schema and participates in the
    /// sync-execution auto-background budget. `SelfManaged` tools return their
    /// own durable handle and must never be wrapped by `async_jobs`.
    #[serde(default)]
    pub background_policy: BackgroundPolicy,
}

impl ToolDefinition {
    pub const fn supports_generic_job(&self) -> bool {
        self.background_policy.supports_generic_job()
    }

    /// Resolve action-level execution semantics without changing dispatch.
    /// This is descriptive metadata and an audit/testing contract; native
    /// executors remain authoritative for the actual lifecycle.
    pub fn invocation_semantics(&self, args: &Value) -> ToolInvocationSemantics {
        match self.background_policy {
            BackgroundPolicy::ForegroundOnly => ToolInvocationSemantics::Inline,
            BackgroundPolicy::GenericJob => ToolInvocationSemantics::GenericJobEligible,
            BackgroundPolicy::SelfManaged { work_kind } => ToolInvocationSemantics::SelfManaged {
                work_kind,
                operation: self.self_managed_operation(args),
            },
        }
    }

    fn self_managed_operation(&self, args: &Value) -> DurableWorkOperation {
        let action = args.get("action").and_then(Value::as_str).unwrap_or("");
        match self.name.as_str() {
            crate::tools::TOOL_SUBAGENT => match action {
                "spawn" | "resume" | "batch_spawn" | "spawn_and_wait" => {
                    DurableWorkOperation::Dispatch
                }
                "wait_all" => DurableWorkOperation::Wait,
                "check" if args.get("wait").and_then(Value::as_bool).unwrap_or(false) => {
                    DurableWorkOperation::Wait
                }
                "check" | "list" | "result" => DurableWorkOperation::Observe,
                "steer" | "kill" | "kill_all" => DurableWorkOperation::Control,
                "send" => DurableWorkOperation::Manage,
                _ => DurableWorkOperation::Manage,
            },
            crate::tools::TOOL_ACP_SPAWN => match action {
                "spawn" => DurableWorkOperation::Dispatch,
                "check" if args.get("wait").and_then(Value::as_bool).unwrap_or(false) => {
                    DurableWorkOperation::Wait
                }
                "check" | "list" | "result" | "backends" => DurableWorkOperation::Observe,
                "kill" | "kill_all" | "steer" => DurableWorkOperation::Control,
                _ => DurableWorkOperation::Manage,
            },
            crate::tools::TOOL_WORKFLOW => match action {
                "create" | "followup" => DurableWorkOperation::Dispatch,
                "list" | "status" | "trace" | "guide" => DurableWorkOperation::Observe,
                "control" => DurableWorkOperation::Control,
                _ => DurableWorkOperation::Manage,
            },
            crate::tools::TOOL_TEAM => match action {
                "create" | "add_member" => DurableWorkOperation::Dispatch,
                "status" | "list_tasks" | "list_members" | "list_templates" => {
                    DurableWorkOperation::Observe
                }
                "pause" | "resume" | "dissolve" | "remove_member" => DurableWorkOperation::Control,
                _ => DurableWorkOperation::Manage,
            },
            _ => DurableWorkOperation::Manage,
        }
    }

    /// Actions that can be exposed as compact call variants when this large
    /// composite tool is activated through the client-side deferred path.
    /// The canonical execution entry remains unchanged.
    pub fn call_variant_actions(&self) -> &'static [&'static str] {
        match self.name.as_str() {
            crate::tools::TOOL_BROWSER => &[
                "status", "profile", "tabs", "navigate", "snapshot", "act", "observe", "control",
            ],
            crate::tools::TOOL_MAC_CONTROL => &[
                "status",
                "permissions",
                "diagnostics",
                "snapshot",
                "visual",
                "elements",
                "wait",
                "apps",
                "dock",
                "spaces",
                "windows",
                "act",
                "menu",
                "clipboard",
                "dialog",
            ],
            crate::tools::TOOL_MANAGE_CRON => &[
                "create",
                "update",
                "list",
                "get",
                "delete",
                "pause",
                "resume",
                "run_now",
                "list_channel_targets",
                "list_projects",
            ],
            crate::tools::TOOL_APP_UPDATE => &["check", "install", "status", "rollback"],
            _ => &[],
        }
    }

    /// Render one action-scoped schema. Only model-facing schema names change;
    /// [`crate::tools::normalize_call_variant`] maps the call back to the
    /// canonical tool before visibility, permissions, hooks, audit, or dispatch.
    pub fn to_compact_call_variant(&self, action: &str, provider: ToolProvider) -> Option<Value> {
        if !self.call_variant_actions().contains(&action) {
            return None;
        }
        let fields = variant_fields(&self.name, action)?;
        let augmented = self.augmented_parameters();
        let all_properties = augmented.get("properties")?.as_object()?;
        let mut properties = serde_json::Map::new();
        for field in fields {
            if let Some(schema) = all_properties.get(*field) {
                properties.insert((*field).to_string(), schema.clone());
            }
        }
        let required = augmented
            .get("required")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|name| *name != "action" && properties.contains_key(*name))
                    .map(|name| Value::String(name.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let parameters = json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false,
        });
        let name = format!("{}__{}", self.name, action);
        let summary = crate::truncate_utf8(&self.description, 220);
        let description = format!(
            "{} Fixed action: `{}`. Executes and audits as `{}`.",
            summary, action, self.name
        );
        Some(match provider {
            ToolProvider::Anthropic => json!({
                "name": name,
                "description": description,
                "input_schema": parameters,
            }),
            ToolProvider::OpenAI => json!({
                "type": "function",
                "name": name,
                "description": description,
                "parameters": parameters,
            }),
        })
    }

    /// Internal capability tools never require user approval.
    pub fn is_internal(&self) -> bool {
        self.internal
    }

    /// Whether this built-in tool supports being moved into the deferred pool.
    pub fn supports_deferred(&self) -> bool {
        match &self.tier {
            ToolTier::Core { subclass } => {
                !matches!(subclass, CoreSubclass::PlanMode)
                    && !matches!(
                        self.name.as_str(),
                        crate::tools::TOOL_TOOL_SEARCH
                            | crate::tools::TOOL_ASK_USER_QUESTION
                            | crate::tools::TOOL_RUNTIME_CANCEL
                            | crate::tools::TOOL_SKILL
                    )
            }
            ToolTier::Memory => true,
            ToolTier::Standard { .. } | ToolTier::Configured { .. } => true,
            _ => false,
        }
    }

    /// 工具是否在 deferred 模式下也强制发送 schema（即"不延迟"）。
    /// Bootstrap / PlanMode / MCP 永远 always_load；其余由加载策略决定。
    pub fn is_always_load(&self) -> bool {
        !self.supports_deferred()
    }

    /// 工具是否支持进入 deferred 池（与 `is_always_load` 互斥）。
    pub fn is_deferred_default(&self) -> bool {
        self.supports_deferred()
    }

    /// 是否属于 Tier 1 Core（用于 system_prompt::build_tools_section 等过滤）。
    pub fn is_core(&self) -> bool {
        matches!(self.tier, ToolTier::Core { .. })
    }

    /// Rich v2 metadata used by tool search, UI metadata, workflow planning, and
    /// future review/workflow policies. This is intentionally derived as a sidecar
    /// instead of a required struct literal field so every existing built-in and
    /// dynamic MCP tool receives v2 metadata without forcing noisy definition
    /// churn or changing execution/permission behavior.
    pub fn v2_metadata(&self) -> ToolMetadata {
        ToolMetadata::for_definition(self)
    }

    /// Render this tool as a JSON metadata payload for `list_builtin_tools`
    /// (Tauri command + `GET /api/chat/tools`). Single source of truth so
    /// both transports return identically-shaped objects to the frontend.
    ///
    /// `app_config` is consulted only for Tier 3 (`Configured`) tools to
    /// probe whether the global provider/feature is provisioned. The
    /// returned `globally_configured` field is `Some(bool)` for Tier 3 and
    /// `null` for every other tier — letting the frontend decide whether to
    /// show the "未配置" hint without re-implementing the probe matrix.
    pub fn to_api_metadata(&self, app_config: &crate::config::AppConfig) -> Value {
        let (
            tier_label,
            core_subclass,
            default_for_main,
            default_for_others,
            config_hint,
            globally_configured,
        ) = match &self.tier {
            ToolTier::Core { subclass } => {
                ("core", Some(subclass.as_str()), None, None, None, None)
            }
            ToolTier::Standard {
                default_for_main,
                default_for_others,
                ..
            } => (
                "standard",
                None,
                Some(*default_for_main),
                Some(*default_for_others),
                None,
                None,
            ),
            ToolTier::Configured {
                default_for_main,
                default_for_others,
                config_hint,
                ..
            } => (
                "configured",
                None,
                Some(*default_for_main),
                Some(*default_for_others),
                Some(*config_hint),
                Some(super::super::dispatch::is_globally_configured(
                    &self.name, app_config,
                )),
            ),
            ToolTier::Memory => ("memory", None, None, None, None, None),
            ToolTier::Mcp => ("mcp", None, None, None, None, None),
        };
        json!({
            "name": self.name,
            "description": self.description,
            "internal": self.internal,
            "tier": tier_label,
            "core_subclass": core_subclass,
            "default_for_main": default_for_main,
            "default_for_others": default_for_others,
            "config_hint": config_hint,
            "defer_capable": self.supports_deferred(),
            "globally_configured": globally_configured,
            "background_policy": self.background_policy,
            "metadata": self.v2_metadata(),
        })
    }

    /// When this tool accepts generic async jobs, inject optional job control
    /// parameters into the JSON schema so the model can discover background
    /// execution and choose a shorter per-call job timeout when warranted.
    /// Idempotent.
    fn augmented_parameters(&self) -> Value {
        if !self.supports_generic_job() {
            return self.parameters.clone();
        }
        let mut params = self.parameters.clone();
        let Some(obj) = params.as_object_mut() else {
            return params;
        };
        let props = obj
            .entry("properties".to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        let Some(props_obj) = props.as_object_mut() else {
            return params;
        };
        if !props_obj.contains_key("run_in_background") {
            props_obj.insert(
                "run_in_background".to_string(),
                json!({
                    "type": "boolean",
                    "description": "Run in background and return immediately with a job_id. Set to true when: (1) the task is expected to take more than a few seconds (long builds, lengthy web searches, image generation, network-heavy operations), AND (2) you can make progress on other things while it runs, OR (3) the user explicitly asked you to continue working in parallel. Set to false (default) when you need the result to decide your very next step; do not background a tool and immediately call job_status just to wait. Results are auto-injected into the conversation as `<task-notification>` messages when ready; use job_status(job_id) only for a quick non-blocking snapshot after meaningful elapsed time or when the user asks for status. For ordinary long-running `exec`, prefer this over exec-native `background`/`yield_ms`; those legacy process-session flags are only for cases that truly need the `process` session surface."
                }),
            );
        }
        if !props_obj.contains_key(ASYNC_JOB_TIMEOUT_ARG) {
            props_obj.insert(
                ASYNC_JOB_TIMEOUT_ARG.to_string(),
                json!({
                    "type": "integer",
                    "minimum": 0,
                    "description": "Optional per-call timeout in seconds for the outer async background job. Omit by default so user/system timeout policy applies. It only applies if the call runs as an async job (explicitly or via auto-background). Use a positive value only when the user requested a per-job deadline or this specific background job should be shorter than the configured default. If asyncTools.maxJobSecs is positive, this can only shorten that hard limit. 0 means no per-call override."
                }),
            );
        }
        params
    }

    pub fn to_anthropic_schema(&self) -> Value {
        json!({
            "name": self.name,
            "description": self.description,
            "input_schema": self.augmented_parameters(),
        })
    }

    pub fn to_openai_schema(&self) -> Value {
        json!({
            "type": "function",
            "name": self.name,
            "description": self.description,
            "parameters": self.augmented_parameters(),
        })
    }

    pub fn to_provider_schema(&self, provider: ToolProvider) -> Value {
        match provider {
            ToolProvider::Anthropic => self.to_anthropic_schema(),
            ToolProvider::OpenAI => self.to_openai_schema(),
        }
    }
}

fn variant_fields(tool: &str, action: &str) -> Option<&'static [&'static str]> {
    match (tool, action) {
        (crate::tools::TOOL_BROWSER, "status") => Some(&[]),
        (crate::tools::TOOL_BROWSER, "profile") => Some(&[
            "op",
            "url",
            "executable_path",
            "headless",
            "profile",
            "run_in_background",
            "job_timeout_secs",
        ]),
        (crate::tools::TOOL_BROWSER, "tabs") => Some(&["op", "target_id", "url", "steal", "keep"]),
        (crate::tools::TOOL_BROWSER, "navigate") => Some(&["op", "url", "target_id"]),
        (crate::tools::TOOL_BROWSER, "snapshot") => Some(&[
            "format",
            "ref",
            "target_id",
            "full_page",
            "image_format",
            "annotate",
            "output_path",
            "paper_format",
            "landscape",
            "print_background",
        ]),
        (crate::tools::TOOL_BROWSER, "act") => Some(&[
            "kind",
            "ref",
            "target_ref",
            "text",
            "key",
            "file_path",
            "values",
            "target_id",
        ]),
        (crate::tools::TOOL_BROWSER, "observe") => Some(&["kind", "target_id", "since"]),
        (crate::tools::TOOL_BROWSER, "control") => Some(&[
            "op",
            "target_id",
            "text",
            "expression",
            "method",
            "params",
            "download_id",
            "width",
            "height",
            "direction",
            "amount",
            "timeout",
            "accept",
            "dialog_text",
        ]),
        (crate::tools::TOOL_MAC_CONTROL, "status")
        | (crate::tools::TOOL_MAC_CONTROL, "permissions") => Some(&[]),
        (crate::tools::TOOL_MAC_CONTROL, "diagnostics") => Some(&["op", "limit", "path"]),
        (crate::tools::TOOL_MAC_CONTROL, "snapshot") => Some(&[
            "windowId",
            "includeScreenshot",
            "screenshotTarget",
            "displayId",
            "maxElements",
            "maxDepth",
        ]),
        (crate::tools::TOOL_MAC_CONTROL, "visual") => Some(&[
            "op",
            "snapshotId",
            "coordinateSpace",
            "x",
            "y",
            "text",
            "textMatch",
            "languages",
            "minConfidence",
            "recognitionLevel",
            "includeOcr",
            "annotate",
            "uiMapLimit",
            "screenshotTarget",
            "displayId",
            "windowId",
            "limit",
            "maxElements",
            "maxDepth",
        ]),
        (crate::tools::TOOL_MAC_CONTROL, "elements") => Some(&["op", "target", "limit"]),
        (crate::tools::TOOL_MAC_CONTROL, "wait") => Some(&["op", "target", "timeoutMs", "pollMs"]),
        (crate::tools::TOOL_MAC_CONTROL, "apps") => Some(&[
            "op",
            "appName",
            "appNameMatch",
            "bundleId",
            "pid",
            "limit",
            "force",
        ]),
        (crate::tools::TOOL_MAC_CONTROL, "dock") => Some(&[
            "op",
            "appName",
            "appNameMatch",
            "bundleId",
            "dockItemId",
            "itemPath",
            "menuIndex",
            "menuItem",
        ]),
        (crate::tools::TOOL_MAC_CONTROL, "spaces") => {
            Some(&["op", "spaceId", "spaceIndex", "direction", "windowId"])
        }
        (crate::tools::TOOL_MAC_CONTROL, "windows") => {
            Some(&["op", "windowScope", "windowId", "x", "y", "width", "height"])
        }
        (crate::tools::TOOL_MAC_CONTROL, "act") => Some(&[
            "op",
            "target",
            "x",
            "y",
            "fromX",
            "fromY",
            "toX",
            "toY",
            "toTarget",
            "text",
            "typingProfile",
            "dryRunOp",
            "explain",
            "typingDelayMs",
            "value",
            "axAction",
            "key",
            "keys",
            "modifiers",
            "repeat",
            "holdMs",
            "intervalMs",
            "deltaX",
            "deltaY",
            "durationMs",
            "steps",
            "motionProfile",
            "verify",
        ]),
        (crate::tools::TOOL_MAC_CONTROL, "menu") => Some(&[
            "op",
            "scope",
            "path",
            "menuIndex",
            "appHint",
            "includeOcr",
            "languages",
            "minConfidence",
            "recognitionLevel",
            "limit",
        ]),
        (crate::tools::TOOL_MAC_CONTROL, "clipboard") => Some(&["op", "text", "maxChars"]),
        (crate::tools::TOOL_MAC_CONTROL, "dialog") => Some(&[
            "op",
            "path",
            "buttonText",
            "field",
            "fieldIndex",
            "text",
            "clear",
            "filePath",
            "fileName",
            "selectButton",
            "ensureExpanded",
            "force",
        ]),
        (crate::tools::TOOL_MANAGE_CRON, "create") => Some(&[
            "name",
            "description",
            "schedule_type",
            "timestamp",
            "interval_ms",
            "start_at",
            "cron_expression",
            "timezone",
            "prompt",
            "agent_id",
            "project_id",
            "max_failures",
            "job_timeout_secs",
            "notify_on_complete",
            "prefix_delivery_with_name",
            "delivery_targets",
        ]),
        (crate::tools::TOOL_MANAGE_CRON, "update") => Some(&[
            "id",
            "name",
            "description",
            "schedule_type",
            "timestamp",
            "interval_ms",
            "start_at",
            "cron_expression",
            "timezone",
            "prompt",
            "agent_id",
            "project_id",
            "max_failures",
            "job_timeout_secs",
            "notify_on_complete",
            "prefix_delivery_with_name",
            "delivery_targets",
        ]),
        (crate::tools::TOOL_MANAGE_CRON, "list")
        | (crate::tools::TOOL_MANAGE_CRON, "list_channel_targets") => Some(&[]),
        (crate::tools::TOOL_MANAGE_CRON, "list_projects") => Some(&["include_archived"]),
        (crate::tools::TOOL_MANAGE_CRON, "get")
        | (crate::tools::TOOL_MANAGE_CRON, "delete")
        | (crate::tools::TOOL_MANAGE_CRON, "pause")
        | (crate::tools::TOOL_MANAGE_CRON, "resume")
        | (crate::tools::TOOL_MANAGE_CRON, "run_now") => Some(&["id"]),
        (crate::tools::TOOL_APP_UPDATE, "check") | (crate::tools::TOOL_APP_UPDATE, "rollback") => {
            Some(&[])
        }
        (crate::tools::TOOL_APP_UPDATE, "install") => Some(&[
            "target_version",
            "prefer_path",
            "run_in_background",
            "job_timeout_secs",
        ]),
        (crate::tools::TOOL_APP_UPDATE, "status") => Some(&["job_id"]),
        _ => None,
    }
}

#[cfg(test)]
mod call_variant_tests {
    use super::*;

    #[test]
    fn large_tool_variant_is_bounded_and_omits_action() {
        for definition in crate::tools::dispatch::all_dispatchable_tools()
            .iter()
            .filter(|definition| !definition.call_variant_actions().is_empty())
        {
            let full_bytes = serde_json::to_vec(&definition.to_openai_schema())
                .unwrap()
                .len();
            for action in definition.call_variant_actions() {
                let variant = definition
                    .to_compact_call_variant(action, ToolProvider::OpenAI)
                    .expect("registered variant");
                let variant_bytes = serde_json::to_vec(&variant).unwrap().len();
                assert!(variant_bytes < full_bytes || full_bytes < 1_500 * 4);
                assert!(
                    variant_bytes / crate::context_compact::CHARS_PER_TOKEN <= 3_000,
                    "{}__{} exceeds 3k token heuristic: {variant_bytes} bytes",
                    definition.name,
                    action
                );
                assert_eq!(variant["name"], format!("{}__{}", definition.name, action));
                assert!(variant["parameters"]["properties"].get("action").is_none());
            }
        }
    }

    #[test]
    fn variant_normalizes_to_canonical_and_fixed_action_wins() {
        let (name, args) = crate::tools::normalize_call_variant(
            "browser__snapshot",
            &serde_json::json!({ "action": "act", "format": "role" }),
        )
        .expect("known variant");
        assert_eq!(name, crate::tools::TOOL_BROWSER);
        assert_eq!(args["action"], "snapshot");
        assert_eq!(args["format"], "role");
    }
}
