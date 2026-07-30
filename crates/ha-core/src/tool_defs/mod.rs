//! 工具契约层（tool spec）——`tools/`（分发注册表 + adapter 目录）与
//! kernel 其余模块共享的类型、常量与纯谓词的**唯一定义处**。
//!
//! # 分层契约（crate-split 阶段 4「tools 类型归位」）
//!
//! `tools/` 是分发注册表：它认识每个工具 adapter，未来随特征 crate 上浮。
//! kernel 模块（agent / async_jobs / permission / system_prompt /
//! context_compact …）需要的共享物——工具名常量、schema 类型、执行上下文
//! [`ToolExecContext`]、拒绝语义 [`ToolRejection`]——全部归位到本模块，
//! kernel 侧一律 `crate::tool_defs::…` 引用，**不得**再指向 `crate::tools`。
//!
//! 本模块面向低层：只依赖 config / agent_config / permission / knowledge /
//! session / hooks 等 kernel 设施，**绝不依赖 `tools/` 的 dispatch /
//! registry / adapter**。schema 目录（`core_tools` / `special_tools` /
//! `definitions::registry` 派生缓存）反向依赖 dispatch 与特征 facade，
//! 属于目录层，留在 `tools/definitions/`。
//!
//! `crate::tools` 门面 `pub use crate::tool_defs::*` 全量再导出——crate 外
//! （特征 crate / 壳层）的 `ha_core::tools::…` 既有路径全部不变。

// 子模块一律 `pub(crate)`：对外只暴露下方 `pub use` 的具体 item（经
// `crate::tools` 门面 glob 保持与迁移前逐字相同的 crate 外符号集），
// 不新增可被外部依赖的模块路径。
pub(crate) mod context;
pub(crate) mod extra_tools;
pub(crate) mod goal_tools;
pub(crate) mod loop_tools;
pub(crate) mod mac_control;
pub(crate) mod metadata;
pub(crate) mod names;
pub(crate) mod plan_tools;
pub(crate) mod rejection;
pub(crate) mod scope;
pub(crate) mod task_tools;
pub(crate) mod types;
pub(crate) mod update_tools;

#[doc(hidden)]
pub use context::EffectiveArgsSink;
pub use context::{
    audit_model_runtime_timeout_override, emit_model_runtime_timeout_metadata,
    should_ignore_model_runtime_timeout_when_user_unlimited, ApprovalOrigin, PidSink,
    SessionDbHandle, ToolExecContext,
};
pub use extra_tools::{
    get_artifact_tool, get_canvas_tool, get_notification_tool, get_web_search_tool,
};
// crate 内可见即可——迁移前 `ha_core::tools::get_design_tool` 就不解析
// （`definitions` 是私有模块），门面 glob 不得顺手把它放开。
pub(crate) use extra_tools::get_design_tool;
pub use mac_control::{normalize_perform_ax_action, MacControlFocusAnchor};
pub use metadata::{
    ToolApprovalHint, ToolEffect, ToolInputMetadata, ToolInterruptBehavior, ToolMetadata,
    ToolPathExtractorMetadata, ToolPermissionMetadata, ToolPermissionSubject, ToolRenderMetadata,
    ToolResultKind, ToolRisk, ToolValidationMetadata,
};
pub use names::*;
pub use plan_tools::{get_ask_user_question_tool, get_enter_plan_mode_tool, get_submit_plan_tool};
pub use rejection::{ToolRejection, TOOL_ERROR_PREFIX};
pub use scope::{
    is_design_scope_tool, is_kb_scoped_tool, is_knowledge_scope_tool, is_memory_tool,
    tool_visible_with_filters, ToolScope,
};
pub use types::{
    BackgroundPolicy, CoreSubclass, DurableWorkKind, DurableWorkOperation, ToolDefinition,
    ToolInvocationSemantics, ToolTier,
};

// ── Provider Enum ─────────────────────────────────────────────────

/// Supported LLM provider types for tool schema adaptation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProvider {
    Anthropic,
    OpenAI,
}

/// Expand ~ and ~/ to home directory.
pub fn expand_tilde(path: &str) -> String {
    if path == "~" || path.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return if path == "~" {
                home.to_string_lossy().to_string()
            } else {
                home.join(&path[2..]).to_string_lossy().to_string()
            };
        }
    }
    path.to_string()
}
