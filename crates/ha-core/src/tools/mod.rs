//! 工具分发注册表 + 内置 adapter 目录。
//!
//! 共享的类型 / 常量 / 纯谓词（`TOOL_*` 名字常量、`ToolDefinition` 家族、
//! [`ToolExecContext`]、`ToolScope`、`ToolRejection` …）的定义处是
//! [`crate::tool_defs`]（crate-split 阶段 4「tools 类型归位」）；本模块
//! 末尾全量再导出，crate 外（特征 crate / 壳层）的 `ha_core::tools::…`
//! 既有路径不变。kernel 内部新代码请直接 `crate::tool_defs::…`。

use serde_json::Value;

mod agents;
mod apply_patch;
pub(crate) mod approval;
// pub：ha-pet 活动快照消费（pending 审批聚合只读面；approval 模块本体保持
// crate 内私有）。
pub use approval::{pending_approvals_per_session, ApprovalReasonPayload, SessionApprovalAgg};
// pub：`ask_user_question::execute` 是结构化问答唯一入口（AGENTS.md 红线），
// 特征 crate 的工具 adapter（如 ha-updater 的 `app_update` install/rollback
// 确认）从 crate 外复用同一入口，不 fork。
pub mod ask_user_question;
mod builtin_registry;
mod context_resource;
#[cfg(test)]
pub(crate) use context_resource::tool_read_context_resource;
mod core_memory;
// `manage_cron` adapter 已随 ha-cron 迁出（§3.2 adapter 随特征走）；
// schema 仍在 definitions::core_tools，handler 由 ha_cron::wire() 注册。
mod definitions;
// pub：ha-design 的 code_sync 复用 diff 摘要工具（不 fork）。
pub mod diff_util;
pub mod dispatch;
mod edit;
mod enter_plan_mode;
pub(crate) mod exec;
mod execution;
mod find;
mod goal;
mod grep;
pub(crate) mod image;
// pub：ha-mac 的截图结果复用 image marker 构建（工具结果图片标记契约）。
pub mod image_markers;
mod issue_report;
pub(crate) mod job_status;
mod loop_tool;
mod ls;
mod lsp;
mod memory;
mod notification;
pub(crate) mod pdf;
mod process;
mod project_memory;
pub(crate) mod read;
pub mod registry;
mod result_store;
mod runtime_cancel;
mod schedule_wakeup;
mod send_attachment;
mod session_continue;
mod sessions;
mod settings;
pub(crate) mod subagent;
mod submit_plan;
mod task;
pub(crate) mod team;
pub(crate) mod tool_search;
pub mod web_fetch;
pub mod web_fetch_common;
pub mod web_search;
mod workflow_tool;
mod write;

// ── 特征 crate 钩子：天气设置热刷新 ──────────────────────────────
//
// weather 迁出为特征 crate 后，settings 写路径命中天气相关 key 时经此钩子
// 触发即时刷新：未装配（未 wire）＝不刷——此时天气特征整体不存在（后台
// 循环也是 wire() 注册的），fail-soft 语义自洽。注意 server 形态本钩子是
// **唯一**刷新路径（周期循环带 desktop 门），与迁移前行为一致。钩子是纯
// `fn()`——spawn 与错误日志由注册方（ha-weather）自带，与迁移前
// settings.rs 内联的 tokio::spawn + app_warn 逐字等价。
static WEATHER_SETTINGS_REFRESH: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// 特征 crate 装配期注册天气设置热刷新回调。重复注册返回 `Err`。
pub fn register_weather_settings_refresh(hook: fn()) -> Result<(), crate::AlreadyRegistered> {
    WEATHER_SETTINGS_REFRESH
        .set(hook)
        .map_err(|_| crate::AlreadyRegistered("weather settings refresh hook"))
}

pub(crate) fn weather_settings_refresh_hook() -> Option<fn()> {
    WEATHER_SETTINGS_REFRESH.get().copied()
}

// ── 特征 crate 钩子：mac_control 执行层 ──────────────────────────
//
// 四件套原子注册（部分注册＝执行层防御残缺，不允许）：审批焦点 capture/
// restore（macOS AX 调用在 ha-mac）+ 工具参数 sanitize/preflight（#247
// 执行层防御）。未装配（未 wire）＝anchor 恒 None、args 原样直通——此时
// mac_control handler 也不存在（dispatch 报 Unknown tool），防御无对象，
// fail-soft 自洽。类型 `MacControlFocusAnchor` 与纯函数
// `normalize_perform_ax_action` 在 `tool_defs::mac_control`（审批分类
// 代码不外迁红线），此处经末尾全量再导出保 ha-mac 原路径。
pub struct MacControlExecHooks {
    pub capture_focus: fn() -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Option<MacControlFocusAnchor>> + Send>,
    >,
    pub restore_focus: fn(
        MacControlFocusAnchor,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<(), String>> + Send>,
    >,
    pub sanitize_args: fn(&Value) -> Value,
    pub preflight_args: fn(&Value) -> Option<String>,
}

static MAC_CONTROL_EXEC_HOOKS: std::sync::OnceLock<MacControlExecHooks> =
    std::sync::OnceLock::new();

/// 特征 crate 装配期注册 mac_control 执行层钩子。重复注册返回 `Err`。
pub fn register_mac_control_exec_hooks(
    hooks: MacControlExecHooks,
) -> Result<(), crate::AlreadyRegistered> {
    MAC_CONTROL_EXEC_HOOKS
        .set(hooks)
        .map_err(|_| crate::AlreadyRegistered("mac_control exec hooks"))
}

pub(crate) fn mac_control_exec_hooks() -> Option<&'static MacControlExecHooks> {
    MAC_CONTROL_EXEC_HOOKS.get()
}

// ── Public Re-exports ─────────────────────────────────────────────

// 契约层全量再导出（类型 / 常量 / 谓词 / ToolExecContext / rejection …）：
// crate 外 `ha_core::tools::…` 与 tools/ 内部 `super::…` 既有路径全部保持。
pub use crate::tool_defs::*;

pub(crate) use task::{task_snapshot_data, TASK_REMINDER_INSTRUCTION};

pub use approval::{
    approval_timeout_secs, deny_all_pending, deny_pending_for_session, emit_approval_resolved,
    list_pending_approval_requests, pending_approval_session_id, pending_request_ids_for_session,
    submit_approval_response, ApprovalReasonKind, ApprovalRequest, ApprovalResolutionSource,
    ApprovalResponse, ApprovalSubmitError, EVENT_APPROVAL_RESOLVED,
};
pub use definitions::{
    background_policy_for_tool, get_available_tools, get_core_tools, get_core_tools_for_provider,
    get_deferred_tools, get_tools_for_provider, is_concurrent_safe, is_generic_job_capable,
    is_internal_tool,
};
pub use definitions::{
    get_audio_generate_tool_dynamic, get_image_generate_tool_dynamic, get_subagent_tool,
    get_tool_search_tool, get_workflow_tool,
};
pub use execution::{
    execute_tool_with_context,
    purge_tool_results_for_session,
    // 迁出的 adapter（ha-cron 的 manage_cron）经门面取审批裁决，
    // 不直接 use 私有的 `execution` 模块。
    resolve_tool_permission,
    run_tool_approval,
};

/// Parse a model-facing compact call variant such as `browser__snapshot`.
/// Only explicitly registered composite tools are accepted, so arbitrary
/// tool names containing `__` (notably MCP names) are never rewritten.
pub(crate) fn split_call_variant_name(name: &str) -> Option<(&str, &str)> {
    let (canonical, action) = name.rsplit_once("__")?;
    let supported = matches!(
        canonical,
        TOOL_BROWSER | TOOL_MAC_CONTROL | TOOL_MANAGE_CRON | TOOL_APP_UPDATE
    ) && dispatch::all_dispatchable_tools()
        .iter()
        .find(|definition| definition.name == canonical)
        .is_some_and(|definition| definition.call_variant_actions().contains(&action));
    supported.then_some((canonical, action))
}

pub(crate) fn canonical_tool_schema_name(name: &str) -> &str {
    split_call_variant_name(name)
        .map(|(canonical, _)| canonical)
        .unwrap_or(name)
}

/// Convert a compact model-facing variant back into the canonical call before
/// permission, hooks, audit, persistence, and execution. The fixed action
/// always wins over a model-supplied conflicting value.
pub(crate) fn normalize_call_variant(name: &str, args: &Value) -> Option<(String, Value)> {
    let (canonical, action) = split_call_variant_name(name)?;
    let mut normalized = args.clone();
    let object = normalized.as_object_mut()?;
    object.insert("action".to_string(), Value::String(action.to_string()));
    Some((canonical.to_string(), normalized))
}

/// Extract a string value from a Value that might be a plain string, `{type:"text", text:"..."}`,
/// or an array of such objects (e.g. `[{type:"text", text:"..."}]`).
pub(crate) fn extract_string_param(val: &Value) -> Option<&str> {
    // Plain string
    if let Some(s) = val.as_str() {
        return Some(s);
    }
    // Structured content: {type: "text", text: "..."}
    if let Some(obj) = val.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("text") {
            return obj.get("text").and_then(|v| v.as_str());
        }
    }
    // Array of structured content: [{type: "text", text: "..."}]
    if let Some(arr) = val.as_array() {
        if let Some(first) = arr.first() {
            return extract_string_param(first);
        }
    }
    None
}
