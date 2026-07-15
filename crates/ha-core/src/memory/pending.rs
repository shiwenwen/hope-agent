//! Review-first and unassigned learning candidates.
//!
//! Pending rows are deliberately outside the normal `memories` / claims read
//! paths, so they cannot enter prompts or recall before an owner approves a
//! scope. This table is durable owner-plane state in memory.db.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex};

use super::{MemoryScope, MemoryType, NewMemory};

// Hope Agent enforces one process per data directory. Serializing the two
// multi-store workflows here makes the pending status re-check authoritative
// before any memory/claim/Core side effect runs. The underlying dedup and
// stale-write guards remain the crash-recovery safety net.
static PENDING_APPROVAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static CORE_PROMOTION_PROPOSAL_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));
static PENDING_ADD_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingMemoryReason {
    ReviewFirst,
    ProjectScopeMissing,
    ScopeUncertain,
    Sensitive,
    Conflict,
    CorePromotion,
}

impl PendingMemoryReason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::ReviewFirst => "review_first",
            Self::ProjectScopeMissing => "project_scope_missing",
            Self::ScopeUncertain => "scope_uncertain",
            Self::Sensitive => "sensitive",
            Self::Conflict => "conflict",
            Self::CorePromotion => "core_promotion",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewPendingMemoryCandidate {
    pub content: String,
    pub memory_type: MemoryType,
    pub tags: Vec<String>,
    pub source_session_id: String,
    pub agent_id: String,
    pub reason: PendingMemoryReason,
    pub suggested_scope: Option<MemoryScope>,
    #[serde(default)]
    pub candidate_kind: String,
    #[serde(default)]
    pub payload_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMemoryCandidate {
    pub id: String,
    pub content: String,
    pub memory_type: MemoryType,
    pub tags: Vec<String>,
    pub source_session_id: String,
    pub agent_id: String,
    pub reason: PendingMemoryReason,
    pub suggested_scope: Option<MemoryScope>,
    pub candidate_kind: String,
    pub payload_json: Option<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMemoryCandidatePage {
    pub items: Vec<PendingMemoryCandidate>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    /// Exact status-scoped counts for overview badges. This avoids deriving
    /// Unassigned from the bounded first page when an inbox exceeds 100 rows.
    pub reason_counts: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CorePromotionPayload {
    memory_id: i64,
}

/// Queue a review-only Core promotion for an already persisted dynamic
/// memory. The source stays in the dynamic store regardless of the decision;
/// approving this row copies a bounded representation into `MEMORY.md`.
pub fn propose_core_promotion(
    memory_id: i64,
    content: &str,
    memory_type: MemoryType,
    tags: Vec<String>,
    source_session_id: &str,
    agent_id: &str,
    suggested_scope: MemoryScope,
) -> Result<Option<String>> {
    let _guard = CORE_PROMOTION_PROPOSAL_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let payload_json = serde_json::to_string(&CorePromotionPayload { memory_id })?;
    let conn = open()?;
    let exists = conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM pending_memory_candidates
             WHERE reason = 'core_promotion' AND status = 'pending' AND payload_json = ?1
         )",
        params![payload_json],
        |row| row.get::<_, bool>(0),
    )?;
    drop(conn);
    if exists {
        return Ok(None);
    }
    add(NewPendingMemoryCandidate {
        content: content.to_string(),
        memory_type,
        tags,
        source_session_id: source_session_id.to_string(),
        agent_id: agent_id.to_string(),
        reason: PendingMemoryReason::CorePromotion,
        suggested_scope: Some(suggested_scope),
        candidate_kind: "memory".to_string(),
        payload_json: Some(payload_json),
    })
    .map(Some)
}

pub fn add(candidate: NewPendingMemoryCandidate) -> Result<String> {
    validate_candidate(&candidate)?;
    let _guard = PENDING_ADD_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let conn = open()?;
    if let Some(existing_id) = find_equivalent_pending(&conn, &candidate)? {
        return Ok(existing_id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let reason = candidate.reason.clone();
    let now = chrono::Utc::now().to_rfc3339();
    let (scope_type, scope_id) = candidate
        .suggested_scope
        .as_ref()
        .map(scope_parts)
        .unwrap_or((None, None));
    conn.execute(
        "INSERT INTO pending_memory_candidates (
            id, content, memory_type, tags_json, source_session_id, agent_id,
            reason, suggested_scope_type, suggested_scope_id, candidate_kind,
            payload_json, status, created_at, updated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'pending', ?12, ?12)",
        params![
            id,
            candidate.content.trim(),
            candidate.memory_type.as_str(),
            serde_json::to_string(&candidate.tags)?,
            candidate.source_session_id,
            candidate.agent_id,
            candidate.reason.as_str(),
            scope_type,
            scope_id,
            if candidate.candidate_kind.trim().is_empty() {
                "memory"
            } else {
                candidate.candidate_kind.trim()
            },
            candidate.payload_json,
            now,
        ],
    )?;
    if let Some(bus) = crate::get_event_bus() {
        let payload = serde_json::json!({
            "id": id,
            "reason": reason.as_str(),
            "candidateKind": if candidate.candidate_kind.trim().is_empty() {
                "memory"
            } else {
                candidate.candidate_kind.trim()
            },
        });
        bus.emit("memory:learning_candidate_created", payload.clone());
        match reason {
            PendingMemoryReason::ProjectScopeMissing | PendingMemoryReason::ScopeUncertain => {
                bus.emit("memory:unassigned_created", payload);
            }
            PendingMemoryReason::CorePromotion => {
                bus.emit("memory:promotion_proposed", payload);
            }
            _ => {}
        }
    }
    Ok(id)
}

/// Review-first extraction can revisit the same conversation before the user
/// opens the inbox. Keep one unresolved row per semantic candidate instead of
/// flooding the UI. This is intentionally scoped to pending rows: once the
/// user approves or rejects an item, a genuinely new extraction may surface it
/// again for a later conversation.
fn find_equivalent_pending(
    conn: &Connection,
    candidate: &NewPendingMemoryCandidate,
) -> Result<Option<String>> {
    let candidate_kind = if candidate.candidate_kind.trim().is_empty() {
        "memory"
    } else {
        candidate.candidate_kind.trim()
    };
    let (scope_type, scope_id) = candidate
        .suggested_scope
        .as_ref()
        .map(scope_parts)
        .unwrap_or((None, None));
    let canonical = canonical_pending_content(&candidate.content);
    let mut stmt = conn.prepare(
        "SELECT id, content, payload_json, candidate_kind
           FROM pending_memory_candidates
          WHERE status = 'pending'
            AND agent_id = ?1
            AND reason = ?2
            AND suggested_scope_type IS ?3
            AND suggested_scope_id IS ?4",
    )?;
    let rows = stmt
        .query_map(
            params![
                candidate.agent_id,
                candidate.reason.as_str(),
                scope_type,
                scope_id,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for row in rows {
        let (id, content, payload_json, existing_kind) = row;
        if canonical_pending_content(&content) != canonical {
            continue;
        }
        if existing_kind == "claim" {
            // A structured candidate already preserves at least as much
            // information as the matching fact row would.
            return Ok(Some(id));
        }
        if candidate_kind == "claim" {
            // Combined extraction commonly emits the same fact in both
            // arrays. Upgrade the first pending row in place so Review shows
            // one item while retaining the richer structured payload.
            conn.execute(
                "UPDATE pending_memory_candidates
                    SET candidate_kind = 'claim', payload_json = ?2,
                        memory_type = ?3, tags_json = ?4, updated_at = ?5
                  WHERE id = ?1 AND status = 'pending'",
                params![
                    id,
                    candidate.payload_json,
                    candidate.memory_type.as_str(),
                    serde_json::to_string(&candidate.tags)?,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )?;
            return Ok(Some(id));
        }
        if payload_json.as_deref().unwrap_or_default()
            == candidate.payload_json.as_deref().unwrap_or_default()
        {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn canonical_pending_content(content: &str) -> String {
    content
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

pub fn list(
    status: Option<&str>,
    offset: usize,
    limit: usize,
) -> Result<PendingMemoryCandidatePage> {
    let conn = open()?;
    let status = status.unwrap_or("pending");
    let limit = limit.clamp(1, 100);
    let total = conn.query_row(
        "SELECT COUNT(*) FROM pending_memory_candidates WHERE status = ?1",
        params![status],
        |row| row.get::<_, i64>(0),
    )? as usize;
    let mut reason_counts = BTreeMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT reason, COUNT(*) FROM pending_memory_candidates
             WHERE status = ?1 GROUP BY reason ORDER BY reason",
        )?;
        let rows = stmt.query_map(params![status], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)? as usize))
        })?;
        for row in rows {
            let (reason, count) = row?;
            reason_counts.insert(reason, count);
        }
    }
    let offset = offset.min(total);
    let mut stmt = conn.prepare(
        "SELECT id, content, memory_type, tags_json, source_session_id, agent_id,
                reason, suggested_scope_type, suggested_scope_id, candidate_kind,
                payload_json, status, created_at, updated_at
         FROM pending_memory_candidates WHERE status = ?1
         ORDER BY created_at DESC, id DESC LIMIT ?2 OFFSET ?3",
    )?;
    let rows = stmt.query_map(params![status, limit as i64, offset as i64], row_candidate)?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(PendingMemoryCandidatePage {
        items,
        total,
        offset,
        limit,
        reason_counts,
    })
}

pub fn get(id: &str) -> Result<Option<PendingMemoryCandidate>> {
    let conn = open()?;
    conn.query_row(
        "SELECT id, content, memory_type, tags_json, source_session_id, agent_id,
                reason, suggested_scope_type, suggested_scope_id, candidate_kind,
                payload_json, status, created_at, updated_at
         FROM pending_memory_candidates WHERE id = ?1",
        params![id],
        row_candidate,
    )
    .optional()
    .map_err(Into::into)
}

pub fn approve(id: &str, scope: MemoryScope) -> Result<i64> {
    let _guard = PENDING_APPROVAL_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let candidate =
        get(id)?.ok_or_else(|| anyhow::anyhow!("pending memory candidate not found"))?;
    if candidate.status != "pending" {
        anyhow::bail!("pending memory candidate has already been resolved");
    }
    validate_scope(&scope)?;
    if candidate.reason == PendingMemoryReason::CorePromotion {
        let payload = candidate
            .payload_json
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("Core promotion payload is missing"))?;
        let payload: CorePromotionPayload =
            serde_json::from_str(payload).context("invalid Core promotion payload")?;
        let (scope_type, scope_id) = match &scope {
            MemoryScope::Global => ("global", None),
            MemoryScope::Agent { id } => ("agent", Some(id.clone())),
            MemoryScope::Project { id } => ("project", Some(id.clone())),
        };
        super::core_repository::promote(super::core_repository::CoreMemoryPromotionInput {
            source_kind: super::core_repository::CoreMemoryPromotionSourceKind::Memory,
            source_id: payload.memory_id.to_string(),
            scope_type: scope_type.to_string(),
            scope_id,
            topic_name: None,
        })?;
        set_status(id, "approved")?;
        return Ok(payload.memory_id);
    }
    let claim = if candidate.candidate_kind == "claim" {
        let payload = candidate
            .payload_json
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("pending claim payload is missing"))?;
        Some(
            serde_json::from_str::<super::claims::ClaimCandidate>(payload)
                .context("invalid pending claim payload")?,
        )
    } else {
        None
    };
    let backend = crate::get_memory_backend()
        .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;
    let dedup = super::load_dedup_config();
    let entry = NewMemory {
        memory_type: candidate.memory_type.clone(),
        scope: scope.clone(),
        content: candidate.content.clone(),
        tags: candidate.tags.clone(),
        source: "review-approved".to_string(),
        source_session_id: Some(candidate.source_session_id.clone()),
        pinned: false,
        attachment_path: None,
        attachment_mime: None,
    };
    let (memory_id, sync_mode) =
        match backend.add_with_dedup(entry, dedup.threshold_high, dedup.threshold_merge)? {
            super::AddResult::Created { id } => (id, "managed"),
            super::AddResult::Updated { id } => (id, "detached"),
            super::AddResult::Duplicate { existing_id, .. } => (existing_id, "detached"),
        };
    if let Some(claim) = claim {
        let outcome = super::claims::write_claim_candidate_with_status(
            &claim,
            &scope,
            &candidate.source_session_id,
            None,
            None,
        )?;
        super::claims::link_claim_memory(&outcome.claim_id, memory_id, sync_mode)?;
    }
    if matches!(
        candidate.memory_type,
        MemoryType::User | MemoryType::Feedback
    ) && candidate.tags.iter().any(|tag| tag == "core_candidate")
    {
        let _ = propose_core_promotion(
            memory_id,
            &candidate.content,
            candidate.memory_type.clone(),
            candidate.tags.clone(),
            &candidate.source_session_id,
            &candidate.agent_id,
            scope,
        )?;
    }
    set_status(id, "approved")?;
    Ok(memory_id)
}

pub fn reject(id: &str) -> Result<()> {
    let _guard = PENDING_APPROVAL_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    set_status(id, "rejected")
}

fn set_status(id: &str, status: &str) -> Result<()> {
    let conn = open()?;
    let changed = conn.execute(
        "UPDATE pending_memory_candidates SET status = ?2, updated_at = ?3
         WHERE id = ?1 AND status = 'pending'",
        params![id, status, chrono::Utc::now().to_rfc3339()],
    )?;
    if changed == 0 {
        anyhow::bail!("pending memory candidate not found or already resolved");
    }
    Ok(())
}

fn open() -> Result<Connection> {
    let path = crate::paths::memory_db_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(&path)
        .with_context(|| format!("open pending memory store {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS pending_memory_candidates (
            id TEXT PRIMARY KEY,
            content TEXT NOT NULL,
            memory_type TEXT NOT NULL,
            tags_json TEXT NOT NULL DEFAULT '[]',
            source_session_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            reason TEXT NOT NULL,
            suggested_scope_type TEXT,
            suggested_scope_id TEXT,
            candidate_kind TEXT NOT NULL DEFAULT 'memory',
            payload_json TEXT,
            status TEXT NOT NULL DEFAULT 'pending'
                CHECK (status IN ('pending', 'approved', 'rejected')),
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_pending_memory_status_created
            ON pending_memory_candidates(status, created_at DESC);
         CREATE INDEX IF NOT EXISTS idx_pending_memory_session
            ON pending_memory_candidates(source_session_id, status);",
    )?;
    Ok(conn)
}

fn validate_candidate(candidate: &NewPendingMemoryCandidate) -> Result<()> {
    if candidate.content.trim().is_empty() {
        anyhow::bail!("pending memory candidate content is empty");
    }
    if candidate.content.len() > 16 * 1024 {
        anyhow::bail!("pending memory candidate is too large");
    }
    if !candidate.candidate_kind.trim().is_empty()
        && !matches!(candidate.candidate_kind.trim(), "memory" | "claim")
    {
        anyhow::bail!("unsupported pending memory candidate kind");
    }
    if candidate.candidate_kind.trim() == "claim" && candidate.payload_json.is_none() {
        anyhow::bail!("pending claim payload is required");
    }
    crate::paths::validate_agent_id(&candidate.agent_id)?;
    Ok(())
}

fn validate_scope(scope: &MemoryScope) -> Result<()> {
    match scope {
        MemoryScope::Global => Ok(()),
        MemoryScope::Agent { id } => crate::paths::validate_agent_id(id),
        MemoryScope::Project { id } => uuid::Uuid::parse_str(id)
            .map(|_| ())
            .map_err(|_| anyhow::anyhow!("invalid project id")),
    }
}

fn scope_parts(scope: &MemoryScope) -> (Option<&'static str>, Option<String>) {
    match scope {
        MemoryScope::Global => (Some("global"), None),
        MemoryScope::Agent { id } => (Some("agent"), Some(id.clone())),
        MemoryScope::Project { id } => (Some("project"), Some(id.clone())),
    }
}

fn scope_from_parts(scope_type: Option<String>, scope_id: Option<String>) -> Option<MemoryScope> {
    match scope_type.as_deref() {
        Some("global") => Some(MemoryScope::Global),
        Some("agent") => scope_id.map(|id| MemoryScope::Agent { id }),
        Some("project") => scope_id.map(|id| MemoryScope::Project { id }),
        _ => None,
    }
}

fn reason_from_str(value: String) -> PendingMemoryReason {
    match value.as_str() {
        "review_first" => PendingMemoryReason::ReviewFirst,
        "project_scope_missing" => PendingMemoryReason::ProjectScopeMissing,
        "sensitive" => PendingMemoryReason::Sensitive,
        "conflict" => PendingMemoryReason::Conflict,
        "core_promotion" => PendingMemoryReason::CorePromotion,
        _ => PendingMemoryReason::ScopeUncertain,
    }
}

fn row_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<PendingMemoryCandidate> {
    let tags_json: String = row.get(3)?;
    Ok(PendingMemoryCandidate {
        id: row.get(0)?,
        content: row.get(1)?,
        memory_type: MemoryType::from_str(&row.get::<_, String>(2)?),
        tags: serde_json::from_str(&tags_json).unwrap_or_default(),
        source_session_id: row.get(4)?,
        agent_id: row.get(5)?,
        reason: reason_from_str(row.get(6)?),
        suggested_scope: scope_from_parts(row.get(7)?, row.get(8)?),
        candidate_kind: row.get(9)?,
        payload_json: row.get(10)?,
        status: row.get(11)?,
        created_at: row.get(12)?,
        updated_at: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_candidates_are_separate_until_approved() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let id = add(NewPendingMemoryCandidate {
                content: "Project Alpha uses Rust".into(),
                memory_type: MemoryType::Project,
                tags: vec!["alpha".into()],
                source_session_id: "s1".into(),
                agent_id: "ha-main".into(),
                reason: PendingMemoryReason::ProjectScopeMissing,
                suggested_scope: None,
                candidate_kind: "memory".into(),
                payload_json: None,
            })
            .unwrap();
            let page = list(Some("pending"), 0, 20).unwrap();
            assert_eq!(page.total, 1);
            assert_eq!(page.items[0].id, id);
            assert_eq!(page.reason_counts.get("project_scope_missing"), Some(&1));
            reject(&id).unwrap();
            assert_eq!(list(Some("pending"), 0, 20).unwrap().total, 0);
            assert_eq!(list(Some("rejected"), 0, 20).unwrap().total, 1);
        });
    }

    #[test]
    fn core_promotion_proposals_are_review_only_and_deduplicated() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let first = propose_core_promotion(
                42,
                "Always answer in Chinese",
                MemoryType::Feedback,
                vec!["core_candidate".into()],
                "s1",
                "ha-main",
                MemoryScope::Agent {
                    id: "ha-main".into(),
                },
            )
            .unwrap();
            assert!(first.is_some());
            let duplicate = propose_core_promotion(
                42,
                "Always answer in Chinese",
                MemoryType::Feedback,
                vec!["core_candidate".into()],
                "s2",
                "ha-main",
                MemoryScope::Agent {
                    id: "ha-main".into(),
                },
            )
            .unwrap();
            assert!(duplicate.is_none());
            let page = list(Some("pending"), 0, 20).unwrap();
            assert_eq!(page.total, 1);
            assert_eq!(page.items[0].reason, PendingMemoryReason::CorePromotion);
            assert_eq!(page.items[0].status, "pending");
        });
    }

    #[test]
    fn repeated_review_first_candidate_reuses_the_pending_row() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let candidate = || NewPendingMemoryCandidate {
                content: "  Always   answer in Chinese ".into(),
                memory_type: MemoryType::Feedback,
                tags: vec!["preference".into()],
                source_session_id: "s1".into(),
                agent_id: "ha-main".into(),
                reason: PendingMemoryReason::ReviewFirst,
                suggested_scope: Some(MemoryScope::Agent {
                    id: "ha-main".into(),
                }),
                candidate_kind: "memory".into(),
                payload_json: None,
            };
            let first = add(candidate()).unwrap();
            let mut repeated = candidate();
            repeated.content = "always answer in chinese".into();
            repeated.source_session_id = "s2".into();
            let second = add(repeated).unwrap();
            assert_eq!(first, second);
            assert_eq!(list(Some("pending"), 0, 20).unwrap().total, 1);
        });
    }

    #[test]
    fn matching_fact_and_claim_share_one_richer_pending_row() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let memory_id = add(NewPendingMemoryCandidate {
                content: "Always answer in Chinese".into(),
                memory_type: MemoryType::Feedback,
                tags: vec!["preference".into()],
                source_session_id: "s1".into(),
                agent_id: "ha-main".into(),
                reason: PendingMemoryReason::ReviewFirst,
                suggested_scope: Some(MemoryScope::Agent {
                    id: "ha-main".into(),
                }),
                candidate_kind: "memory".into(),
                payload_json: None,
            })
            .unwrap();
            let claim_id = add(NewPendingMemoryCandidate {
                content: "always  answer in chinese".into(),
                memory_type: MemoryType::Feedback,
                tags: vec!["preference".into(), "structured".into()],
                source_session_id: "s1".into(),
                agent_id: "ha-main".into(),
                reason: PendingMemoryReason::ReviewFirst,
                suggested_scope: Some(MemoryScope::Agent {
                    id: "ha-main".into(),
                }),
                candidate_kind: "claim".into(),
                payload_json: Some("{\"claimType\":\"preference\"}".into()),
            })
            .unwrap();

            assert_eq!(memory_id, claim_id);
            let page = list(Some("pending"), 0, 20).unwrap();
            assert_eq!(page.total, 1);
            assert_eq!(page.items[0].candidate_kind, "claim");
            assert_eq!(
                page.items[0].payload_json.as_deref(),
                Some("{\"claimType\":\"preference\"}")
            );
        });
    }
}
