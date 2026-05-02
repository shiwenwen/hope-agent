use serde::Serialize;
use serde_json::{json, Value};

use super::super::ToolProvider;

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
        /// 主 agent (`id == "default"`) 的默认开关状态
        default_for_main: bool,
        /// 其他新建 agent 的默认开关状态
        default_for_others: bool,
        /// true 表示该工具支持被用户放入延迟加载池。
        default_deferred: bool,
    },

    /// Tier 3: 需要全局 provider / capability 配置。即使 agent 开了，没配也不真正注入；
    /// 此时在系统提示词的 `# Unconfigured Capabilities` 段提示用户去配置。
    Configured {
        default_for_main: bool,
        default_for_others: bool,
        /// true 表示该工具支持被用户放入延迟加载池。
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
    /// Async-capable tools may be backgrounded: the model sets `run_in_background: true`
    /// in the arguments and the tool returns an immediate synthetic job_id. The real
    /// execution continues in a tokio task and the result is delivered to the parent
    /// session via the async_jobs injection pipeline when the session becomes idle.
    /// Also participates in the sync-execution auto-background budget.
    #[serde(default)]
    pub async_capable: bool,
}

impl ToolDefinition {
    /// Internal capability tools never require user approval.
    pub fn is_internal(&self) -> bool {
        self.internal
    }

    /// Whether this built-in tool supports being moved into the deferred pool.
    pub fn supports_deferred(&self) -> bool {
        match &self.tier {
            ToolTier::Standard {
                default_deferred, ..
            }
            | ToolTier::Configured {
                default_deferred, ..
            } => *default_deferred,
            _ => false,
        }
    }

    /// 工具是否在 deferred 模式下也强制发送 schema（即"不延迟"）。
    /// Core / Memory / Mcp 永远 always_load；
    /// Standard / Configured 只有支持 deferred 的工具才可能不 always_load。
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
        })
    }

    /// When this tool is async-capable, inject an optional `run_in_background`
    /// parameter into the tool's JSON schema so the model can discover and opt
    /// into background execution. Idempotent.
    fn augmented_parameters(&self) -> Value {
        if !self.async_capable {
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
        if props_obj.contains_key("run_in_background") {
            return params;
        }
        props_obj.insert(
            "run_in_background".to_string(),
            json!({
                "type": "boolean",
                "description": "Run in background and return immediately with a job_id. Set to true when: (1) the task is expected to take more than a few seconds (long builds, lengthy web searches, image generation, network-heavy operations), AND (2) you can make progress on other things while it runs, OR (3) the user explicitly asked you to continue working in parallel. Set to false (default) when you need the result to decide your very next step. Results are auto-injected into the conversation when ready; you can also call job_status(job_id, block=true) to actively wait."
            }),
        );
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
