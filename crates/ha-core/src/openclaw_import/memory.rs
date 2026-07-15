//! OpenClaw memory import helpers.
//!
//! OpenClaw `MEMORY.md` maps to Hope Agent's canonical Core `MEMORY.md` files. OpenClaw's
//! SQLite vector store at `~/.openclaw/memory/{agentId}.sqlite` maps to Hope
//! Agent's memory database by importing chunk text only; embeddings are not
//! reused because model/dimension/signature contracts differ.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};

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

/// Path to OpenClaw's SQLite vector memory for a given agent.
pub fn agent_sqlite_memory_path(state_dir: &Path, agent_id: &str) -> Option<PathBuf> {
    let path = state_dir
        .join("memory")
        .join(format!("{}.sqlite", agent_id));
    path.exists().then_some(path)
}

/// Count importable SQLite chunk rows without reading content.
pub fn sqlite_memory_entry_count(path: &Path) -> Result<usize> {
    let conn = open_readonly(path)?;
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM chunks WHERE trim(text) != ''",
            [],
            |row| row.get(0),
        )
        .with_context(|| format!("Failed to count SQLite memory chunks in {}", path.display()))?;
    Ok(count.max(0) as usize)
}

/// Import OpenClaw SQLite chunk text as Hope Agent memory rows.
pub fn parse_openclaw_sqlite_memory_db(path: &Path, scope: MemoryScope) -> Result<Vec<NewMemory>> {
    let conn = open_readonly(path)?;
    let mut stmt = conn
        .prepare(
            "SELECT text FROM chunks \
             WHERE trim(text) != '' \
             ORDER BY updated_at ASC, id ASC",
        )
        .with_context(|| {
            format!(
                "Failed to prepare SQLite memory query for {}",
                path.display()
            )
        })?;

    let mut rows = stmt.query([]).with_context(|| {
        format!(
            "Failed to read SQLite memory chunks from {}",
            path.display()
        )
    })?;
    let mut out = Vec::new();
    while let Some(row) = rows.next()? {
        let text: String = row.get(0)?;
        let content = text.trim();
        if !content.is_empty() {
            out.push(NewMemory {
                memory_type: MemoryType::User,
                scope: scope.clone(),
                content: content.to_string(),
                tags: Vec::new(),
                source: "openclaw-db-import".to_string(),
                source_session_id: None,
                pinned: false,
                attachment_path: None,
                attachment_mime: None,
            });
        }
    }
    Ok(out)
}

fn open_readonly(path: &Path) -> Result<Connection> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("Failed to open OpenClaw SQLite memory {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sqlite_memory_chunks_import_text_only() {
        let temp = tempdir().expect("tempdir");
        let path = temp.path().join("main.sqlite");
        let conn = Connection::open(&path).expect("open sqlite");
        conn.execute_batch(
            r#"
            CREATE TABLE chunks (
              id TEXT PRIMARY KEY,
              path TEXT NOT NULL,
              source TEXT NOT NULL DEFAULT 'memory',
              start_line INTEGER NOT NULL,
              end_line INTEGER NOT NULL,
              hash TEXT NOT NULL,
              model TEXT NOT NULL,
              text TEXT NOT NULL,
              embedding TEXT NOT NULL,
              updated_at INTEGER NOT NULL
            );
            INSERT INTO chunks
              (id, path, start_line, end_line, hash, model, text, embedding, updated_at)
            VALUES
              ('a', 'memory', 1, 1, 'h1', 'm', ' first memory ', '[]', 1),
              ('b', 'memory', 2, 2, 'h2', 'm', '', '[]', 2),
              ('c', 'memory', 3, 3, 'h3', 'm', 'second memory', '[]', 3);
            "#,
        )
        .expect("seed sqlite");
        drop(conn);

        assert_eq!(sqlite_memory_entry_count(&path).expect("count"), 2);
        let entries = parse_openclaw_sqlite_memory_db(
            &path,
            MemoryScope::Agent {
                id: "main".to_string(),
            },
        )
        .expect("parse sqlite");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].content, "first memory");
        assert_eq!(entries[0].source, "openclaw-db-import");
        assert_eq!(entries[1].content, "second memory");
    }
}
