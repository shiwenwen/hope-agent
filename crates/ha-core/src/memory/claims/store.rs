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
use rusqlite::{params, params_from_iter, types::Value as SqlValue, OptionalExtension, Row};

use crate::memory::{MemoryScope, SqliteMemoryBackend};

use super::backfill::BackfillCandidate;
use super::types::{ClaimCandidate, ClaimDetail, ClaimLink, ClaimRecord, EvidenceRecord};
use super::write;

/// Fixed-width UTC RFC3339 (`...SSSZ`) — same format the dreaming store uses,
/// so `valid_until < now` comparisons (here and in the injection JOIN) are
/// lexically monotonic.
fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Outcome of writing one claim candidate. `created` is false when the
/// candidate canonicalized onto an existing claim (evidence merged).
#[derive(Debug, Clone)]
pub struct ClaimWriteOutcome {
    pub claim_id: String,
    pub created: bool,
}

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

/// Build a scope filter from primitive `scope_type` + `scope_id` params
/// (what the Tauri/HTTP shells receive). Strict by design: an unknown
/// `scope_type` is an error rather than a silent "no filter" — that prevents
/// the fail-open where a broken scope silently returns ALL claims. `None`
/// scope_type means "no scope filter" (intended), not a degraded filter.
pub fn parse_claim_scope(
    scope_type: Option<&str>,
    scope_id: Option<&str>,
) -> Result<Option<MemoryScope>> {
    match scope_type.map(str::trim) {
        None | Some("") => Ok(None),
        Some("global") => Ok(Some(MemoryScope::Global)),
        Some("agent") => scope_id
            .filter(|s| !s.is_empty())
            .map(|id| MemoryScope::Agent { id: id.to_string() })
            .map(Some)
            .ok_or_else(|| anyhow!("agent scope requires scopeId")),
        Some("project") => scope_id
            .filter(|s| !s.is_empty())
            .map(|id| MemoryScope::Project { id: id.to_string() })
            .map(Some)
            .ok_or_else(|| anyhow!("project scope requires scopeId")),
        Some(other) => Err(anyhow!("invalid scopeType: {other}")),
    }
}

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

// ── Write path (claim dual-write from memory extraction) ─────────

/// Canonicalize + write one claim candidate (rule-only Light dedup): exact
/// match on `scope + claim_type + subject + predicate + normalized object`
/// merges evidence into the existing active claim; otherwise a new claim +
/// evidence row are created. `default_scope` is the extraction scope used when
/// the candidate carries no scope hint. `session_id` anchors evidence.
pub fn write_claim_candidate(
    candidate: &ClaimCandidate,
    default_scope: &MemoryScope,
    session_id: &str,
    source_run_id: Option<&str>,
) -> Result<ClaimWriteOutcome> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.write_candidate(candidate, default_scope, session_id, source_run_id)
}

/// Link a claim to a legacy `memories` row (the dual-write shadow), consuming
/// the `add_with_dedup` 3-state at the call site. Idempotent (INSERT OR
/// IGNORE on the composite PK).
pub fn link_claim_memory(claim_id: &str, memory_id: i64, sync_mode: &str) -> Result<()> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.link_claim_memory(claim_id, memory_id, sync_mode)
}

/// All `memory_id`s that already have at least one claim link — backfill skips
/// these so a re-run is idempotent and never double-links a memory the live
/// dual-write already claimed.
pub fn all_linked_memory_ids() -> Result<std::collections::HashSet<i64>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.all_linked_memory_ids()
}

/// Write one backfill claim (Phase 2.5): a deterministic claim derived from a
/// legacy `memories` row, with `memory`-sourced evidence and a `detached` link
/// (the claim never owns / hides the pre-existing memory). `proposed_status` is
/// `active` (low-risk) or `needs_review`. Returns `Some(claim_id)` when created,
/// or `None` when skipped (the memory vanished after the scan, or already has a
/// claim link — idempotent under concurrent apply / live dual-write races).
pub fn write_backfill_claim(candidate: &BackfillCandidate) -> Result<Option<String>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.write_backfill_claim(candidate)
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
        // Effective-status filtering (design §4.5): an `active` claim past its
        // `valid_until` reads as `expired` on every read path. The stored
        // `status` column stays `active` (only the injection JOIN and these read
        // APIs derive the effective value), so filtering must mirror that —
        // otherwise `status=active` would leak expired claims and
        // `status=expired` would miss them (Codex finding #3). `now` is reused
        // for the post-query effective mapping below.
        let now = now_rfc3339();
        if let Some(status) = &filter.status {
            match status.as_str() {
                "active" => {
                    conditions.push(
                        "status = 'active' AND (valid_until IS NULL OR valid_until = '' OR valid_until >= ?)"
                            .to_string(),
                    );
                    args.push(SqlValue::Text(now.clone()));
                }
                "expired" => {
                    conditions.push(
                        "(status = 'expired' OR (status = 'active' AND valid_until IS NOT NULL AND valid_until != '' AND valid_until < ?))"
                            .to_string(),
                    );
                    args.push(SqlValue::Text(now.clone()));
                }
                other => {
                    conditions.push("status = ?".to_string());
                    args.push(SqlValue::Text(other.to_string()));
                }
            }
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
        Ok(rows
            .filter_map(|r| r.ok())
            .map(|mut c| {
                c.status = write::effective_status(&c.status, c.valid_until.as_deref(), &now);
                c
            })
            .collect())
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
        let Some(mut claim) = claim else {
            return Ok(None);
        };
        // Mirror the injection path's effective status (design §4.5): an active
        // claim past its valid_until reads as expired here too (Codex #3).
        claim.status =
            write::effective_status(&claim.status, claim.valid_until.as_deref(), &now_rfc3339());

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

    /// Canonicalize + write a candidate. See [`write_claim_candidate`].
    fn write_candidate(
        &self,
        candidate: &ClaimCandidate,
        default_scope: &MemoryScope,
        session_id: &str,
        source_run_id: Option<&str>,
    ) -> Result<ClaimWriteOutcome> {
        // Scope is the trusted extraction scope (same as the dual-write shadow
        // memory), NOT the model's `candidate.scope` hint: letting the model
        // route a claim into an arbitrary agent/project scope would (a) split a
        // claim from its shadow across scopes and (b) risk cross-project
        // routing. Model-hinted scope routing returns once it can be validated
        // against the session's real project/agent.
        let (scope_type, scope_id) = scope_columns(default_scope);
        let normalized = write::normalize_object(&candidate.object);
        let evidence_class = write::normalize_evidence_class(candidate.evidence_class.as_deref());
        let now = now_rfc3339();

        let conn = self.backend.write_conn()?;

        // Rule-only canonicalize: pull the small (scope+subject+predicate)
        // candidate set and match the normalized object in Rust — avoids
        // SQL-side normalization and the idx_memory_claims_spo index keeps the
        // set small.
        let existing = self.find_exact_active_claim(
            &conn,
            &scope_type,
            scope_id.as_deref(),
            &candidate.claim_type,
            &candidate.subject,
            &candidate.predicate,
            &normalized,
        )?;

        if let Some(claim_id) = existing {
            // Merge: bump updated_at + append this round's evidence atomically
            // so a crash can't leave the claim updated but evidence-less.
            with_tx(&conn, || {
                conn.execute(
                    "UPDATE memory_claims SET updated_at = ?1 WHERE id = ?2",
                    params![now, claim_id],
                )?;
                self.insert_evidence_rows(
                    &conn,
                    &claim_id,
                    candidate,
                    evidence_class,
                    session_id,
                    &now,
                )
            })?;
            return Ok(ClaimWriteOutcome {
                claim_id,
                created: false,
            });
        }

        // Create a new claim. Confidence is derived from evidence_class — never
        // taken from the model. `valid_until` is normalized to canonical
        // RFC3339 so the injection filter's lexical compare is sound.
        let claim_id = uuid::Uuid::new_v4().to_string();
        let confidence = write::confidence_baseline(evidence_class);
        let tags_json = serde_json::to_string(&candidate.tags).unwrap_or_else(|_| "[]".to_string());
        let salience = candidate.salience.unwrap_or(0.5).clamp(0.0, 1.0);
        let valid_from = candidate
            .temporal
            .as_ref()
            .and_then(|t| t.valid_from.clone());
        let valid_until = write::normalize_valid_until(
            candidate
                .temporal
                .as_ref()
                .and_then(|t| t.valid_until.as_deref()),
        );

        // Claim row + its evidence in one transaction (an active claim always
        // has at least one evidence anchor — the §11 "every active claim is
        // traceable" invariant).
        with_tx(&conn, || {
            conn.execute(
                "INSERT INTO memory_claims
                    (id, scope_type, scope_id, claim_type, subject, predicate, object,
                     content, tags_json, confidence, confidence_source, salience,
                     freshness_policy_json, status, valid_from, valid_until,
                     supersedes_claim_id, source_run_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'derived', ?11,
                         '{}', 'active', ?12, ?13, NULL, ?14, ?15, ?15)",
                params![
                    claim_id,
                    scope_type,
                    scope_id,
                    candidate.claim_type,
                    candidate.subject,
                    candidate.predicate,
                    candidate.object,
                    candidate.content,
                    tags_json,
                    confidence as f64,
                    salience as f64,
                    valid_from,
                    valid_until,
                    source_run_id,
                    now,
                ],
            )?;
            self.insert_evidence_rows(
                &conn,
                &claim_id,
                candidate,
                evidence_class,
                session_id,
                &now,
            )
        })?;

        Ok(ClaimWriteOutcome {
            claim_id,
            created: true,
        })
    }

    /// Exact-match canonicalize lookup. Returns the id of an active claim with
    /// the same scope/type/subject/predicate and a normalized object equal to
    /// `normalized`, if any.
    #[allow(clippy::too_many_arguments)]
    fn find_exact_active_claim(
        &self,
        conn: &rusqlite::Connection,
        scope_type: &str,
        scope_id: Option<&str>,
        claim_type: &str,
        subject: &str,
        predicate: &str,
        normalized: &str,
    ) -> Result<Option<String>> {
        let mut stmt = conn.prepare(
            "SELECT id, object FROM memory_claims
             WHERE scope_type = ?1
               AND ((?2 IS NULL AND scope_id IS NULL) OR scope_id = ?2)
               AND claim_type = ?3 AND subject = ?4 AND predicate = ?5
               AND status = 'active'",
        )?;
        let rows = stmt.query_map(
            params![scope_type, scope_id, claim_type, subject, predicate],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        for row in rows {
            let (id, object) = row?;
            if write::normalize_object(&object) == normalized {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Insert one evidence row per cited anchor (or a single session-anchored
    /// row when the model cited none). Evidence never comes from incognito
    /// sessions — extraction skips those upstream.
    fn insert_evidence_rows(
        &self,
        conn: &rusqlite::Connection,
        claim_id: &str,
        candidate: &ClaimCandidate,
        evidence_class: &str,
        session_id: &str,
        now: &str,
    ) -> Result<()> {
        let anchors = evidence_anchors(candidate, session_id);
        for (source_type, source_id, ev_session_id, message_id) in anchors {
            conn.execute(
                "INSERT INTO memory_evidence
                    (id, claim_id, source_type, evidence_class, source_id, session_id,
                     message_id, redaction_status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'anchor_only', ?8)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    claim_id,
                    source_type,
                    evidence_class,
                    source_id,
                    ev_session_id,
                    message_id,
                    now,
                ],
            )?;
        }
        Ok(())
    }

    fn link_claim_memory(&self, claim_id: &str, memory_id: i64, sync_mode: &str) -> Result<()> {
        let now = now_rfc3339();
        let conn = self.backend.write_conn()?;
        conn.execute(
            "INSERT OR IGNORE INTO memory_claim_links
                (claim_id, memory_id, sync_mode, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![claim_id, memory_id, sync_mode, now],
        )?;
        Ok(())
    }

    fn all_linked_memory_ids(&self) -> Result<std::collections::HashSet<i64>> {
        let conn = self.backend.read_conn()?;
        let mut stmt = conn.prepare("SELECT DISTINCT memory_id FROM memory_claim_links")?;
        let ids = stmt
            .query_map([], |row| row.get::<_, i64>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    }

    /// Write a deterministic backfill claim + its `memory` evidence + a
    /// `detached` link, all in one transaction. The `detached` link is the
    /// invariant that keeps backfill from changing current prompt injection:
    /// the hidden-set query only hides memories whose `managed` claims are all
    /// dead, so a backfilled claim's status never affects the source memory.
    ///
    /// Idempotent: the "memory still exists AND has no link yet" guard runs
    /// INSIDE the write transaction (on the single writer connection, so
    /// check-then-insert is atomic against concurrent apply / live dual-write).
    /// Returns `None` (skipped) instead of creating a duplicate or an
    /// FK-violating row when the memory was linked or deleted after the scan.
    fn write_backfill_claim(&self, c: &BackfillCandidate) -> Result<Option<String>> {
        let now = now_rfc3339();
        let conn = self.backend.write_conn()?;

        with_tx(&conn, || {
            // Memory deleted between scan and write → skip (don't create a
            // claim whose detached link would FK-fail / orphan).
            let memory_exists = conn
                .query_row(
                    "SELECT 1 FROM memories WHERE id = ?1",
                    params![c.memory_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !memory_exists {
                return Ok(None);
            }
            // Already represented in the claim world (idempotent re-run, or a
            // concurrent apply / live dual-write linked it first) → skip.
            let already_linked = conn
                .query_row(
                    "SELECT 1 FROM memory_claim_links WHERE memory_id = ?1 LIMIT 1",
                    params![c.memory_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if already_linked {
                return Ok(None);
            }

            let claim_id = uuid::Uuid::new_v4().to_string();
            let confidence = write::confidence_baseline(&c.evidence_class);
            let tags_json = serde_json::to_string(&c.tags).unwrap_or_else(|_| "[]".to_string());
            let salience = c.salience.clamp(0.0, 1.0);
            let source_id = format!("memory:{}", c.memory_id);

            conn.execute(
                "INSERT INTO memory_claims
                    (id, scope_type, scope_id, claim_type, subject, predicate, object,
                     content, tags_json, confidence, confidence_source, salience,
                     freshness_policy_json, status, valid_from, valid_until,
                     supersedes_claim_id, source_run_id, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 'derived', ?11,
                         '{}', ?12, NULL, NULL, NULL, NULL, ?13, ?13)",
                params![
                    claim_id,
                    c.scope_type,
                    c.scope_id,
                    c.claim_type,
                    c.subject,
                    c.predicate,
                    c.object,
                    c.content,
                    tags_json,
                    confidence as f64,
                    salience as f64,
                    c.proposed_status,
                    now,
                ],
            )?;
            conn.execute(
                "INSERT INTO memory_evidence
                    (id, claim_id, source_type, evidence_class, source_id,
                     redaction_status, created_at)
                 VALUES (?1, ?2, 'memory', ?3, ?4, 'anchor_only', ?5)",
                params![
                    uuid::Uuid::new_v4().to_string(),
                    claim_id,
                    c.evidence_class,
                    source_id,
                    now,
                ],
            )?;
            conn.execute(
                "INSERT INTO memory_claim_links
                    (claim_id, memory_id, sync_mode, created_at, updated_at)
                 VALUES (?1, ?2, 'detached', ?3, ?3)",
                params![claim_id, c.memory_id, now],
            )?;
            Ok(Some(claim_id))
        })
    }
}

/// Map a scope to its (scope_type, scope_id) columns.
fn scope_columns(scope: &MemoryScope) -> (String, Option<String>) {
    match scope {
        MemoryScope::Global => ("global".to_string(), None),
        MemoryScope::Agent { id } => ("agent".to_string(), Some(id.clone())),
        MemoryScope::Project { id } => ("project".to_string(), Some(id.clone())),
    }
}

/// Run `f` inside a BEGIN/COMMIT transaction on `conn`, rolling back on error.
/// Used to keep a claim and its evidence rows atomic.
fn with_tx<T>(conn: &rusqlite::Connection, f: impl FnOnce() -> Result<T>) -> Result<T> {
    conn.execute_batch("BEGIN")?;
    match f() {
        Ok(v) => {
            conn.execute_batch("COMMIT")?;
            Ok(v)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

/// Build evidence anchors for a candidate. Parses cited `message:<id>` /
/// `memory:<id>` refs; falls back to a single session-message anchor when the
/// model cited none. Returns (source_type, source_id, session_id, message_id).
fn evidence_anchors(
    candidate: &ClaimCandidate,
    session_id: &str,
) -> Vec<(String, String, Option<String>, Option<String>)> {
    let mut anchors: Vec<(String, String, Option<String>, Option<String>)> = Vec::new();
    for r in &candidate.evidence_refs {
        let r = r.trim();
        if let Some(mid) = r.strip_prefix("message:") {
            let mid = mid.trim();
            if !mid.is_empty() {
                anchors.push((
                    "session_message".to_string(),
                    mid.to_string(),
                    Some(session_id.to_string()),
                    Some(mid.to_string()),
                ));
            }
        } else if let Some(memid) = r.strip_prefix("memory:") {
            let memid = memid.trim();
            if !memid.is_empty() {
                anchors.push(("memory".to_string(), memid.to_string(), None, None));
            }
        }
    }
    if anchors.is_empty() {
        anchors.push((
            "session_message".to_string(),
            session_id.to_string(),
            Some(session_id.to_string()),
            None,
        ));
    }
    anchors
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

    #[test]
    fn read_api_returns_effective_status_for_expired_claims() {
        let s = temp_store();
        {
            let conn = s.backend.write_conn().unwrap();
            // An `active` claim whose valid_until is far in the past →
            // effective expired (the stored column stays 'active').
            conn.execute(
                "INSERT INTO memory_claims
                    (id, scope_type, claim_type, subject, predicate, object, content,
                     status, valid_until, created_at, updated_at)
                 VALUES ('exp', 'global', 'preference', 'user', 'prefers', 'x', 'c',
                         'active', '2020-01-01T00:00:00.000Z',
                         '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .unwrap();
            // A genuinely active claim (no expiry) as a control.
            conn.execute(
                "INSERT INTO memory_claims
                    (id, scope_type, claim_type, subject, predicate, object, content,
                     status, created_at, updated_at)
                 VALUES ('live', 'global', 'preference', 'user', 'prefers', 'y', 'c',
                         'active', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .unwrap();
        }

        // Unfiltered list reports the EFFECTIVE status, not the stored 'active'.
        let all = s.list_claims(&ClaimListFilter::default()).unwrap();
        assert_eq!(
            all.iter().find(|c| c.id == "exp").unwrap().status,
            "expired",
            "past valid_until reads as expired"
        );
        assert_eq!(
            all.iter().find(|c| c.id == "live").unwrap().status,
            "active"
        );

        // status=active EXCLUDES the effectively-expired claim.
        let active = s
            .list_claims(&ClaimListFilter {
                status: Some("active".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(active.iter().any(|c| c.id == "live"));
        assert!(
            !active.iter().any(|c| c.id == "exp"),
            "expired-by-date must not show under status=active"
        );

        // status=expired FINDS the active-but-past claim.
        let expired = s
            .list_claims(&ClaimListFilter {
                status: Some("expired".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(
            expired.iter().any(|c| c.id == "exp"),
            "status=expired must find an active claim past valid_until"
        );
        assert!(!expired.iter().any(|c| c.id == "live"));

        // get_claim mirrors the effective status too.
        let detail = s.get_claim("exp").unwrap().unwrap();
        assert_eq!(detail.claim.status, "expired");
    }

    fn backfill_candidate(
        memory_id: i64,
        claim_type: &str,
        status: &str,
    ) -> crate::memory::claims::BackfillCandidate {
        crate::memory::claims::BackfillCandidate {
            memory_id,
            scope_type: "global".into(),
            scope_id: None,
            claim_type: claim_type.into(),
            subject: "user".into(),
            predicate: "prefers".into(),
            object: "terse replies".into(),
            content: "Prefers terse replies".into(),
            tags: vec!["t".into()],
            evidence_class: "explicit_user_statement".into(),
            confidence: 0.85,
            salience: 0.9,
            pinned: true,
            proposed_status: status.into(),
        }
    }

    #[test]
    fn write_backfill_claim_creates_claim_memory_evidence_and_detached_link() {
        let s = temp_store();
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO memories (id, memory_type, scope_type, content, source, created_at, updated_at)
                 VALUES (77, 'feedback', 'global', 'Prefers terse replies', 'user', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            ).unwrap();
        }

        let claim_id = s
            .write_backfill_claim(&backfill_candidate(77, "preference", "active"))
            .unwrap()
            .expect("created");

        let detail = s.get_claim(&claim_id).unwrap().unwrap();
        assert_eq!(detail.claim.status, "active");
        assert_eq!(detail.claim.claim_type, "preference");
        // Confidence is derived from evidence_class, not the candidate field.
        assert!((detail.claim.confidence - 0.85).abs() < 1e-6);
        assert_eq!(detail.claim.confidence_source, "derived");
        // Evidence is memory-sourced, anchored to memory:<id>.
        assert_eq!(detail.evidence.len(), 1);
        assert_eq!(detail.evidence[0].source_type, "memory");
        assert_eq!(detail.evidence[0].source_id, "memory:77");
        // Link is detached → backfill never hides the source memory.
        assert_eq!(detail.links.len(), 1);
        assert_eq!(detail.links[0].memory_id, 77);
        assert_eq!(detail.links[0].sync_mode, "detached");
        // The backfilled memory now counts as linked (idempotent skip on re-run).
        assert!(s.all_linked_memory_ids().unwrap().contains(&77));

        // Re-running on the SAME memory must skip (no duplicate claim) — the
        // in-tx "already linked" guard, the concurrency-safety fix.
        assert!(
            s.write_backfill_claim(&backfill_candidate(77, "preference", "active"))
                .unwrap()
                .is_none(),
            "second backfill of a linked memory must skip"
        );
        let all = s.list_claims(&ClaimListFilter::default()).unwrap();
        assert_eq!(
            all.iter().filter(|c| c.subject == "user").count(),
            1,
            "no duplicate claim for an already-linked memory"
        );
    }

    #[test]
    fn write_backfill_claim_skips_missing_memory() {
        // The memory was deleted after the scan → skip, not an FK error / orphan.
        let s = temp_store();
        let out = s
            .write_backfill_claim(&backfill_candidate(999, "preference", "active"))
            .unwrap();
        assert!(out.is_none(), "missing memory must be skipped");
        assert!(s
            .list_claims(&ClaimListFilter::default())
            .unwrap()
            .is_empty());
    }

    #[test]
    fn write_backfill_claim_needs_review_lands_in_review_queue() {
        let s = temp_store();
        {
            // The link FK to memories(id) is enforced, so the source memory must
            // exist — real backfill always scans it out of `memories` first.
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO memories (id, memory_type, scope_type, content, source, created_at, updated_at)
                 VALUES (88, 'user', 'global', 'Lives in Berlin', 'user', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            ).unwrap();
        }
        let id = s
            .write_backfill_claim(&backfill_candidate(88, "user_profile", "needs_review"))
            .unwrap()
            .expect("created");
        let detail = s.get_claim(&id).unwrap().unwrap();
        assert_eq!(detail.claim.status, "needs_review");
        // Findable via the review-queue filter.
        let queue = s
            .list_claims(&ClaimListFilter {
                status: Some("needs_review".into()),
                ..Default::default()
            })
            .unwrap();
        assert!(queue.iter().any(|c| c.id == id));
    }

    fn candidate(object: &str, evidence_class: Option<&str>) -> ClaimCandidate {
        ClaimCandidate {
            claim_type: "preference".into(),
            subject: "user".into(),
            predicate: "prefers".into(),
            object: object.into(),
            content: format!("user prefers {object}"),
            scope: None,
            evidence_class: evidence_class.map(|s| s.to_string()),
            salience: Some(0.7),
            temporal: None,
            evidence_refs: Vec::new(),
            tags: vec!["t".into()],
        }
    }

    #[test]
    fn write_candidate_creates_then_merges_on_exact_match() {
        let s = temp_store();
        let scope = MemoryScope::Global;

        let first = s
            .write_candidate(
                &candidate("Bun", Some("explicit_user_statement")),
                &scope,
                "sess-1",
                None,
            )
            .unwrap();
        assert!(first.created);

        // Same scope+type+subject+predicate, object differs only by case /
        // whitespace → canonicalize merges (no new claim), evidence appended.
        let second = s
            .write_candidate(
                &candidate("  bun ", Some("user_confirmed")),
                &scope,
                "sess-2",
                None,
            )
            .unwrap();
        assert!(!second.created, "exact-match candidate must merge");
        assert_eq!(second.claim_id, first.claim_id);

        let all = s.list_claims(&ClaimListFilter::default()).unwrap();
        assert_eq!(all.len(), 1, "merge must not create a duplicate claim");

        let detail = s.get_claim(&first.claim_id).unwrap().unwrap();
        assert_eq!(detail.evidence.len(), 2, "both rounds appended evidence");
        // Confidence derived from the CREATE class baseline, not the model.
        assert!((detail.claim.confidence - 0.85).abs() < 1e-6);
        assert_eq!(detail.claim.confidence_source, "derived");
    }

    #[test]
    fn write_candidate_distinct_object_creates_separate_claim() {
        let s = temp_store();
        let scope = MemoryScope::Global;
        s.write_candidate(&candidate("Bun", None), &scope, "sess-1", None)
            .unwrap();
        s.write_candidate(&candidate("pnpm", None), &scope, "sess-1", None)
            .unwrap();
        assert_eq!(s.list_claims(&ClaimListFilter::default()).unwrap().len(), 2);
    }

    #[test]
    fn write_candidate_uses_extraction_scope_not_model_hint() {
        let s = temp_store();
        let mut c = candidate("Bun", None);
        // A model hint pointing at a DIFFERENT scope must be ignored — the
        // claim follows the trusted extraction scope (here: a project),
        // keeping claim + shadow consistent and blocking cross-scope routing.
        c.scope = Some(super::super::types::ClaimScopeHint {
            scope_type: "global".into(),
            id: None,
        });
        let out = s
            .write_candidate(
                &c,
                &MemoryScope::Project {
                    id: "proj-1".into(),
                },
                "sess-1",
                None,
            )
            .unwrap();
        let detail = s.get_claim(&out.claim_id).unwrap().unwrap();
        assert_eq!(detail.claim.scope_type, "project");
        assert_eq!(detail.claim.scope_id.as_deref(), Some("proj-1"));
    }

    #[test]
    fn unknown_evidence_class_defaults_confidence() {
        let s = temp_store();
        let out = s
            .write_candidate(
                &candidate("Bun", Some("totally-bogus")),
                &MemoryScope::Global,
                "sess-1",
                None,
            )
            .unwrap();
        let detail = s.get_claim(&out.claim_id).unwrap().unwrap();
        assert!((detail.claim.confidence - 0.45).abs() < 1e-6);
        assert_eq!(detail.evidence[0].evidence_class, "assistant_inferred");
    }

    #[test]
    fn parse_claim_scope_is_strict() {
        assert!(matches!(super::parse_claim_scope(None, None), Ok(None)));
        assert!(matches!(
            super::parse_claim_scope(Some("global"), None),
            Ok(Some(MemoryScope::Global))
        ));
        assert!(matches!(
            super::parse_claim_scope(Some("agent"), Some("ha-main")),
            Ok(Some(MemoryScope::Agent { .. }))
        ));
        // agent/project without id → error (not silent None).
        assert!(super::parse_claim_scope(Some("agent"), None).is_err());
        // Unknown scope type → error (no fail-open to "all").
        assert!(super::parse_claim_scope(Some("[object Object]"), None).is_err());
        assert!(super::parse_claim_scope(Some("bogus"), None).is_err());
    }

    #[test]
    fn link_claim_memory_is_idempotent() {
        let s = temp_store();
        let out = s
            .write_candidate(
                &candidate("Bun", None),
                &MemoryScope::Global,
                "sess-1",
                None,
            )
            .unwrap();
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO memories (id, memory_type, scope_type, content, source, created_at, updated_at)
                 VALUES (7, 'user', 'global', 'x', 'auto', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            )
            .unwrap();
        }
        s.link_claim_memory(&out.claim_id, 7, "managed").unwrap();
        s.link_claim_memory(&out.claim_id, 7, "managed").unwrap(); // idempotent
        let detail = s.get_claim(&out.claim_id).unwrap().unwrap();
        assert_eq!(detail.links.len(), 1);
        assert_eq!(detail.links[0].memory_id, 7);
    }
}
