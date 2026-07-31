//! Learning 事件**发布面**——kernel 侧的事件通道。
//!
//! # 为什么在 kernel 而不在 dashboard（crate-split 破环）
//!
//! 生产者遍布四层：kernel（`tools::memory` 的 recall 埋点）、未来的
//! ha-skills（skill CRUD）、ha-knowledge（维护调度）、以及**已经独立的
//! 特征 crate ha-mcp**（工具调用成败）。发布面若留在 dashboard，
//! ha-dash 一旦成 crate，这些生产者就要全部反向依赖它——ha-mcp 那条尤其
//! 荒谬：一个已拆出的 crate 为了打点去依赖另一个特征 crate。
//!
//! 发布与消费之间本就**没有代码耦合**，只共享表名与 `EVT_*` 常量：
//! DDL / INSERT / prune / 会话级联删除全在 [`crate::session::SessionDB`]，
//! dashboard 侧只有 4 个只读聚合查询。所以事件通道下沉 kernel、dashboard
//! 退化为纯订阅方，是恢复单向依赖的最小改动。
//!
//! **新增事件种类由生产者侧声明**：常量放这里（跨生产者共享的）或生产者
//! 自己的模块（单点使用的，如 `skills::auto_review` 的
//! `EVT_SKILL_REVIEW_SKIPPED`），dashboard 不需要预先认识它们——聚合查询
//! 按 kind 字符串过滤，未知 kind 只是不出现在现有卡片里。

// ── Event kinds (stable strings stored in learning_events.kind) ──
pub const EVT_SKILL_CREATED: &str = "skill_created";
pub const EVT_SKILL_PATCHED: &str = "skill_patched";
pub const EVT_SKILL_ACTIVATED: &str = "skill_activated";
pub const EVT_SKILL_DISCARDED: &str = "skill_discarded";
pub const EVT_SKILL_USED: &str = "skill_used";
pub const EVT_RECALL_HIT: &str = "recall_hit";
pub const EVT_RECALL_SUMMARY_USED: &str = "recall_summary_used";
/// An MCP tool call returned a result (success path). `ref_id` is the
/// namespaced name `mcp__<server>__<tool>`; `meta` carries `server`,
/// `tool`, `durationMs`.
pub const EVT_MCP_TOOL_CALLED: &str = "mcp_tool_called";
/// An MCP tool call produced an error (protocol / timeout / tool-side
/// `isError=true`). `meta` carries the same fields plus `error`.
pub const EVT_MCP_TOOL_FAILED: &str = "mcp_tool_failed";

/// Best-effort emitter. Silently no-ops if the session DB hasn't been
/// initialized (e.g. in unit tests for subsystems that don't need one).
///
/// Dispatches the INSERT onto `spawn_blocking` when we're inside a Tokio
/// runtime so the caller (often a hot path like `tool_recall_memory` or a
/// skill CRUD op) doesn't wait on the SessionDB writer Mutex. Falls back to
/// a sync call outside an async context (e.g. from a blocking worker).
pub fn emit(
    kind: &str,
    session_id: Option<&str>,
    ref_id: Option<&str>,
    meta: Option<&serde_json::Value>,
) {
    let Some(db) = crate::get_session_db().cloned() else {
        return;
    };
    let kind = kind.to_string();
    let session_id = session_id.map(str::to_string);
    let ref_id = ref_id.map(str::to_string);
    let meta = meta.cloned();
    let write = move || {
        db.record_learning_event(
            &kind,
            session_id.as_deref(),
            ref_id.as_deref(),
            meta.as_ref(),
        );
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn_blocking(write);
        }
        Err(_) => write(),
    }
}
