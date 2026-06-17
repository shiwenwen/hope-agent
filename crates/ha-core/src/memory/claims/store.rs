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
use super::types::{
    ClaimCandidate, ClaimDetail, ClaimLink, ClaimRecord, EvidenceRecord, ResolveClaim,
};
use super::write;

use crate::util::now_rfc3339;

/// Shared `WHERE` fragment for claim relevance search: active + not-expired +
/// scope (design §4.8). Conditions use the `c.` alias so both the FTS JOIN and
/// the vec0 `rowid IN (...)` subquery can splice them in verbatim. The returned
/// args bind in order — `now` first, then the optional scope id.
fn claim_search_filters(scope: Option<&MemoryScope>, now: &str) -> (String, Vec<SqlValue>) {
    let mut conditions = vec![
        "c.status = 'active'".to_string(),
        "(c.valid_until IS NULL OR c.valid_until = '' OR c.valid_until >= ?)".to_string(),
    ];
    let mut args = vec![SqlValue::Text(now.to_string())];
    match scope {
        Some(MemoryScope::Global) => conditions.push("c.scope_type = 'global'".to_string()),
        Some(MemoryScope::Agent { id }) => {
            conditions.push("c.scope_type = 'agent' AND c.scope_id = ?".to_string());
            args.push(SqlValue::Text(id.clone()));
        }
        Some(MemoryScope::Project { id }) => {
            conditions.push("c.scope_type = 'project' AND c.scope_id = ?".to_string());
            args.push(SqlValue::Text(id.clone()));
        }
        None => {}
    }
    (conditions.join(" AND "), args)
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

/// FTS5 relevance search over active claims for the Context Pack "Relevant
/// Claims" section. Scope-filtered, effective-active only, ranked by FTS.
pub fn search_claims(
    query: &str,
    scope: Option<MemoryScope>,
    limit: usize,
) -> Result<Vec<ClaimRecord>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.search_claims(query, scope.as_ref(), limit)
}

/// "Pinned" claims for the Context Pack: high-salience active claims that inject
/// regardless of the current query (design §4.5 — high confidence + high
/// salience → prompt candidate). Scope-filtered, effective-active only, ranked
/// salience DESC then confidence DESC. `min_salience` is the pin threshold;
/// `limit` clamped to `[1, 500]`.
pub fn list_pinned_claims(
    scope: Option<MemoryScope>,
    min_salience: f32,
    limit: usize,
) -> Result<Vec<ClaimRecord>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.list_pinned_claims(scope.as_ref(), min_salience, limit)
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

// ── Deep resolver primitives (expire / merge / conflict) ─────────

/// Load every `active` claim for the Deep resolver to group + reason over.
pub fn list_active_claims_for_resolve() -> Result<Vec<ResolveClaim>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.list_active_claims_for_resolve()
}

/// Flip an `active` claim to `expired` (its `valid_until` has passed). No-op if
/// it isn't currently `active` (another path already moved it). Returns whether
/// a row changed.
pub fn expire_claim(claim_id: &str) -> Result<bool> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.set_claim_status(claim_id, "expired", None)
}

/// Flip an `active` claim to `needs_review` (conflict the resolver won't
/// auto-resolve). Returns whether a row changed.
pub fn mark_claim_needs_review(claim_id: &str) -> Result<bool> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.set_claim_status(claim_id, "needs_review", None)
}

/// Merge `drop_id` into `keep_id`: re-point the dropped claim's evidence onto
/// the kept claim, then archive the dropped one (status `archived`). Atomic.
/// Returns whether the drop was archived (false if it wasn't active anymore).
pub fn merge_claims(keep_id: &str, drop_id: &str) -> Result<bool> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.merge_claims(keep_id, drop_id)
}

// ── User-correction primitives (Phase 6 / design §5.2 §5.3) ─────
//
// These power the Lucid Review loop: edit / approve / reject / move-scope /
// pin / forget. Unlike the resolver's `set_claim_status` (active-gated), a
// user action operates on the stored row in ANY status (e.g. approving a
// `needs_review` claim back to `active`). Orchestration — diff → decision
// audit → events — lives in [`super::review`]; these are the storage layer.

/// Snapshot of a claim's user-editable fields, read before a correction so the
/// audit diff and the re-embed decision can be computed. Returns the RAW
/// stored `status` (read APIs derive the effective one; a user action mutates
/// the stored row).
#[derive(Debug, Clone)]
pub struct ClaimEditState {
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub content: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub tags: Vec<String>,
    pub status: String,
    pub salience: f32,
    pub confidence: f32,
    pub confidence_source: String,
}

/// A resolved set of field changes for [`apply_claim_fields`]. Every field is
/// optional — `None` means "leave unchanged". `scope` sets both columns
/// together so a move to global clears `scope_id`.
#[derive(Debug, Clone, Default)]
pub struct ClaimFieldUpdate {
    pub content: Option<String>,
    pub subject: Option<String>,
    pub predicate: Option<String>,
    pub object: Option<String>,
    pub tags: Option<Vec<String>>,
    /// active | needs_review | expired | archived (caller validates).
    pub status: Option<String>,
    /// `(scope_type, scope_id)` — `scope_id=None` for global.
    pub scope: Option<(String, Option<String>)>,
    pub salience: Option<f32>,
    pub confidence: Option<f32>,
    pub confidence_source: Option<String>,
}

/// Load a claim's mutable fields for a user-correction diff. `None` if the
/// claim id is unknown.
pub fn claim_edit_state(claim_id: &str) -> Result<Option<ClaimEditState>> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.claim_edit_state(claim_id)
}

/// Apply a resolved set of user field changes in one UPDATE (any → any status).
/// Always bumps `updated_at`. Returns whether a row changed.
pub fn apply_claim_fields(claim_id: &str, upd: &ClaimFieldUpdate) -> Result<bool> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.apply_claim_fields(claim_id, upd)
}

/// Re-embed a claim's current content into vec0 (call after a content edit so
/// Active Memory v2 / Context Pack recall reflects the new text). No-op when
/// the row is gone or embeddings are disabled. MUST run without the write lock
/// held — `embed_and_index_claim` re-acquires it for the embedding cache.
pub fn reembed_claim(claim_id: &str) -> Result<()> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.reembed_claim(claim_id)
}

/// Append one highest-priority `manual_correction` evidence row for a user
/// action (design §5.3 — user corrections are authoritative provenance).
/// `quote` is user-authored, so `raw_allowed` (no redaction).
pub fn add_correction_evidence(
    claim_id: &str,
    scope_type: &str,
    scope_id: Option<&str>,
    evidence_class: &str,
    quote: &str,
) -> Result<()> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.add_correction_evidence(claim_id, scope_type, scope_id, evidence_class, quote)
}

/// Forget a claim. `permanent=false` (default) archives it (kept as an audit
/// trail) and stops its linked legacy memories from re-injecting; `true`
/// hard-deletes the claim graph (claim + evidence + links + vector) plus any
/// legacy memory that becomes orphaned. Returns whether the claim existed.
pub fn forget_claim(claim_id: &str, permanent: bool) -> Result<bool> {
    let store = store().ok_or_else(|| anyhow!("claim store not initialised"))?;
    store.forget_claim(claim_id, permanent)
}

/// Hard-delete every claim in `scope` (with its evidence / links / vec0 rows)
/// plus that scope's profile snapshots. Used by the project-deletion cascade:
/// the claim layer lives in `memory.db` with `PRAGMA foreign_keys` off, so the
/// graph is torn down explicitly (mirrors the permanent-forget teardown).
/// Returns the number of claims removed. No-op (Ok(0)) if the store is absent.
pub fn delete_claims_for_scope(scope: &MemoryScope) -> Result<usize> {
    let Some(store) = store() else {
        return Ok(0);
    };
    store.delete_claims_for_scope(scope)
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

    fn list_pinned_claims(
        &self,
        scope: Option<&MemoryScope>,
        min_salience: f32,
        limit: usize,
    ) -> Result<Vec<ClaimRecord>> {
        let now = now_rfc3339();
        let mut conditions: Vec<String> = vec![
            "status = 'active'".to_string(),
            "(valid_until IS NULL OR valid_until = '' OR valid_until >= ?)".to_string(),
            "salience >= ?".to_string(),
        ];
        let mut args: Vec<SqlValue> = vec![
            SqlValue::Text(now.clone()),
            SqlValue::Real(min_salience as f64),
        ];
        match scope {
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
        let where_clause = conditions.join(" AND ");
        let limit = limit.clamp(1, MAX_LIST_LIMIT);
        let sql = format!(
            "SELECT id, scope_type, scope_id, claim_type, subject, predicate, object,
                    content, tags_json, confidence, confidence_source, salience,
                    freshness_policy_json, status, valid_from, valid_until,
                    supersedes_claim_id, source_run_id, created_at, updated_at
             FROM memory_claims
             WHERE {where_clause}
             ORDER BY salience DESC, confidence DESC, updated_at DESC
             LIMIT ?"
        );
        args.push(SqlValue::Integer(limit as i64));
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

    /// FTS5 relevance search over active claims (Context Pack "Relevant
    /// Claims"). Matches content/subject/object, keeps only effective-active
    /// claims (status='active' AND not past valid_until) in the given scope,
    /// ordered by FTS rank. Empty query / no FTS tokens → empty result.
    fn search_claims(
        &self,
        query: &str,
        scope: Option<&MemoryScope>,
        limit: usize,
    ) -> Result<Vec<ClaimRecord>> {
        let Some(fts_query) = crate::memory::helpers::expand_query(query) else {
            return Ok(Vec::new());
        };
        let now = now_rfc3339();
        let limit = limit.clamp(1, MAX_LIST_LIMIT);
        // Over-fetch each arm so RRF has room to re-rank before the final cut.
        let cand_limit = (limit * 3) as i64;
        let (filter_sql, filter_args) = claim_search_filters(scope, &now);

        let conn = self.backend.read_conn()?;

        // ── Arm 1: FTS5 keyword candidates (rowids in rank order) ──
        let mut fts_rowids: Vec<i64> = Vec::new();
        {
            let sql = format!(
                "SELECT fts.rowid FROM memory_claims_fts fts
                 JOIN memory_claims c ON c.rowid = fts.rowid
                 WHERE memory_claims_fts MATCH ? AND {filter_sql}
                 ORDER BY fts.rank LIMIT ?"
            );
            let mut args: Vec<SqlValue> = vec![SqlValue::Text(fts_query)];
            args.extend(filter_args.iter().cloned());
            args.push(SqlValue::Integer(cand_limit));
            if let Ok(mut stmt) = conn.prepare(&sql) {
                if let Ok(rows) = stmt.query_map(params_from_iter(args), |r| r.get::<_, i64>(0)) {
                    fts_rowids.extend(rows.filter_map(|r| r.ok()));
                }
            }
        }

        // ── Arm 2: vector candidates (rowids in distance order), only when an
        // embedder is configured. Mirrors the `memories` hybrid path: vec0 KNN
        // with a `rowid IN (...)` filter for signature + scope/freshness. ──
        let mut vec_rowids: Vec<i64> = Vec::new();
        if let Some(signature) = crate::memory::helpers::active_embedding_signature() {
            if let Some(emb) = self.backend.generate_embedding(query) {
                let emb_bytes: Vec<u8> = emb.iter().flat_map(|f| f.to_le_bytes()).collect();
                let sql = format!(
                    "SELECT rowid FROM memory_claims_vec
                     WHERE embedding MATCH ?
                       AND rowid IN (
                           SELECT c.rowid FROM memory_claims c
                           WHERE c.embedding_signature = ? AND {filter_sql}
                       )
                     ORDER BY distance LIMIT ?"
                );
                let mut args: Vec<SqlValue> =
                    vec![SqlValue::Blob(emb_bytes), SqlValue::Text(signature)];
                args.extend(filter_args.iter().cloned());
                args.push(SqlValue::Integer(cand_limit));
                if let Ok(mut stmt) = conn.prepare(&sql) {
                    if let Ok(rows) = stmt.query_map(params_from_iter(args), |r| r.get::<_, i64>(0))
                    {
                        vec_rowids.extend(rows.filter_map(|r| r.ok()));
                    }
                }
            }
        }

        if fts_rowids.is_empty() && vec_rowids.is_empty() {
            return Ok(Vec::new());
        }

        // ── Weighted RRF fusion (same weights as `memories` hybrid search) ──
        let hybrid = crate::memory::helpers::load_hybrid_search_config();
        let k = hybrid.rrf_k;
        let mut scores: std::collections::HashMap<i64, f64> = std::collections::HashMap::new();
        for (rank, rowid) in fts_rowids.iter().enumerate() {
            *scores.entry(*rowid).or_insert(0.0) +=
                hybrid.text_weight as f64 / (k + rank as f64 + 1.0);
        }
        for (rank, rowid) in vec_rowids.iter().enumerate() {
            *scores.entry(*rowid).or_insert(0.0) +=
                hybrid.vector_weight as f64 / (k + rank as f64 + 1.0);
        }
        let mut ranked: Vec<(i64, f64)> = scores.into_iter().collect();
        // Score desc, then rowid asc for a deterministic tie-break.
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });
        ranked.truncate(limit);
        let top: Vec<i64> = ranked.iter().map(|(rowid, _)| *rowid).collect();

        // ── Fetch the winning rows, restore fused order ──
        let placeholders = top.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT c.id, c.scope_type, c.scope_id, c.claim_type, c.subject, c.predicate, c.object,
                    c.content, c.tags_json, c.confidence, c.confidence_source, c.salience,
                    c.freshness_policy_json, c.status, c.valid_from, c.valid_until,
                    c.supersedes_claim_id, c.source_run_id, c.created_at, c.updated_at, c.rowid
             FROM memory_claims c WHERE c.rowid IN ({placeholders})"
        );
        let args: Vec<SqlValue> = top.iter().map(|id| SqlValue::Integer(*id)).collect();
        let mut stmt = conn.prepare(&sql)?;
        let mut by_rowid: std::collections::HashMap<i64, ClaimRecord> =
            std::collections::HashMap::new();
        let rows = stmt.query_map(params_from_iter(args), |row| {
            let claim = row_to_claim(row)?;
            let rowid: i64 = row.get(20)?;
            Ok((rowid, claim))
        })?;
        for r in rows.flatten() {
            by_rowid.insert(r.0, r.1);
        }
        Ok(top
            .iter()
            .filter_map(|rowid| {
                by_rowid.remove(rowid).map(|mut c| {
                    c.status = write::effective_status(&c.status, c.valid_until.as_deref(), &now);
                    c
                })
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
            &now,
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

        // Release the write lock before embedding: `embed_and_index_claim` calls
        // `generate_embedding`, which re-acquires `write_conn` for the embedding
        // cache (the writer Mutex is not re-entrant). The claim is already
        // durably committed above; the vector is a best-effort follow-up that
        // the reembed job backfills if the embedder is offline right now.
        drop(conn);
        self.backend
            .embed_and_index_claim(&claim_id, &candidate.content);

        Ok(ClaimWriteOutcome {
            claim_id,
            created: true,
        })
    }

    /// Exact-match canonicalize lookup. Returns the id of an active claim with
    /// the same scope/type/subject/predicate and a normalized object equal to
    /// `normalized`, if any.
    #[allow(clippy::too_many_arguments)]
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
        now: &str,
    ) -> Result<Option<String>> {
        // Dedup must target only *effectively-active* claims (design §4.5): a
        // claim stored `active` but past its `valid_until` reads as `expired`
        // everywhere, so merging a fresh fact into it would bury the new
        // evidence behind a non-injectable status until the (manual-only) Deep
        // resolver reconciles it. Mirror the read-path filter so an expired
        // shadow no longer swallows a re-stated fact — the candidate falls
        // through and gets a live claim instead.
        let mut stmt = conn.prepare(
            "SELECT id, object FROM memory_claims
             WHERE scope_type = ?1
               AND ((?2 IS NULL AND scope_id IS NULL) OR scope_id = ?2)
               AND claim_type = ?3 AND subject = ?4 AND predicate = ?5
               AND status = 'active'
               AND (valid_until IS NULL OR valid_until = '' OR valid_until >= ?6)",
        )?;
        let rows = stmt.query_map(
            params![scope_type, scope_id, claim_type, subject, predicate, now],
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

    /// See the module-level [`delete_claims_for_scope`]. FK cascade is off on
    /// this DB, so the graph is torn down explicitly (claim + evidence + link +
    /// vec0) plus the scope's profile snapshots.
    fn delete_claims_for_scope(&self, scope: &MemoryScope) -> Result<usize> {
        let (scope_type, scope_id) = scope_columns(scope);
        let conn = self.backend.write_conn()?;

        // Snapshot the target (id, rowid) pairs up front, then tear them down in
        // one transaction. Scoped block drops the prepared statement before the
        // write transaction begins.
        let targets: Vec<(String, i64)> = {
            let mut stmt = conn.prepare(
                "SELECT id, rowid FROM memory_claims
                 WHERE scope_type = ?1
                   AND ((?2 IS NULL AND scope_id IS NULL) OR scope_id = ?2)",
            )?;
            let rows: Vec<(String, i64)> = stmt
                .query_map(params![scope_type, scope_id], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
                })?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        let removed = targets.len();
        with_tx(&conn, || {
            for (claim_id, rowid) in &targets {
                // vec0 has no delete trigger (and is created lazily, so tolerate
                // it being absent); FTS is trigger-maintained on the DELETE.
                let _ = conn.execute(
                    "DELETE FROM memory_claims_vec WHERE rowid = ?1",
                    params![rowid],
                );
                conn.execute(
                    "DELETE FROM memory_evidence WHERE claim_id = ?1",
                    params![claim_id],
                )?;
                conn.execute(
                    "DELETE FROM memory_claim_links WHERE claim_id = ?1",
                    params![claim_id],
                )?;
                conn.execute("DELETE FROM memory_claims WHERE id = ?1", params![claim_id])?;
            }
            // Profile snapshots are scope-keyed, not claim-keyed, and store '' for
            // the global scope_id. A project scope always carries its id, so this
            // matches the rows `insert_profile_snapshot` wrote for the scope.
            conn.execute(
                "DELETE FROM memory_profile_snapshots WHERE scope_type = ?1 AND scope_id = ?2",
                params![scope_type, scope_id.as_deref().unwrap_or("")],
            )?;
            Ok(())
        })?;
        Ok(removed)
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
            // Idempotent per anchor: re-extracting the same fact from the same
            // source (canonicalize merge re-runs this every round) must not
            // append a duplicate evidence row. Dedup on the anchor identity
            // (claim_id + source_type + source_id + session + message), treating
            // NULL as '' so a fully-NULL anchor still collapses. NOT a UNIQUE
            // index (no schema migration; correction evidence goes through a
            // different path and is unaffected).
            conn.execute(
                "INSERT INTO memory_evidence
                    (id, claim_id, source_type, evidence_class, source_id, session_id,
                     message_id, redaction_status, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'anchor_only', ?8
                 WHERE NOT EXISTS (
                     SELECT 1 FROM memory_evidence
                     WHERE claim_id = ?2
                       AND source_type = ?3
                       AND IFNULL(source_id, '') = IFNULL(?5, '')
                       AND IFNULL(session_id, '') = IFNULL(?6, '')
                       AND IFNULL(message_id, '') = IFNULL(?7, ''))",
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

    fn list_active_claims_for_resolve(&self) -> Result<Vec<ResolveClaim>> {
        let conn = self.backend.read_conn()?;
        let mut stmt = conn.prepare(
            "SELECT id, scope_type, scope_id, claim_type, subject, predicate, object,
                    content, confidence, confidence_source, salience, valid_until,
                    created_at, updated_at
             FROM memory_claims
             WHERE status = 'active'
             ORDER BY scope_type, scope_id, claim_type, subject, predicate, created_at",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ResolveClaim {
                id: row.get(0)?,
                scope_type: row.get(1)?,
                scope_id: row.get(2)?,
                claim_type: row.get(3)?,
                subject: row.get(4)?,
                predicate: row.get(5)?,
                object: row.get(6)?,
                content: row.get(7)?,
                confidence: row.get::<_, f64>(8)? as f32,
                confidence_source: row.get(9)?,
                salience: row.get::<_, f64>(10)? as f32,
                valid_until: row.get(11)?,
                created_at: row.get(12)?,
                updated_at: row.get(13)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Set a claim's status, guarded on it still being `active` (so two resolver
    /// passes / a user edit can't double-apply). Returns whether a row changed.
    fn set_claim_status(
        &self,
        claim_id: &str,
        status: &str,
        supersedes: Option<&str>,
    ) -> Result<bool> {
        let now = now_rfc3339();
        let conn = self.backend.write_conn()?;
        let changed = conn.execute(
            "UPDATE memory_claims
             SET status = ?2, supersedes_claim_id = COALESCE(?3, supersedes_claim_id),
                 updated_at = ?4
             WHERE id = ?1 AND status = 'active'",
            params![claim_id, status, supersedes, now],
        )?;
        Ok(changed > 0)
    }

    fn merge_claims(&self, keep_id: &str, drop_id: &str) -> Result<bool> {
        if keep_id == drop_id {
            return Ok(false);
        }
        let now = now_rfc3339();
        let conn = self.backend.write_conn()?;
        with_tx(&conn, || {
            // The survivor must still be active — we're folding evidence onto it.
            let keep_active = conn
                .query_row(
                    "SELECT 1 FROM memory_claims WHERE id = ?1 AND status = 'active'",
                    params![keep_id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();
            if !keep_active {
                return Ok(false);
            }
            // Archive the dropped claim FIRST, guarded on it still being active.
            // If it raced to a terminal state between the resolve scan and now,
            // bail BEFORE touching evidence so a stale decision can't silently
            // re-point provenance onto the survivor without an audit row.
            let archived = conn.execute(
                "UPDATE memory_claims SET status = 'archived', updated_at = ?2
                 WHERE id = ?1 AND status = 'active'",
                params![drop_id, now],
            )?;
            if archived == 0 {
                return Ok(false);
            }
            // Drop archived → safe to fold its evidence onto the survivor.
            conn.execute(
                "UPDATE memory_evidence SET claim_id = ?1 WHERE claim_id = ?2",
                params![keep_id, drop_id],
            )?;
            Ok(true)
        })
    }

    fn claim_edit_state(&self, claim_id: &str) -> Result<Option<ClaimEditState>> {
        let conn = self.backend.read_conn()?;
        let row = conn
            .query_row(
                "SELECT scope_type, scope_id, content, subject, predicate,
                        object, tags_json, status, salience, confidence, confidence_source
                 FROM memory_claims WHERE id = ?1",
                params![claim_id],
                |r| {
                    let tags_json: String = r.get(6)?;
                    Ok(ClaimEditState {
                        scope_type: r.get(0)?,
                        scope_id: r.get(1)?,
                        content: r.get(2)?,
                        subject: r.get(3)?,
                        predicate: r.get(4)?,
                        object: r.get(5)?,
                        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
                        status: r.get(7)?,
                        salience: r.get::<_, f64>(8)? as f32,
                        confidence: r.get::<_, f64>(9)? as f32,
                        confidence_source: r.get(10)?,
                    })
                },
            )
            .optional()?;
        Ok(row)
    }

    fn apply_claim_fields(&self, claim_id: &str, upd: &ClaimFieldUpdate) -> Result<bool> {
        // Build the SET clause positionally; each `?N` is pushed in lockstep
        // with its arg so indices stay aligned (the final `?N` is the WHERE id).
        let mut sets: Vec<String> = Vec::new();
        let mut args: Vec<SqlValue> = Vec::new();
        let push = |sets: &mut Vec<String>, args: &mut Vec<SqlValue>, col: &str, v: SqlValue| {
            sets.push(format!("{col} = ?{}", args.len() + 1));
            args.push(v);
        };
        if let Some(v) = &upd.content {
            push(&mut sets, &mut args, "content", SqlValue::Text(v.clone()));
            // The stored vector now reflects the OLD content. Clear the
            // signature so the row drops out of the signature-gated vec0 search
            // (degrades to FTS-only) instead of returning a stale, semantically
            // wrong match. The caller's `reembed_claim` restores it on success;
            // if embedding is offline the NULL persists until the next reembed
            // job picks it up — strictly better than a stale hit.
            sets.push("embedding_signature = NULL".to_string());
        }
        if let Some(v) = &upd.subject {
            push(&mut sets, &mut args, "subject", SqlValue::Text(v.clone()));
        }
        if let Some(v) = &upd.predicate {
            push(&mut sets, &mut args, "predicate", SqlValue::Text(v.clone()));
        }
        if let Some(v) = &upd.object {
            push(&mut sets, &mut args, "object", SqlValue::Text(v.clone()));
        }
        if let Some(v) = &upd.tags {
            let tags_json = serde_json::to_string(v).unwrap_or_else(|_| "[]".to_string());
            push(&mut sets, &mut args, "tags_json", SqlValue::Text(tags_json));
        }
        if let Some(v) = &upd.status {
            push(&mut sets, &mut args, "status", SqlValue::Text(v.clone()));
        }
        if let Some((stype, sid)) = &upd.scope {
            push(
                &mut sets,
                &mut args,
                "scope_type",
                SqlValue::Text(stype.clone()),
            );
            push(
                &mut sets,
                &mut args,
                "scope_id",
                match sid {
                    Some(x) => SqlValue::Text(x.clone()),
                    None => SqlValue::Null,
                },
            );
        }
        if let Some(v) = upd.salience {
            push(
                &mut sets,
                &mut args,
                "salience",
                SqlValue::Real(v.clamp(0.0, 1.0) as f64),
            );
        }
        if let Some(v) = upd.confidence {
            push(
                &mut sets,
                &mut args,
                "confidence",
                SqlValue::Real(v.clamp(0.0, 1.0) as f64),
            );
        }
        if let Some(v) = &upd.confidence_source {
            push(
                &mut sets,
                &mut args,
                "confidence_source",
                SqlValue::Text(v.clone()),
            );
        }
        if sets.is_empty() {
            return Ok(false);
        }
        push(
            &mut sets,
            &mut args,
            "updated_at",
            SqlValue::Text(now_rfc3339()),
        );
        args.push(SqlValue::Text(claim_id.to_string()));
        let sql = format!(
            "UPDATE memory_claims SET {} WHERE id = ?{}",
            sets.join(", "),
            args.len()
        );
        let conn = self.backend.write_conn()?;
        let changed = conn.execute(&sql, params_from_iter(args))?;
        Ok(changed > 0)
    }

    fn reembed_claim(&self, claim_id: &str) -> Result<()> {
        let content: Option<String> = {
            let conn = self.backend.read_conn()?;
            conn.query_row(
                "SELECT content FROM memory_claims WHERE id = ?1",
                params![claim_id],
                |r| r.get(0),
            )
            .optional()?
        };
        if let Some(content) = content {
            // Write lock is NOT held here (read_conn dropped above) — safe to
            // call the embed path which re-acquires the writer.
            self.backend.embed_and_index_claim(claim_id, &content);
        }
        Ok(())
    }

    fn add_correction_evidence(
        &self,
        claim_id: &str,
        scope_type: &str,
        scope_id: Option<&str>,
        evidence_class: &str,
        quote: &str,
    ) -> Result<()> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        let access_scope = serde_json::json!({
            "scopeType": scope_type,
            "scopeId": scope_id,
        })
        .to_string();
        conn.execute(
            "INSERT INTO memory_evidence
                (id, claim_id, source_type, evidence_class, source_id, quote,
                 redaction_status, access_scope_json, weight, created_at)
             VALUES (?1, ?2, 'manual', ?3, ?4, ?5, 'raw_allowed', ?6, 1.0, ?7)",
            params![
                uuid::Uuid::new_v4().to_string(),
                claim_id,
                evidence_class,
                format!("manual:{now}"),
                quote,
                access_scope,
                now,
            ],
        )?;
        Ok(())
    }

    fn forget_claim(&self, claim_id: &str, permanent: bool) -> Result<bool> {
        let conn = self.backend.write_conn()?;
        let now = now_rfc3339();
        with_tx(&conn, || {
            // Idempotent: already-gone claim → Ok(false).
            let rowid: Option<i64> = conn
                .query_row(
                    "SELECT rowid FROM memory_claims WHERE id = ?1",
                    params![claim_id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(rowid) = rowid else {
                return Ok(false);
            };

            // The legacy memories this claim manages — needed to stop them from
            // re-injecting once the claim is gone / archived.
            let mut linked: Vec<i64> = Vec::new();
            {
                let mut stmt =
                    conn.prepare("SELECT memory_id FROM memory_claim_links WHERE claim_id = ?1")?;
                let rows = stmt.query_map(params![claim_id], |r| r.get::<_, i64>(0))?;
                for r in rows {
                    linked.push(r?);
                }
            }

            if permanent {
                // Hard-delete the claim graph. vec0 has no delete trigger →
                // remove by rowid first (the table is created lazily on first
                // embed, so tolerate it being absent); FTS is trigger-maintained
                // on the memory_claims DELETE. Evidence is dropped too (design
                // §5.3 — permanent is the only path that discards provenance).
                let _ = conn.execute(
                    "DELETE FROM memory_claims_vec WHERE rowid = ?1",
                    params![rowid],
                );
                conn.execute(
                    "DELETE FROM memory_evidence WHERE claim_id = ?1",
                    params![claim_id],
                )?;
                conn.execute(
                    "DELETE FROM memory_claim_links WHERE claim_id = ?1",
                    params![claim_id],
                )?;
                conn.execute("DELETE FROM memory_claims WHERE id = ?1", params![claim_id])?;
                // Delete only memories that this claim was the sole manager of —
                // a memory still linked to another claim is left intact.
                for mid in &linked {
                    let remaining: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM memory_claim_links WHERE memory_id = ?1",
                        params![mid],
                        |r| r.get(0),
                    )?;
                    if remaining == 0 {
                        let _ =
                            conn.execute("DELETE FROM memories_vec WHERE rowid = ?1", params![mid]);
                        conn.execute("DELETE FROM memories WHERE id = ?1", params![mid])?;
                    }
                }
            } else {
                // Archive: keep the claim + evidence as an audit trail, flip to
                // `archived` so it stops injecting, and make sure the linked
                // memories also stop — convert this claim's links to `managed`
                // so the read-time hidden-set covers them. Unpin a sole-managed
                // memory too (a pinned memory / `user_pinned` link is otherwise
                // exempt and would stay injected).
                conn.execute(
                    "UPDATE memory_claims SET status = 'archived', updated_at = ?2 WHERE id = ?1",
                    params![claim_id, now],
                )?;
                conn.execute(
                    "UPDATE memory_claim_links SET sync_mode = 'managed', updated_at = ?2
                     WHERE claim_id = ?1",
                    params![claim_id, now],
                )?;
                for mid in &linked {
                    let other_links: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM memory_claim_links
                         WHERE memory_id = ?1 AND claim_id != ?2",
                        params![mid, claim_id],
                        |r| r.get(0),
                    )?;
                    if other_links == 0 {
                        conn.execute(
                            "UPDATE memories SET pinned = 0, updated_at = ?2 WHERE id = ?1",
                            params![mid, now],
                        )?;
                    }
                }
            }
            Ok(true)
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

    #[test]
    fn search_claims_matches_active_by_fts_and_scope() {
        let s = temp_store();
        let insert = |id: &str,
                      scope_type: &str,
                      scope_id: &str,
                      content: &str,
                      status: &str,
                      valid_until: Option<&str>| {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO memory_claims (id, scope_type, scope_id, claim_type, subject, predicate, object, content, status, valid_until, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'preference', 'user', 'likes', ?4, ?4, ?5, ?6, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                params![id, scope_type, scope_id, content, status, valid_until],
            ).unwrap();
        };
        insert(
            "c1",
            "global",
            "",
            "rust programming language",
            "active",
            None,
        );
        insert("c2", "global", "", "typescript frontend", "active", None);
        insert(
            "c3",
            "agent",
            "ha-main",
            "rust agent scoped",
            "active",
            None,
        );
        insert("c4", "global", "", "archived rust", "archived", None);
        insert(
            "c5",
            "global",
            "",
            "expired rust",
            "active",
            Some("2020-01-01T00:00:00.000Z"),
        );

        // FTS "rust" across all scopes: active c1/c3 in; archived c4 + effective-expired c5 out.
        let ids: Vec<String> = s
            .search_claims("rust", None, 10)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(
            ids.contains(&"c1".to_string()),
            "active global rust: {ids:?}"
        );
        assert!(
            ids.contains(&"c3".to_string()),
            "active agent rust: {ids:?}"
        );
        assert!(
            !ids.contains(&"c4".to_string()),
            "archived excluded: {ids:?}"
        );
        assert!(
            !ids.contains(&"c5".to_string()),
            "expired excluded: {ids:?}"
        );
        assert!(
            !ids.contains(&"c2".to_string()),
            "non-matching excluded: {ids:?}"
        );

        // Scope filter: global only excludes the agent-scoped match.
        let gids: Vec<String> = s
            .search_claims("rust", Some(&MemoryScope::Global), 10)
            .unwrap()
            .into_iter()
            .map(|c| c.id)
            .collect();
        assert!(gids.contains(&"c1".to_string()));
        assert!(
            !gids.contains(&"c3".to_string()),
            "agent scope excluded: {gids:?}"
        );

        // No token / no match → empty.
        assert!(s.search_claims("zzzznomatch", None, 10).unwrap().is_empty());
        assert!(s.search_claims("   ", None, 10).unwrap().is_empty());
    }

    fn seed_claim(s: &ClaimStore, id: &str, status: &str) {
        let conn = s.backend.write_conn().unwrap();
        conn.execute(
            "INSERT INTO memory_claims (id, scope_type, claim_type, subject, predicate, object, content, status, created_at, updated_at)
             VALUES (?1, 'global', 'preference', 'user', 'uses', ?1, 'c', ?2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            params![id, status],
        ).unwrap();
    }
    fn seed_evidence(s: &ClaimStore, ev_id: &str, claim_id: &str) {
        let conn = s.backend.write_conn().unwrap();
        conn.execute(
            "INSERT INTO memory_evidence (id, claim_id, source_type, source_id, created_at)
             VALUES (?1, ?2, 'session_message', 'sess:1', '2026-01-01T00:00:00.000Z')",
            params![ev_id, claim_id],
        )
        .unwrap();
    }
    fn evidence_owner(s: &ClaimStore, ev_id: &str) -> String {
        let conn = s.backend.read_conn().unwrap();
        conn.query_row(
            "SELECT claim_id FROM memory_evidence WHERE id = ?1",
            params![ev_id],
            |r| r.get(0),
        )
        .unwrap()
    }
    fn claim_status(s: &ClaimStore, id: &str) -> String {
        let conn = s.backend.read_conn().unwrap();
        conn.query_row(
            "SELECT status FROM memory_claims WHERE id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap()
    }

    #[test]
    fn merge_claims_archives_drop_then_moves_evidence() {
        let s = temp_store();
        seed_claim(&s, "keep", "active");
        seed_claim(&s, "drop", "active");
        seed_evidence(&s, "ev1", "drop");

        assert!(s.merge_claims("keep", "drop").unwrap());
        assert_eq!(claim_status(&s, "drop"), "archived");
        assert_eq!(claim_status(&s, "keep"), "active");
        // Evidence folded onto the survivor.
        assert_eq!(evidence_owner(&s, "ev1"), "keep");
    }

    #[test]
    fn merge_claims_skips_when_drop_not_active_without_moving_evidence() {
        // drop raced to a terminal state before the merge applied → must NOT
        // re-point evidence (Codex finding: stale drop silently moved evidence).
        let s = temp_store();
        seed_claim(&s, "keep", "active");
        seed_claim(&s, "drop", "archived");
        seed_evidence(&s, "ev1", "drop");

        assert!(!s.merge_claims("keep", "drop").unwrap());
        // Evidence stays on the drop — untouched.
        assert_eq!(evidence_owner(&s, "ev1"), "drop");
    }

    #[test]
    fn merge_claims_skips_when_keep_not_active() {
        let s = temp_store();
        seed_claim(&s, "keep", "archived");
        seed_claim(&s, "drop", "active");
        seed_evidence(&s, "ev1", "drop");

        assert!(!s.merge_claims("keep", "drop").unwrap());
        // Drop not archived, evidence not moved.
        assert_eq!(claim_status(&s, "drop"), "active");
        assert_eq!(evidence_owner(&s, "ev1"), "drop");
    }

    // ── User-correction primitives (Phase 6) ──

    fn seed_memory(s: &ClaimStore, id: i64, pinned: bool) {
        let conn = s.backend.write_conn().unwrap();
        conn.execute(
            "INSERT INTO memories (id, memory_type, scope_type, content, tags, source, pinned, created_at, updated_at)
             VALUES (?1, 'fact', 'global', 'm', '[]', 'manual', ?2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            params![id, pinned as i64],
        )
        .unwrap();
    }
    fn memory_exists(s: &ClaimStore, id: i64) -> bool {
        let conn = s.backend.read_conn().unwrap();
        conn.query_row("SELECT 1 FROM memories WHERE id = ?1", params![id], |_| {
            Ok(())
        })
        .optional()
        .unwrap()
        .is_some()
    }
    fn memory_pinned(s: &ClaimStore, id: i64) -> bool {
        let conn = s.backend.read_conn().unwrap();
        conn.query_row(
            "SELECT pinned FROM memories WHERE id = ?1",
            params![id],
            |r| r.get::<_, i64>(0),
        )
        .unwrap()
            != 0
    }

    #[test]
    fn apply_claim_fields_updates_content_and_status() {
        let s = temp_store();
        seed_claim(&s, "c1", "needs_review");
        let upd = ClaimFieldUpdate {
            content: Some("rewritten".into()),
            status: Some("active".into()),
            ..Default::default()
        };
        assert!(s.apply_claim_fields("c1", &upd).unwrap());
        let st = s.claim_edit_state("c1").unwrap().unwrap();
        assert_eq!(st.content, "rewritten");
        assert_eq!(st.status, "active");
    }

    #[test]
    fn apply_claim_fields_can_move_to_global_clearing_id() {
        let s = temp_store();
        // Seed an agent-scoped claim, then move it to global.
        {
            let conn = s.backend.write_conn().unwrap();
            conn.execute(
                "INSERT INTO memory_claims (id, scope_type, scope_id, claim_type, subject, predicate, object, content, status, created_at, updated_at)
                 VALUES ('c1', 'agent', 'ha-main', 'preference', 'user', 'uses', 'x', 'c', 'active', '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                [],
            ).unwrap();
        }
        let upd = ClaimFieldUpdate {
            scope: Some(("global".into(), None)),
            ..Default::default()
        };
        assert!(s.apply_claim_fields("c1", &upd).unwrap());
        let st = s.claim_edit_state("c1").unwrap().unwrap();
        assert_eq!(st.scope_type, "global");
        assert_eq!(st.scope_id, None);
    }

    #[test]
    fn apply_claim_fields_empty_is_noop() {
        let s = temp_store();
        seed_claim(&s, "c1", "active");
        assert!(!s
            .apply_claim_fields("c1", &ClaimFieldUpdate::default())
            .unwrap());
    }

    fn claim_signature(s: &ClaimStore, id: &str) -> Option<String> {
        let conn = s.backend.read_conn().unwrap();
        conn.query_row(
            "SELECT embedding_signature FROM memory_claims WHERE id = ?1",
            params![id],
            |r| r.get::<_, Option<String>>(0),
        )
        .unwrap()
    }

    #[test]
    fn editing_content_clears_embedding_signature() {
        let s = temp_store();
        seed_claim(&s, "c1", "active");
        // Stamp a signature as if the claim had been embedded.
        s.backend
            .write_conn()
            .unwrap()
            .execute(
                "UPDATE memory_claims SET embedding_signature = 'sig-v1' WHERE id = 'c1'",
                [],
            )
            .unwrap();
        assert_eq!(claim_signature(&s, "c1").as_deref(), Some("sig-v1"));

        // Content edit must clear the signature (the stale vector drops out of
        // the signature-gated vec0 search until re-embedded).
        let upd = ClaimFieldUpdate {
            content: Some("new content".into()),
            ..Default::default()
        };
        assert!(s.apply_claim_fields("c1", &upd).unwrap());
        assert_eq!(claim_signature(&s, "c1"), None);
    }

    #[test]
    fn status_only_update_keeps_embedding_signature() {
        let s = temp_store();
        seed_claim(&s, "c1", "needs_review");
        s.backend
            .write_conn()
            .unwrap()
            .execute(
                "UPDATE memory_claims SET embedding_signature = 'sig-v1' WHERE id = 'c1'",
                [],
            )
            .unwrap();
        // A non-content edit must NOT clear the signature (the vector is still
        // valid for the unchanged text).
        let upd = ClaimFieldUpdate {
            status: Some("active".into()),
            ..Default::default()
        };
        assert!(s.apply_claim_fields("c1", &upd).unwrap());
        assert_eq!(claim_signature(&s, "c1").as_deref(), Some("sig-v1"));
    }

    #[test]
    fn forget_archive_hides_sole_managed_pinned_memory() {
        let s = temp_store();
        seed_claim(&s, "c1", "active");
        seed_memory(&s, 1, true);
        s.link_claim_memory("c1", 1, "user_pinned").unwrap();

        assert!(s.forget_claim("c1", false).unwrap());
        // Claim archived (kept), link converted to managed, memory unpinned so
        // the read-time hidden-set covers it.
        assert_eq!(claim_status(&s, "c1"), "archived");
        assert!(memory_exists(&s, 1));
        assert!(!memory_pinned(&s, 1));
        let conn = s.backend.read_conn().unwrap();
        let mode: String = conn
            .query_row(
                "SELECT sync_mode FROM memory_claim_links WHERE claim_id = 'c1' AND memory_id = 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(mode, "managed");
    }

    #[test]
    fn forget_permanent_deletes_graph_and_orphan_memory() {
        let s = temp_store();
        seed_claim(&s, "c1", "active");
        seed_evidence(&s, "ev1", "c1");
        seed_memory(&s, 1, false);
        s.link_claim_memory("c1", 1, "managed").unwrap();

        assert!(s.forget_claim("c1", true).unwrap());
        // Claim, evidence, link, and the orphaned memory are all gone.
        let conn = s.backend.read_conn().unwrap();
        assert!(conn
            .query_row("SELECT 1 FROM memory_claims WHERE id='c1'", [], |_| Ok(()))
            .optional()
            .unwrap()
            .is_none());
        assert!(conn
            .query_row("SELECT 1 FROM memory_evidence WHERE id='ev1'", [], |_| Ok(
                ()
            ))
            .optional()
            .unwrap()
            .is_none());
        assert!(!memory_exists(&s, 1));
    }

    #[test]
    fn forget_permanent_keeps_memory_shared_with_another_claim() {
        let s = temp_store();
        seed_claim(&s, "c1", "active");
        seed_claim(&s, "c2", "active");
        seed_memory(&s, 1, false);
        s.link_claim_memory("c1", 1, "managed").unwrap();
        s.link_claim_memory("c2", 1, "managed").unwrap();

        assert!(s.forget_claim("c1", true).unwrap());
        // Memory still managed by c2 → left intact.
        assert!(memory_exists(&s, 1));
        assert_eq!(claim_status(&s, "c2"), "active");
    }

    #[test]
    fn forget_missing_claim_is_noop() {
        let s = temp_store();
        assert!(!s.forget_claim("nope", false).unwrap());
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
