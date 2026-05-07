mod acp_db;
mod db;
mod helpers;
mod pending;
mod subagent_db;
mod tasks;
mod types;

pub(crate) use db::strip_fts_snippet_sentinels;
pub use db::{
    LastAssistantTokens, ProjectFilter, SessionDB, SessionSearchResult, SessionTypeFilter,
};
pub use helpers::{
    auto_title, cleanup_orphan_incognito, db_path, effective_session_working_dir,
    ensure_first_message_title, is_session_incognito, lookup_session_meta,
};
pub use pending::enrich_pending_interactions;
pub use tasks::{
    delete_task_and_snapshot, emit_task_snapshot, set_task_status_and_snapshot, Task, TaskStatus,
};
pub use types::{
    build_chat_user_attachments_meta, MessageRole, NewMessage, SessionMessage, SessionMeta,
};
