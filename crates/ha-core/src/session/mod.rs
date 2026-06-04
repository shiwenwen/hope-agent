mod acp_db;
mod artifacts;
mod db;
mod environment;
pub mod export;
mod helpers;
mod pending;
mod subagent_db;
mod tasks;
mod turns;
mod types;

pub use artifacts::{aggregate_session_artifacts, FileArtifact, SessionArtifacts, UrlSource};
pub(crate) use db::strip_fts_snippet_sentinels;
pub use db::{
    LastAssistantTokens, ProjectFilter, SessionDB, SessionSearchResult, SessionTypeFilter,
};
pub use environment::{
    load_session_environment, WorkspaceEnvironmentSnapshot, WorkspaceGitCommit,
    WorkspaceGitSnapshot, WorkspaceGitStatus, WorkspaceGitSync, WorkspaceGitSyncState,
    WorkspaceWorkingDirSnapshot, WorkspaceWorkingDirSource,
};
pub use helpers::{
    auto_title, cleanup_orphan_incognito, db_path, effective_session_working_dir,
    effective_working_dir_for_meta, ensure_first_message_title, is_session_incognito,
    lookup_session_meta,
};
pub use pending::enrich_pending_interactions;
pub use tasks::{
    delete_task_and_snapshot, emit_task_snapshot, set_task_status_and_snapshot, Task, TaskStatus,
};
pub use turns::{ChatTurn, ChatTurnInterruptReason, ChatTurnStatus};
pub use types::{
    build_chat_user_attachments_meta, build_tool_media_items_attachments_meta, MessageRole,
    NewMessage, SessionMessage, SessionMeta, ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS,
};
