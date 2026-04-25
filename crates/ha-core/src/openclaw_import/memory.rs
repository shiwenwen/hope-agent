//! Parse OpenClaw `MEMORY.md` files into `NewMemory` rows for Hope Agent's
//! SQLite memory backend. v1 only handles markdown; the SQLite vector store
//! at `~/.openclaw/memory/{agentId}.sqlite` is left for v2 (schema + vector
//! dimensions are not directly compatible).

use std::path::{Path, PathBuf};

use crate::memory::types::{MemoryScope, MemoryType, NewMemory};

use super::paths;

/// Parse an OpenClaw-style MEMORY.md content into Hope Agent NewMemory rows.
///
/// Rules (deliberately simple — OpenClaw's MEMORY.md has no fixed schema):
///   - Every `- ` / `* ` bullet → one memory entry
///   - Each non-empty paragraph (group of consecutive non-blank lines) that
///     contains no bullets → one memory entry
///   - Skip headings (lines starting with `#`)
///   - All entries get `memory_type = User`, `source = "import"`, the caller-
///     provided scope
pub fn parse_openclaw_memory_md(content: &str, scope: MemoryScope) -> Vec<NewMemory> {
    let mut out: Vec<NewMemory> = Vec::new();
    let mut paragraph: Vec<String> = Vec::new();

    for raw_line in content.lines() {
        let line = raw_line.trim_end();
        let trimmed = line.trim();

        if trimmed.is_empty() {
            flush_paragraph(&mut paragraph, &scope, &mut out);
            continue;
        }
        if trimmed.starts_with('#') {
            flush_paragraph(&mut paragraph, &scope, &mut out);
            continue;
        }

        let bullet = strip_bullet(trimmed);
        if let Some(bullet_text) = bullet {
            // Bullets break paragraphs — flush whatever was collecting first.
            flush_paragraph(&mut paragraph, &scope, &mut out);
            if !bullet_text.is_empty() {
                out.push(make_entry(bullet_text.to_string(), scope.clone()));
            }
        } else {
            paragraph.push(trimmed.to_string());
        }
    }
    flush_paragraph(&mut paragraph, &scope, &mut out);
    out
}

fn strip_bullet(line: &str) -> Option<&str> {
    if let Some(rest) = line.strip_prefix("- ") {
        return Some(rest.trim());
    }
    if let Some(rest) = line.strip_prefix("* ") {
        return Some(rest.trim());
    }
    if line == "-" || line == "*" {
        return Some("");
    }
    None
}

fn flush_paragraph(buf: &mut Vec<String>, scope: &MemoryScope, out: &mut Vec<NewMemory>) {
    if buf.is_empty() {
        return;
    }
    let joined = buf.join(" ").trim().to_string();
    buf.clear();
    if !joined.is_empty() {
        out.push(make_entry(joined, scope.clone()));
    }
}

fn make_entry(content: String, scope: MemoryScope) -> NewMemory {
    NewMemory {
        memory_type: MemoryType::User,
        scope,
        content,
        tags: Vec::new(),
        source: "import".to_string(),
        source_session_id: None,
        pinned: false,
        attachment_path: None,
        attachment_mime: None,
    }
}

// ── Discovery ──────────────────────────────────────────────────

/// Path to the global `MEMORY.md` (root level).
pub fn global_memory_path(state_dir: &Path) -> PathBuf {
    state_dir.join("MEMORY.md")
}

/// Path to `MEMORY.md` for a given agent. OpenClaw stores it under
/// `agents/{id}/agent/MEMORY.md`. We also probe the lowercase variant which
/// some users may have written by hand.
pub fn agent_memory_path(state_dir: &Path, agent_id: &str) -> Option<PathBuf> {
    let dir = paths::agent_dir(state_dir, agent_id);
    for name in ["MEMORY.md", "memory.md"] {
        let p = dir.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Count how many entries `MEMORY.md` would produce, used by the scan preview
/// without instantiating full NewMemory rows.
pub fn estimate_entries(content: &str) -> usize {
    parse_openclaw_memory_md(content, MemoryScope::Global).len()
}

// TODO(v2): Import OpenClaw's SQLite vector memory store at
// `~/.openclaw/memory/{agentId}.sqlite`. Blockers: (a) embedding model and
// dimensions may differ from Hope Agent's configured embedder, requiring
// re-embedding; (b) the vec_memories table schema is OpenClaw-specific and
// would need to be translated to NewMemory rows here.
