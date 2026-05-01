mod acp_db;
mod db;
mod helpers;
mod pending;
mod subagent_db;
mod tasks;
mod types;

pub use db::{ProjectFilter, SessionDB, SessionSearchResult, SessionTypeFilter};
pub use helpers::{
    auto_title, cleanup_orphan_incognito, db_path, effective_session_working_dir,
    ensure_first_message_title, is_session_incognito, lookup_session_meta,
};
pub use pending::enrich_pending_interactions;
pub use tasks::{Task, TaskStatus};
pub use types::{ChannelSessionInfo, MessageRole, NewMessage, SessionMessage, SessionMeta};
