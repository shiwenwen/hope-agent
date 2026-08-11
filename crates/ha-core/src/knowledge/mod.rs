//! Knowledge Base subsystem ("Knowledge Space", see `docs/architecture/core/knowledge-base.md`).
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
//! 上浮 `ha-knowledge`；本模块留下的契约各有恒留 kernel 的理由：
//!
//! - [`registry`] —— 81 处直接 `session_db.conn.lock()`，`SessionDB` 的写连接
//!   按红线不对特征 crate 开放。
//! - [`types`] / [`maintenance_defs`] —— registry 方法签名用到的 wire 类型。
//! - [`access`] —— [`effective_kb_access`] 是「访问默认 deny」的唯一裁决点，
//!   `agent` / `tool_defs` / `chat_engine` / `subagent` / `channel` 均直接引用
//!   [`KbAccess`] / [`KbAccessSource`] / [`ChannelKbContext`]。契约留下，这些
//!   引用一条都不必改成钩子。
//! - [`wikilink_target`] / [`legacy_wikilink_targets`] —— typed note wire、
//!   legacy turn detector 与 `ha-knowledge` injector 共用的语法/target 语义，
//!   避免 `#anchor` / `|alias` 在解析链和队列路由上漂移。
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

/// The note-resolution target inside a `[[wikilink]]`: aliases and anchors
/// affect presentation/selection, but not which note the reference binds.
pub fn wikilink_target(inner: &str) -> &str {
    let before_alias = inner
        .split_once('|')
        .map(|(target, _)| target)
        .unwrap_or(inner);
    before_alias
        .split_once('#')
        .map(|(target, _)| target)
        .unwrap_or(before_alias)
        .trim()
}

/// Extract the unique note targets consumed by the retained legacy
/// `[[wikilink]]` turn injector. This deliberately mirrors that injector's
/// current raw-text semantics: a single-line non-empty body ending in `]]` is
/// recognized even inside Markdown code. Ordinary `[label](url)` links do not
/// match. Keep the scanner linear and dependency-free so kernel queue routing
/// and the optional `ha-knowledge` feature share one syntax boundary.
pub fn legacy_wikilink_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for_each_legacy_wikilink(text, |inner| {
        let target = wikilink_target(inner);
        if !target.is_empty() && !targets.iter().any(|existing| existing == target) {
            targets.push(target.to_string());
        }
        false
    });
    targets
}

/// Whether a full chat turn may resolve at least one retained legacy note
/// reference. Queue insertion uses this to avoid dropping Knowledge injection
/// semantics at a raw mid-turn user-message boundary.
pub fn contains_legacy_wikilink(text: &str) -> bool {
    for_each_legacy_wikilink(text, |inner| !wikilink_target(inner).is_empty())
}

fn for_each_legacy_wikilink(text: &str, mut stop: impl FnMut(&str) -> bool) -> bool {
    let bytes = text.as_bytes();
    let mut cursor = 0usize;
    let mut content_start = None;
    while cursor < bytes.len() {
        match content_start {
            None if bytes[cursor..].starts_with(b"[[") => {
                content_start = Some(cursor + 2);
                cursor += 2;
            }
            None => cursor += 1,
            Some(_) if bytes[cursor] == b'\n' => {
                content_start = None;
                cursor += 1;
            }
            Some(start) if bytes[cursor] == b']' => {
                let closes = bytes[cursor..].starts_with(b"]]");
                content_start = None;
                if closes && cursor > start && stop(&text[start..cursor]) {
                    return true;
                }
                cursor += if closes { 2 } else { 1 };
            }
            Some(_) => cursor += 1,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::{contains_legacy_wikilink, legacy_wikilink_targets};

    #[test]
    fn legacy_wikilink_detector_matches_injector_syntax() {
        let text = "see [[Roadmap#Next|plan]] and [[Roadmap]]";
        assert!(contains_legacy_wikilink(text));
        assert_eq!(legacy_wikilink_targets(text), vec!["Roadmap"]);
        // The retained turn injector scans raw text today, including code.
        assert!(contains_legacy_wikilink("`[[Roadmap]]`"));
        assert!(contains_legacy_wikilink("```md\n[[Roadmap]]\n```"));
    }

    #[test]
    fn legacy_wikilink_detector_rejects_unconsumable_or_markdown_links() {
        assert!(!contains_legacy_wikilink("[Roadmap](notes/Roadmap.md)"));
        assert!(!contains_legacy_wikilink("[[]] [[  ]] [[broken\nlink]]"));
        assert!(legacy_wikilink_targets("ordinary text").is_empty());
    }
}
