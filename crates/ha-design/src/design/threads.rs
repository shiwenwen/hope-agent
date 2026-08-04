//! Design-space per-project chat threads.
//!
//! A "thread" is a `kind='design'` session bound to the design project it
//! iterates on. Mirrors the knowledge-space chat threads (`knowledge/registry.rs`):
//! the anchor rows live in **sessions.db** (`design_chat_threads`, created in
//! `session/db.rs`) so the history picker can JOIN `sessions` / `messages`; the
//! `project_id` is a plain column because the design project row lives in the
//! separate `design.db` (no cross-db FK). Threads are hidden from the main
//! sidebar / `/sessions` / global FTS via `SessionKind::Design`.
//!
//! This is NOT a security boundary — like knowledge threads it only scopes the
//! conversation container.

use anyhow::Result;

// 类型与查询已下沉 kernel（表在 sessions.db、类型随表），原路径再导出。
pub use ha_core::session::DesignChatThread;

fn session_db() -> Result<&'static std::sync::Arc<ha_core::session::SessionDB>> {
    ha_core::globals::get_session_db().ok_or_else(|| anyhow::anyhow!("SessionDB not initialized"))
}

/// Record a `kind='design'` session as a chat thread anchored to a project.
/// Idempotent on `session_id`.
pub fn create_thread(session_id: &str, project_id: &str) -> Result<()> {
    session_db()?.create_design_thread(session_id, project_id)?;
    ha_core::pet::emit_activity_changed();
    Ok(())
}

/// The design project a chat-thread session is anchored to, if any. Used by the
/// `design` tool to resolve which project a `kind='design'` chat turn edits.
pub fn project_for_session(session_id: &str) -> Result<Option<String>> {
    session_db()?.design_thread_project(session_id)
}

/// Most-recently-active chat thread session for a project (default-load target).
pub fn latest_thread_for_project(project_id: &str) -> Result<Option<String>> {
    session_db()?.latest_design_thread(project_id)
}

/// A page of chat threads in a project, newest-active first — see
/// `SessionDB::list_design_threads`.
pub fn list_threads(
    project_id: &str,
    query: Option<&str>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DesignChatThread>> {
    session_db()?.list_design_threads(project_id, query, limit, offset)
}

/// Session ids of every design chat thread bound to `project_id`. Used by the
/// design-project delete cascade.
pub fn thread_session_ids(project_id: &str) -> Result<Vec<String>> {
    session_db()?.design_thread_session_ids(project_id)
}
