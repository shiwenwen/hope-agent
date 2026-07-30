mod acp_db;
pub mod design_hooks;
mod design_threads;
pub use design_threads::DesignChatThread;
pub mod privacy;
// ACP run 行类型（表随 kernel、类型随表）：供 ha-acp 特征 crate 原路径再导出
pub use acp_db::{AcpRun, AcpRunStatus};
mod artifacts;
pub(crate) mod cleanup_watcher;
pub(crate) mod db;
mod environment;
pub(crate) mod events;
pub mod export;
mod helpers;
pub(crate) use helpers::workspace_root;
mod ide_context;
mod pending;
mod stream_persistence;
mod subagent_db;
mod tasks;
mod turn_queue;
mod turns;
mod types;

pub use artifacts::{aggregate_session_artifacts, FileArtifact, SessionArtifacts, UrlSource};
pub(crate) use db::strip_fts_snippet_sentinels;
pub use db::{
    sanitize_fts_query, LastAssistantTokens, ParentSessionFilter, ProjectFilter, SessionDB,
    SessionSearchResult, SessionTypeFilter,
};
pub use environment::build_git_snapshot; // pub：ha-vcs git_control 消费
pub(crate) use environment::load_git_diff_for_root;
pub use environment::{
    load_session_environment, load_session_git_diff, WorkspaceEnvironmentSnapshot,
    WorkspaceGitCommit, WorkspaceGitDiff, WorkspaceGitFileAction, WorkspaceGitFileChange,
    WorkspaceGitSnapshot, WorkspaceGitStatus, WorkspaceGitSync, WorkspaceGitSyncState,
    WorkspaceWorkingDirSnapshot, WorkspaceWorkingDirSource,
};
pub use helpers::{
    auto_title, cleanup_orphan_incognito, db_path, effective_session_working_dir,
    effective_working_dir_for_meta, ensure_first_message_title, ensure_session_runtime_defaults,
    first_message_title_candidate, is_session_incognito, lookup_session_meta,
    resolve_chat_runtime_defaults, set_session_model_preference,
    set_session_reasoning_effort_preference, set_session_temperature_preference,
    ChatRuntimeDefaults,
};
pub use ide_context::{
    IdeDiagnosticContext, IdeLineRange, IdeSymbolContext, SessionIdeContext,
    SessionIdeContextSnapshot,
};
pub use pending::enrich_pending_interactions;
pub use stream_persistence::{
    journal_events_have_assistant_output, select_recoverable_attempt_prefix,
    stream_attempt_context_checkpoint, trailing_text_from_journal_events, verify_block,
    ChatStreamAttempt, ChatStreamJournalBlock, ChatStreamRun, CommitAssistantTurn,
    CommitInterruptedTurn, CommittedTurn, CreateStreamRun, JournalBatch, JournalEvent,
    StreamRunRegistration, StreamRunSnapshot,
};
pub use tasks::{
    create_task_and_snapshot, delete_task_and_snapshot, emit_task_snapshot,
    set_task_status_and_snapshot, Task, TaskStatus,
};
pub use turn_queue::{
    EnqueueQueuedTurnMessageOutcome, NewQueuedTurnMessage, QueuedTurnMessageMode,
    QueuedTurnMessageRecord, QueuedTurnMessageStatus, QueuedTurnMessageView,
    EVENT_TURN_QUEUE_CHANGED, MAX_QUEUED_TURN_MESSAGES_PER_SESSION,
};
pub use turns::{ChatTurn, ChatTurnInterruptReason, ChatTurnStatus};
pub use types::{
    build_chat_user_attachments_meta, build_tool_media_items_attachments_meta, ChannelSessionInfo,
    ForkSessionResult, MessageRole, NewMessage, PendingCountdown, SessionDefaultsInput,
    SessionKind, SessionMemoryPolicy, SessionMemoryPolicyValue, SessionMessage, SessionMeta,
    UnreadSessionTarget, ATTACHMENT_META_KEY_ACTIVE_MEMORY, ATTACHMENT_META_KEY_QUEUED_MESSAGE,
    ATTACHMENT_META_KEY_RETRIEVAL_PLANNER, ATTACHMENT_META_KEY_TOOL_MEDIA_ITEMS,
    ATTACHMENT_META_KEY_USED_MEMORY_REFS,
};
