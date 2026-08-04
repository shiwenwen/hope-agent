//! Knowledge Base subsystem ("Knowledge Space", see `docs/architecture/knowledge-base.md`).
//!
//! Zero Tauri dependency (red line). Two storage classes (D9):
//! - **Registry** ([`KnowledgeRegistry`]) — `knowledge_bases` + access bindings in
//!   `sessions.db` (truth source).
//! - **Index cache** (`ha_knowledge::knowledge::IndexDb`) — note/chunk/link/tag
//!   + FTS5 + vec0 in `~/.hope-agent/knowledge/index.db` (rebuildable from the
//!   `.md` files).
//!
//! Note files (`.md`) are the single truth source for content; the index is a
//! cache. Internal KBs are app-managed + writable; external (bound) KBs are
//! browse-only in Phase 1 (D11).
//!
//! # 阶段 5 第六刀：kernel 只留台账、契约与裁决
//!
//! 索引缓存 / 解析编译 / 检索 / embedding / Layer-2 维护 / `note_*` 工具全部
//! 上浮 `ha-knowledge`；本模块留下的三样各有恒留 kernel 的理由：
//!
//! - [`registry`] —— 81 处直接 `session_db.conn.lock()`，`SessionDB` 的写连接
//!   按红线不对特征 crate 开放。
//! - [`types`] / [`maintenance_defs`] —— registry 方法签名用到的 wire 类型。
//! - [`access`] —— [`effective_kb_access`] 是「访问默认 deny」的唯一裁决点，
//!   `agent` / `tool_defs` / `chat_engine` / `subagent` / `channel` 均直接引用
//!   [`KbAccess`] / [`KbAccessSource`] / [`ChannelKbContext`]。契约留下，这些
//!   引用一条都不必改成钩子。
//!
//! 上浮部分对 kernel 的反向调用走 [`crate::knowledge_hooks`]。

pub mod access;
pub mod maintenance_defs;
pub mod registry;
pub(crate) use registry::workspace_root;
pub mod types;

pub use access::{
    effective_kb_access, im_kb_access_allowed, ChannelKbContext, KbAccessSource,
    KnowledgeAccessContext,
};
pub use registry::{resolve_kb_dir, KbRoot, KnowledgeRegistry};
pub use types::*;
