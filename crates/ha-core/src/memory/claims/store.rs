//! Read store for the structured claim layer (next-gen Dreaming, PR: schema
//! + read API).
//!
//! Reuses the memory backend's connection pool (never opens a second
//! connection to `memory.db`), mirroring the dreaming store. This PR is
//! read-only — claim writes / dual-write / canonicalize land later; the
//! `OnceLock` handle and free functions are the stable entry the Tauri / HTTP
//! shells call.

use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, Result};
use rusqlite::{params_from_iter, types::Value as SqlValue, OptionalExtension, Row};

use crate::memory::{MemoryScope, SqliteMemoryBackend};

use super::types::{ClaimDetail, ClaimLink, ClaimRecord, EvidenceRecord};

/// Process-wide store handle, initialised once at startup from the concrete
/// `SqliteMemoryBackend` (see [`init_claim_store`]). `None` in contexts that
/// never opened the memory backend (some tests, minimal ACP).
static CLAIM_STORE: OnceLock<ClaimStore> = OnceLock::new();

/// Default / max page sizes for `list_claims`, matching the dreaming run list.
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;

/// Filter for [`list_claims`]. All fields optional; `None` means "any".
#[derive(Debug, Clone, Default)]
pub struct ClaimListFilter {
    pub scope: Option<MemoryScope>,
    /// active | superseded | expired | archived | needs_review.
    pub status: Option<String>,
    pub claim_type: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

pub struct ClaimStore {
    backend: Arc<SqliteMemoryBackend>,
}

/// Initialise the global claim store. Called once during app init with the
/// same concrete backend that backs `MEMORY_BACKEND`. Idempotent.
pub fn init_claim_store(backend: Arc<SqliteMemoryBackend>) {
    let _ = CLAIM_STORE.set(ClaimStore::new(backend));
}

fn store() -> Option<&'static ClaimStore> {
    CLAIM_STORE.get()
}

// ── Public command API (Tauri / HTTP layers call these) ─────────

/// List claims, newest-updated first, with optional scope / status / type
/// filters. `limit` is clamped to `[1, 500]`.
pub fn list_claims(filter: ClaimListFilter) -> Result<Vec<ClaimRecord>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.list_claims(&filter)
}

/// Fetch a single claim plus its evidence and legacy-memory links. Returns
/// `None` if the id is unknown.
pub fn get_claim(id: &str) -> Result<Option<ClaimDetail>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.get_claim(id)
}

impl ClaimStore {
    fn new(backend: Arc<SqliteMemoryBackend>) -> Self {
        Self { backend }
    }

    fn list_claims(&self, filter: &ClaimListFilter) -> Result<Vec<ClaimRecord>> {
        let mut conditions: Vec<String> = Vec::new();
        let mut args: Vec<SqlValue> = Vec::new();

        match &filter.scope {
            Some(MemoryScope::Global) => conditions.push("scope_type = 'global'".to_string()),
            Some(MemoryScope::Agent { id }) => {
                conditions.push("scope_type = 'agent' AND scope_id = ?".to_string());
                args.push(SqlValue::Text(id.clone()));
            }
            Some(MemoryScope::Project { id }) => {
                conditions.push("scope_type = 'project' AND scope_id = ?".to_string());
                args.push(SqlValue::Text(id.clone()));
            }
            None => {}
        }
        if let Some(status) = &filter.status {
            conditions.push("status = ?".to_string());
            args.push(SqlValue::Text(status.clone()));
        }
        if let Some(claim_type) = &filter.claim_type {
            conditions.push("claim_type = ?".to_string());
            args.push(SqlValue::Text(claim_type.clone()));
        }

        let where_clause = if conditions.is_empty() {
            "1=1".to_string()
        } else {
            conditions.join(" AND ")
        };
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .clamp(1, MAX_LIST_LIMIT);
        let offset = filter.offset.unwrap_or(0);

        let sql = format!(
            "SELECT id, scope_type, scope_id, claim_type, subject, predicate, object,
                    content, tags_json, confidence, confidence_source, salience,
                    freshness_policy_json, status, valid_from, valid_until,
                    supersedes_claim_id, source_run_id, created_at, updated_at
             FROM memory_claims
             WHERE {where_clause}
             ORDER BY updated_at DESC
             LIMIT ? OFFSET ?"
        );
        args.push(SqlValue::Integer(limit as i64));
        args.push(SqlValue::Integer(offset as i64));

        let conn = self.backend.read_conn()?;
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(args), row_to_claim)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    fn get_claim(&self, id: &str) -> Result<Option<ClaimDetail>> {
        let conn = self.backend.read_conn()?;
        let claim = conn
            .query_row(
                "SELECT id, scope_type, scope_id, claim_type, subject, predicate, object,
                        content, tags_json, confidence, confidence_source, salience,
                        freshness_policy_json, status, valid_from, valid_until,
                        supersedes_claim_id, source_run_id, created_at, updated_at
                 FROM memory_claims WHERE id = ?1",
                params_from_iter([SqlValue::Text(id.to_string())]),
                row_to_claim,
            )
            .optional()?;
        let Some(claim) = claim else {
            return Ok(None);
        };

        let mut ev_stmt = conn.prepare(
            "SELECT id, claim_id, source_type, evidence_class, source_id, session_id,
                    message_id, file_path, url, quote, redaction_status,
                    access_scope_json, weight, created_at
             FROM memory_evidence WHERE claim_id = ?1
             ORDER BY weight DESC, created_at ASC",
        )?;
        let evidence = ev_stmt
            .query_map(
                params_from_iter([SqlValue::Text(id.to_string())]),
                row_to_evidence,
            )?
            .filter_map(|r| r.ok())
            .collect();

        let mut link_stmt = conn.prepare(
            "SELECT claim_id, memory_id, sync_mode, last_synced_claim_status,
                    created_at, updated_at
             FROM memory_claim_links WHERE claim_id = ?1
             ORDER BY created_at ASC",
        )?;
        let links = link_stmt
            .query_map(
                params_from_iter([SqlValue::Text(id.to_string())]),
                row_to_link,
            )?
            .filter_map(|r| r.ok())
            .collect();

        Ok(Some(ClaimDetail {
            claim,
            evidence,
            links,
        }))
    }
}

/// Parse a `'[]'`-style JSON array column into a `Vec<String>`, tolerating
/// malformed values.
fn parse_tags(raw: &str) -> Vec<String> {
    serde_json::from_str(raw).unwrap_or_default()
}

/// Parse a JSON-object column into a value, defaulting to `{}` on error.
fn parse_json_object(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({}))
}

fn row_to_claim(row: &Row) -> rusqlite::Result<ClaimRecord> {
    let tags_json: String = row.get(8)?;
    let freshness_json: String = row.get(12)?;
    Ok(ClaimRecord {
        id: row.get(0)?,
        scope_type: row.get(1)?,
        scope_id: row.get(2)?,
        claim_type: row.get(3)?,
        subject: row.get(4)?,
        predicate: row.get(5)?,
        object: row.get(6)?,
        content: row.get(7)?,
        tags: parse_tags(&tags_json),
        confidence: row.get::<_, f64>(9)? as f32,
        confidence_source: row.get(10)?,
        salience: row.get::<_, f64>(11)? as f32,
        freshness_policy: parse_json_object(&freshness_json),
        status: row.get(13)?,
        valid_from: row.get(14)?,
        valid_until: row.get(15)?,
        supersedes_claim_id: row.get(16)?,
        source_run_id: row.get(17)?,
        created_at: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

fn row_to_evidence(row: &Row) -> rusqlite::Result<EvidenceRecord> {
    let access_json: String = row.get(11)?;
    Ok(EvidenceRecord {
        id: row.get(0)?,
        claim_id: row.get(1)?,
        source_type: row.get(2)?,
        evidence_class: row.get(3)?,
        source_id: row.get(4)?,
        session_id: row.get(5)?,
        message_id: row.get(6)?,
        file_path: row.get(7)?,
        url: row.get(8)?,
        quote: row.get(9)?,
        redaction_status: row.get(10)?,
        access_scope: parse_json_object(&access_json),
        weight: row.get::<_, f64>(12)? as f32,
        created_at: row.get(13)?,
    })
}

fn row_to_link(row: &Row) -> rusqlite::Result<ClaimLink> {
    Ok(ClaimLink {
        claim_id: row.get(0)?,
        memory_id: row.get(1)?,
        sync_mode: row.get(2)?,
        last_synced_claim_status: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;

    /// A claim store over a fresh temp `memory.db` (the `open` path creates the
    /// claim tables alongside `memories` + the dreaming tables).
    fn temp_store() -> ClaimStore {
        let dir = std::env::temp_dir().join(format!("ha-claims-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let backend = Arc::new(SqliteMemoryBackend::open(&dir.join("memory.db")).unwrap());
        ClaimStore::new(backend)
    }

    fn insert_claim(
        store: &ClaimStore,
        id: &str,
        scope_type: &str,
        scope_id: Option<&str>,
        status: &str,
    ) {
        let conn = store.backend.write_conn().unwrap();
        conn.execute(
            "INSERT INTO memory_claims
                (id, scope_type, scope_id, claim_type, subject, predicate, object,
                 content, status, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'preference', 'user', 'prefers', 'x', 'c', ?4,
                     '2026-01-01T00:00:00.000Z', ?5)",
            params![
                id,
                scope_type,
                scope_id,
                status,
                format!("2026-01-0{}T00:00:00.000Z", (id.len() % 9) + 1)
            ],
        )
        .unwrap();
    }

    #[test]
    fn list_empty_when_no_claims() {
        let s = temp_store();
        let out = s.list_claims(&ClaimListFilter::default()).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn list_filters_by_scope_status_and_type() {
        let s = temp_store();
        insert_claim(&s, "c1", "global", None, "active");
        insert_claim(&s, "c2", "agent", Some("ha-main"), "active");
        insert_claim(&s, "c3", "agent", Some("ha-main"), "archived");

        let global = s
            .list_claims(&ClaimListFilter {
                scope: Some(MemoryScope::Global),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].id, "c1");

        let agent_active = s
            .list_claims(&ClaimListFilter {
                scope: Some(MemoryScope::Agent {
                    id: "ha-main".into(),
                }),
                status: Some("active".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(agent_active.len(), 1);
        assert_eq!(agent_active[0].id, "c2");
    }

    #[test]
    fn get_claim_returns_detail_with_evidence_and_links() {
        let s = temp_store();
        insert_claim(&s, "c1", "global", None, "active");
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO memory_evidence
                    (id, claim_id, source_type, source_id, created_at)
                 VALUES ('e1', 'c1', 'session_message', 'sess:1', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .unwrap();
            // A legacy memory row to satisfy the link (FK declared but not
            // enforced here; insert a real memory for realism).
            conn.execute(
                "INSERT INTO memories (id, memory_type, scope_type, content, source, created_at, updated_at)
                 VALUES (42, 'user', 'global', 'hello', 'auto', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO memory_claim_links
                    (claim_id, memory_id, created_at, updated_at)
                 VALUES ('c1', 42, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .unwrap();
        }

        let detail = s.get_claim("c1").unwrap().expect("claim exists");
        assert_eq!(detail.claim.id, "c1");
        assert_eq!(detail.claim.tags.len(), 0);
        assert_eq!(detail.evidence.len(), 1);
        assert_eq!(detail.evidence[0].source_type, "session_message");
        assert_eq!(detail.links.len(), 1);
        assert_eq!(detail.links[0].memory_id, 42);
        // Defaults from schema hydrate correctly.
        assert_eq!(detail.claim.status, "active");
        assert_eq!(detail.evidence[0].redaction_status, "redacted");
        assert_eq!(detail.links[0].sync_mode, "managed");
    }

    #[test]
    fn get_claim_unknown_id_returns_none() {
        let s = temp_store();
        assert!(s.get_claim("nope").unwrap().is_none());
    }

    #[test]
    fn list_clamps_limit() {
        let s = temp_store();
        let out = s
            .list_claims(&ClaimListFilter {
                limit: Some(99999),
                ..Default::default()
            })
            .unwrap();
        // No rows, but the query must not error with an over-large limit.
        assert!(out.is_empty());
    }
}
