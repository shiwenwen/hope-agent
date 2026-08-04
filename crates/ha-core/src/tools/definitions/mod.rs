//! 工具 schema **目录层**——认识全部工具、反向依赖 dispatch 与特征
//! facade（media_gen / mcp / awareness），随 `tools/` 走。
//!
//! 共享的 schema **类型**（`ToolDefinition` / `BackgroundPolicy` /
//! metadata 家族）与单工具 schema 构造器已归位 `crate::tool_defs`
//! （crate-split 阶段 4）；本目录只剩三个「全表汇编」文件：
//! `core_tools`（Core 工具总表）、`special_tools`（动态 schema）、
//! `registry`（从 ToolDefinition 派生的只读查询缓存）。

mod core_tools;
mod registry;
mod special_tools;

// 已归位 tool_defs 的兄弟模块别名——core_tools / special_tools / registry
// 内部的 `super::goal_tools::…` 等既有路径不变。
pub(crate) use crate::tool_defs::{
    goal_tools, loop_tools, plan_tools, task_tools, types, update_tools,
};
// 类型与单工具构造器的兼容再导出——tools/ 内部既有 `definitions::X` /
// `super::definitions::X` 路径不变（定义处在 tool_defs）。
pub(crate) use crate::tool_defs::get_design_tool;
pub use crate::tool_defs::{
    get_artifact_tool, get_canvas_tool, get_enter_plan_mode_tool, get_notification_tool,
    get_submit_plan_tool, get_web_search_tool, BackgroundPolicy, CoreSubclass, DurableWorkKind,
    ToolDefinition, ToolMetadata, ToolTier,
};

// ── Public Re-exports ─────────────────────────────────────────────

pub use core_tools::get_available_tools;
pub use registry::{
    background_policy_for_tool, get_core_tools, get_core_tools_for_provider, get_deferred_tools,
    get_tools_for_provider, is_concurrent_safe, is_generic_job_capable, is_internal_tool,
};
pub use special_tools::{
    get_acp_spawn_tool, get_audio_generate_tool_dynamic, get_image_generate_tool_dynamic,
    get_subagent_tool, get_tool_search_tool, get_workflow_tool,
};
