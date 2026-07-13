//! Unified file-backed Core Memory repository.
//!
//! V2 canonicalises all scope indexes as uppercase `MEMORY.md`. Global and
//! Agent scopes retain their legacy lowercase files as synchronized mirrors
//! during the compatibility window. A content-free manifest distinguishes an
//! old-version write from a new-version write and fails closed on true
//! two-sided conflicts.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

pub const CORE_INDEX_FILE: &str = "MEMORY.md";
pub const CORE_INDEX_MAX_BYTES: usize = 25 * 1024;
pub const CORE_INDEX_MAX_LINES: usize = 200;
pub const CORE_TOPIC_MAX_BYTES: usize = 128 * 1024;
pub const CORE_MAX_TOPIC_FILES: usize = 256;

const MANIFEST_VERSION: u32 = 1;
const LOCK_TIMEOUT: Duration = Duration::from_secs(10);
const LOCK_POLL: Duration = Duration::from_millis(10);
const TOPICS_DIR: &str = "topics";
const MEMORY_TYPES: [&str; 4] = ["feedback", "project", "reference", "user"];

// A Core snapshot is part of a session's semantic state, not a best-effort
// performance cache. An LRU would silently change an old session's system
// prefix once enough other sessions had been used. Entries are therefore kept
// until an explicit reload/compaction/policy transition or session cleanup;
// process restart remains a natural reload boundary.
static SESSION_SNAPSHOTS: LazyLock<Mutex<HashMap<String, CoreMemorySnapshot>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreMemoryScope {
    Global,
    Agent { id: String },
    Project { id: String },
}

impl CoreMemoryScope {
    pub fn key(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Agent { id } => format!("agent:{id}"),
            Self::Project { id } => format!("project:{id}"),
        }
    }

    pub fn scope_type(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Agent { .. } => "agent",
            Self::Project { .. } => "project",
        }
    }

    pub fn scope_id(&self) -> Option<&str> {
        match self {
            Self::Global => None,
            Self::Agent { id } | Self::Project { id } => Some(id),
        }
    }
}

pub fn emit_core_changed(scope: &CoreMemoryScope, action: &str) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "memory:core_changed",
            serde_json::json!({
                "scopeType": scope.scope_type(),
                "scopeId": scope.scope_id(),
                "action": action,
            }),
        );
    }
}

#[derive(Debug, Clone)]
pub struct CoreMemoryPaths {
    pub dir: PathBuf,
    pub canonical: PathBuf,
    pub legacy: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreMemoryMigrationState {
    Canonical,
    Mirrored,
    MigratedLegacy,
    RecoveredLegacyChange,
    RecoveredCanonicalChange,
    Conflict,
    Empty,
}

impl CoreMemoryMigrationState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Canonical => "canonical",
            Self::Mirrored => "mirrored",
            Self::MigratedLegacy => "migrated_legacy",
            Self::RecoveredLegacyChange => "recovered_legacy_change",
            Self::RecoveredCanonicalChange => "recovered_canonical_change",
            Self::Conflict => "conflict",
            Self::Empty => "empty",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryIndex {
    pub content: Option<String>,
    pub file_hash: Option<String>,
    pub state: CoreMemoryMigrationState,
    pub canonical_path: PathBuf,
    pub legacy_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryConflict {
    pub canonical_content: String,
    pub canonical_hash: String,
    pub legacy_content: String,
    pub legacy_hash: String,
    pub last_synced_content: Option<String>,
    pub last_synced_hash: Option<String>,
    pub canonical_path: PathBuf,
    pub legacy_path: PathBuf,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreMemoryConflictChoice {
    Canonical,
    Legacy,
    Merged,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryConflictResolution {
    pub choice: CoreMemoryConflictChoice,
    pub expected_canonical_hash: String,
    pub expected_legacy_hash: String,
    #[serde(default)]
    pub merged_content: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryStats {
    pub index_bytes: usize,
    pub estimated_tokens: u32,
    pub index_entry_count: usize,
    pub topic_count: usize,
    pub updated_at: Option<String>,
    pub state: CoreMemoryMigrationState,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CoreMemoryPromotionSourceKind {
    Memory,
    Claim,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryPromotionInput {
    pub source_kind: CoreMemoryPromotionSourceKind,
    pub source_id: String,
    pub scope_type: String,
    #[serde(default)]
    pub scope_id: Option<String>,
    /// Optional topic name. When set, the full source is written to a topic
    /// and the bounded index receives only the generated topic link.
    #[serde(default)]
    pub topic_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryPromotionResult {
    pub source_kind: String,
    pub source_id: String,
    pub scope_key: String,
    pub index_file_hash: Option<String>,
    pub topic_file_name: Option<String>,
    pub already_present: bool,
}

pub fn parse_scope(scope_type: &str, scope_id: Option<&str>) -> Result<CoreMemoryScope> {
    match scope_type.trim().to_ascii_lowercase().as_str() {
        "global" => Ok(CoreMemoryScope::Global),
        "agent" => {
            let id = scope_id
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("scopeId is required for Agent Core Memory"))?;
            crate::paths::validate_agent_id(id)?;
            Ok(CoreMemoryScope::Agent { id: id.to_string() })
        }
        "project" => {
            let id = scope_id
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .ok_or_else(|| anyhow::anyhow!("scopeId is required for Project Core Memory"))?;
            uuid::Uuid::parse_str(id).map_err(|_| anyhow::anyhow!("invalid project id"))?;
            Ok(CoreMemoryScope::Project { id: id.to_string() })
        }
        other => anyhow::bail!("invalid Core Memory scope: {other}"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryTopicEntry {
    pub file_name: String,
    pub relative_path: String,
    pub name: String,
    pub description: String,
    pub memory_type: String,
    pub size_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryTopicFile {
    #[serde(flatten)]
    pub entry: CoreMemoryTopicEntry,
    pub content: String,
    pub file_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryTopicWriteInput {
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub expected_file_hash: Option<String>,
    pub name: String,
    pub description: String,
    #[serde(default = "default_memory_type")]
    pub memory_type: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryTopicSearchHit {
    #[serde(flatten)]
    pub entry: CoreMemoryTopicEntry,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreMemoryTopicPage {
    pub entries: Vec<CoreMemoryTopicEntry>,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone)]
struct TopicRecord {
    entry: CoreMemoryTopicEntry,
    path: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct CoreMemoryLayerSnapshot {
    pub content: String,
    pub file_hash: String,
    pub state: CoreMemoryMigrationState,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct CoreMemorySnapshot {
    pub agent_id: String,
    pub project_id: Option<String>,
    pub shared_global: bool,
    pub global: Option<CoreMemoryLayerSnapshot>,
    pub agent: Option<CoreMemoryLayerSnapshot>,
    pub project: Option<CoreMemoryLayerSnapshot>,
    pub fingerprint: String,
    pub captured_at: String,
}

impl CoreMemorySnapshot {
    pub(crate) fn capture(
        agent_id: &str,
        project_id: Option<&str>,
        shared_global: bool,
    ) -> Result<Self> {
        let global = if shared_global {
            snapshot_layer(load_index(&CoreMemoryScope::Global)?)
        } else {
            None
        };
        let agent = snapshot_layer(load_index(&CoreMemoryScope::Agent {
            id: agent_id.to_string(),
        })?);
        let project = project_id
            .map(|id| {
                load_index(&CoreMemoryScope::Project { id: id.to_string() }).map(snapshot_layer)
            })
            .transpose()?
            .flatten();
        let mut hasher = blake3::Hasher::new();
        hasher.update(agent_id.as_bytes());
        hasher.update(&[u8::from(shared_global)]);
        if let Some(project_id) = project_id {
            hasher.update(project_id.as_bytes());
        }
        for layer in [&global, &agent, &project].into_iter().flatten() {
            hasher.update(layer.file_hash.as_bytes());
        }
        Ok(Self {
            agent_id: agent_id.to_string(),
            project_id: project_id.map(str::to_string),
            shared_global,
            global,
            agent,
            project,
            fingerprint: hasher.finalize().to_hex()[..16].to_string(),
            captured_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    pub(crate) fn matches_context(
        &self,
        agent_id: &str,
        project_id: Option<&str>,
        shared_global: bool,
    ) -> bool {
        self.agent_id == agent_id
            && self.project_id.as_deref() == project_id
            && self.shared_global == shared_global
    }
}

pub(crate) fn session_snapshot(
    session_id: &str,
    agent_id: &str,
    project_id: Option<&str>,
    shared_global: bool,
) -> Result<CoreMemorySnapshot> {
    let key = session_snapshot_key(session_id)?;
    if let Some(snapshot) = SESSION_SNAPSHOTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(&key)
        .filter(|snapshot| snapshot.matches_context(agent_id, project_id, shared_global))
        .cloned()
    {
        return Ok(snapshot);
    }
    let snapshot = CoreMemorySnapshot::capture(agent_id, project_id, shared_global)?;
    let mut snapshots = SESSION_SNAPSHOTS
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    // Two turns can race on a cold session. Preserve whichever compatible
    // snapshot became authoritative first so both callers observe one prefix.
    if let Some(existing) = snapshots
        .get(&key)
        .filter(|existing| existing.matches_context(agent_id, project_id, shared_global))
    {
        return Ok(existing.clone());
    }
    snapshots.insert(key, snapshot.clone());
    Ok(snapshot)
}

pub fn invalidate_session_snapshot(session_id: &str) {
    let Ok(key) = session_snapshot_key(session_id) else {
        return;
    };
    SESSION_SNAPSHOTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .remove(&key);
}

/// Drop every frozen Core snapshot after an owner-level restore replaces
/// repository files out of band. Per-session invalidation remains the normal
/// path; full backup restore is the deliberate process-wide exception.
pub fn invalidate_all_session_snapshots() {
    SESSION_SNAPSHOTS
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clear();
}

fn session_snapshot_key(session_id: &str) -> Result<String> {
    let root = crate::paths::root_dir()?;
    Ok(format!("{}:{}", root.display(), session_id))
}

fn snapshot_layer(index: CoreMemoryIndex) -> Option<CoreMemoryLayerSnapshot> {
    let content = index.content?;
    Some(CoreMemoryLayerSnapshot {
        estimated_tokens: crate::system_prompt::conservative_core_token_estimate(&content)
            .min(u32::MAX as usize) as u32,
        content,
        file_hash: index.file_hash?,
        state: index.state,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct CoreMemoryMigrationManifest {
    version: u32,
    #[serde(default)]
    scopes: BTreeMap<String, CoreMemoryMigrationEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CoreMemoryMigrationEntry {
    canonical_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    legacy_path: Option<String>,
    synced_hash: String,
    snapshot_path: String,
    revision: u64,
    updated_at: String,
}

pub fn paths(scope: &CoreMemoryScope) -> Result<CoreMemoryPaths> {
    match scope {
        CoreMemoryScope::Global => {
            let root = crate::paths::root_dir()?;
            let dir = root.join("memory");
            Ok(CoreMemoryPaths {
                canonical: dir.join(CORE_INDEX_FILE),
                legacy: Some(root.join("memory.md")),
                dir,
            })
        }
        CoreMemoryScope::Agent { id } => {
            crate::paths::validate_agent_id(id)?;
            let agent_dir = crate::paths::agent_dir(id)?;
            let dir = agent_dir.join("memory");
            Ok(CoreMemoryPaths {
                canonical: dir.join(CORE_INDEX_FILE),
                legacy: Some(agent_dir.join("memory.md")),
                dir,
            })
        }
        CoreMemoryScope::Project { id } => {
            uuid::Uuid::parse_str(id).map_err(|_| anyhow::anyhow!("invalid project id"))?;
            let dir = crate::paths::project_dir(id)?.join("memory");
            Ok(CoreMemoryPaths {
                canonical: dir.join(CORE_INDEX_FILE),
                legacy: None,
                dir,
            })
        }
    }
}

/// Resolve, migrate and load one Core index. This function performs blocking
/// filesystem IO and must be called from the blocking pool in async paths.
pub fn load_index(scope: &CoreMemoryScope) -> Result<CoreMemoryIndex> {
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    resolve_locked(scope, &resolved)
}

pub fn load_stats(scope: &CoreMemoryScope) -> Result<CoreMemoryStats> {
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let index = resolve_locked(scope, &resolved)?;
    let content = index.content.as_deref().unwrap_or_default();
    let updated_path = if resolved.canonical.exists() {
        &resolved.canonical
    } else {
        resolved.legacy.as_ref().unwrap_or(&resolved.canonical)
    };
    let updated_at = fs::metadata(updated_path)
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .map(chrono::DateTime::<chrono::Utc>::from)
        .map(|value| value.to_rfc3339());
    Ok(CoreMemoryStats {
        index_bytes: content.len(),
        estimated_tokens: crate::system_prompt::conservative_core_token_estimate(content)
            .min(u32::MAX as usize) as u32,
        index_entry_count: content
            .lines()
            .filter(|line| {
                let line = line.trim_start();
                line.starts_with("- ") || line.starts_with("* ")
            })
            .count(),
        topic_count: list_topic_records_unlocked(scope, &resolved)?.len(),
        updated_at,
        state: index.state,
    })
}

/// Promote one owner-selected dynamic memory/claim into bounded Core Memory.
/// This never deletes or re-scopes the source record. Automatic callers must
/// create a review proposal instead; this function is the explicit owner/tool
/// action and still rejects credential-like material deterministically.
pub fn promote(input: CoreMemoryPromotionInput) -> Result<CoreMemoryPromotionResult> {
    let scope = parse_scope(&input.scope_type, input.scope_id.as_deref())?;
    let source_id = input.source_id.trim();
    if source_id.is_empty() {
        anyhow::bail!("sourceId is required");
    }
    let (content, memory_type, source_kind) = match input.source_kind {
        CoreMemoryPromotionSourceKind::Memory => {
            let id = source_id
                .parse::<i64>()
                .context("memory sourceId must be an integer")?;
            let backend = crate::get_memory_backend()
                .ok_or_else(|| anyhow::anyhow!("Memory backend not initialized"))?;
            let entry = backend
                .get(id)?
                .ok_or_else(|| anyhow::anyhow!("memory source not found"))?;
            (
                entry.content,
                entry.memory_type.as_str().to_string(),
                "memory",
            )
        }
        CoreMemoryPromotionSourceKind::Claim => {
            let detail = super::claims::get_claim(source_id)?
                .ok_or_else(|| anyhow::anyhow!("claim source not found"))?;
            if !super::claims::is_injectable_status(&super::claims::effective_status(
                &detail.claim.status,
                detail.claim.valid_until.as_deref(),
                &chrono::Utc::now().to_rfc3339(),
            )) {
                anyhow::bail!("only effective-active claims can be promoted to Core Memory");
            }
            (
                detail.claim.content,
                core_topic_type_for_claim(&detail.claim.claim_type).to_string(),
                "claim",
            )
        }
    };
    let content = content.trim();
    if content.is_empty() {
        anyhow::bail!("cannot promote empty content");
    }
    reject_secret_like_core_content(content)?;

    let current = load_index(&scope)?;
    let target_fact = canonical_fact(content);
    let already_present = current.content.as_deref().is_some_and(|index| {
        index
            .lines()
            .any(|line| canonical_fact(line) == target_fact)
    });
    let mut topic_file_name = None;
    let index = if already_present {
        current
    } else if let Some(topic_name) = input
        .topic_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
    {
        let topic = write_topic(
            &scope,
            CoreMemoryTopicWriteInput {
                file_name: None,
                expected_file_hash: None,
                name: topic_name.to_string(),
                description: one_line(content, 240),
                memory_type,
                content: content.to_string(),
            },
        )?;
        topic_file_name = Some(topic.entry.file_name);
        load_index(&scope)?
    } else {
        let summary = one_line(content, 480);
        let next = append_core_fact(current.content.as_deref(), &summary);
        save_index(&scope, &next, current.file_hash.as_deref())?
    };
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "memory:promotion_completed",
            serde_json::json!({
                "sourceKind": source_kind,
                "sourceIdHash": blake3::hash(source_id.as_bytes()).to_hex()[..16].to_string(),
                "scopeType": scope.scope_type(),
                "scopeId": scope.scope_id(),
                "topic": topic_file_name.is_some(),
                "alreadyPresent": already_present,
            }),
        );
    }
    Ok(CoreMemoryPromotionResult {
        source_kind: source_kind.to_string(),
        source_id: source_id.to_string(),
        scope_key: scope.key(),
        index_file_hash: index.file_hash,
        topic_file_name,
        already_present,
    })
}

fn append_core_fact(current: Option<&str>, fact: &str) -> String {
    let current = current.unwrap_or_default().trim_end();
    if current.is_empty() {
        format!("# Core Memory\n\n- {fact}\n")
    } else if let Some(topics_offset) = current.find("\n## Topics") {
        let stable_facts = current[..topics_offset].trim_end();
        let topics = current[topics_offset..].trim_start();
        format!("{stable_facts}\n- {fact}\n\n{topics}\n")
    } else {
        format!("{current}\n- {fact}\n")
    }
}

fn canonical_fact(value: &str) -> String {
    value
        .trim_start_matches(|ch: char| matches!(ch, '-' | '*' | '+') || ch.is_whitespace())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn core_topic_type_for_claim(claim_type: &str) -> &'static str {
    match claim_type {
        "project_fact" | "task_pattern" => "project",
        "reference" => "reference",
        "preference" | "standing_rule" => "feedback",
        _ => "user",
    }
}

fn reject_secret_like_core_content(content: &str) -> Result<()> {
    static SECRET_PATTERNS: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
        [
            r"(?i)-----BEGIN [A-Z ]*PRIVATE KEY-----",
            r"(?i)\bauthorization\s*:\s*bearer\s+\S{8,}",
            r#"(?i)\b(api[_ -]?key|access[_ -]?token|refresh[_ -]?token|password|secret)\s*[:=]\s*[\"']?\S{8,}"#,
            r"\bsk-[A-Za-z0-9_-]{16,}\b",
        ]
        .into_iter()
        .map(|pattern| regex::Regex::new(pattern).expect("valid Core Memory secret regex"))
        .collect()
    });
    if SECRET_PATTERNS
        .iter()
        .any(|pattern| pattern.is_match(content))
    {
        anyhow::bail!("credential-like content cannot be promoted to Core Memory");
    }
    Ok(())
}

pub fn save_index(
    scope: &CoreMemoryScope,
    content: &str,
    expected_hash: Option<&str>,
) -> Result<CoreMemoryIndex> {
    validate_index_content(content)?;
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current = resolve_locked(scope, &resolved)?;
    if current.state == CoreMemoryMigrationState::Conflict {
        anyhow::bail!(
            "core memory migration conflict: resolve the lowercase/uppercase files before saving"
        );
    }
    match (current.file_hash.as_deref(), expected_hash) {
        (Some(actual), Some(expected)) if actual != expected => {
            anyhow::bail!("core memory stale-write conflict: read it again before saving")
        }
        (Some(_), None) => anyhow::bail!("expectedHash is required when updating Core Memory"),
        (None, Some(_)) => {
            anyhow::bail!("core memory stale-write conflict: the index no longer exists")
        }
        _ => {}
    }
    synchronize_locked(
        scope,
        &resolved,
        content.as_bytes(),
        SyncKind::CanonicalWrite,
    )?;
    let index = resolve_locked(scope, &resolved)?;
    drop(_guard);
    emit_core_changed(scope, "save_index");
    Ok(index)
}

/// Compatibility owner write for existing APIs that predate stale-write
/// hashes. New V2 APIs should use [`save_index`]. Conflict state still blocks
/// the write; otherwise this preserves the existing last-writer-wins surface.
pub fn save_index_owner(scope: &CoreMemoryScope, content: &str) -> Result<CoreMemoryIndex> {
    validate_index_content(content)?;
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current = resolve_locked(scope, &resolved)?;
    if current.state == CoreMemoryMigrationState::Conflict {
        anyhow::bail!(
            "core memory migration conflict: resolve the lowercase/uppercase files before saving"
        );
    }
    synchronize_locked(
        scope,
        &resolved,
        content.as_bytes(),
        SyncKind::CanonicalWrite,
    )?;
    let index = resolve_locked(scope, &resolved)?;
    drop(_guard);
    emit_core_changed(scope, "save_index");
    Ok(index)
}

/// Return both sides of an unresolved lowercase/uppercase migration conflict.
/// This is owner-plane data: agent tools never receive either full document.
pub fn load_conflict(scope: &CoreMemoryScope) -> Result<Option<CoreMemoryConflict>> {
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current = resolve_locked(scope, &resolved)?;
    if current.state != CoreMemoryMigrationState::Conflict {
        return Ok(None);
    }
    load_conflict_unlocked(scope, &resolved).map(Some)
}

/// Resolve a two-sided migration conflict with an explicit owner choice. Both
/// source hashes are required so a concurrently edited file can never be
/// overwritten by a stale merge dialog.
pub fn resolve_conflict(
    scope: &CoreMemoryScope,
    resolution: CoreMemoryConflictResolution,
) -> Result<CoreMemoryIndex> {
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current = resolve_locked(scope, &resolved)?;
    if current.state != CoreMemoryMigrationState::Conflict {
        anyhow::bail!("Core Memory conflict was already resolved; reload before saving");
    }
    let conflict = load_conflict_unlocked(scope, &resolved)?;
    if conflict.canonical_hash != resolution.expected_canonical_hash
        || conflict.legacy_hash != resolution.expected_legacy_hash
    {
        anyhow::bail!("Core Memory conflict changed on disk; reload before resolving");
    }
    let selected = match resolution.choice {
        CoreMemoryConflictChoice::Canonical => conflict.canonical_content,
        CoreMemoryConflictChoice::Legacy => conflict.legacy_content,
        CoreMemoryConflictChoice::Merged => resolution
            .merged_content
            .ok_or_else(|| anyhow::anyhow!("mergedContent is required for merged resolution"))?,
    };
    validate_index_content(&selected)?;
    synchronize_locked(
        scope,
        &resolved,
        selected.as_bytes(),
        SyncKind::CanonicalWrite,
    )?;
    let index = resolve_locked(scope, &resolved)?;
    drop(_guard);
    emit_core_changed(scope, "resolve_conflict");
    Ok(index)
}

fn load_conflict_unlocked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
) -> Result<CoreMemoryConflict> {
    let legacy_path = resolved
        .legacy
        .clone()
        .ok_or_else(|| anyhow::anyhow!("Project Core Memory has no legacy conflict surface"))?;
    let canonical = read_regular_optional(&resolved.canonical)?
        .ok_or_else(|| anyhow::anyhow!("canonical Core Memory file is missing"))?;
    let legacy = read_regular_optional(&legacy_path)?
        .ok_or_else(|| anyhow::anyhow!("legacy Core Memory file is missing"))?;
    let manifest = load_manifest()?;
    let entry = manifest.scopes.get(&scope.key());
    let last_synced = entry
        .map(|entry| read_regular_optional(Path::new(&entry.snapshot_path)))
        .transpose()?
        .flatten();
    Ok(CoreMemoryConflict {
        canonical_hash: content_hash(&canonical),
        canonical_content: String::from_utf8(canonical)
            .context("canonical Core MEMORY.md must be UTF-8")?,
        legacy_hash: content_hash(&legacy),
        legacy_content: String::from_utf8(legacy).context("legacy memory.md must be UTF-8")?,
        last_synced_hash: last_synced.as_deref().map(content_hash),
        last_synced_content: last_synced
            .map(|bytes| String::from_utf8(bytes).context("Core Memory snapshot must be UTF-8"))
            .transpose()?,
        canonical_path: resolved.canonical.clone(),
        legacy_path,
    })
}

pub fn list_topics(scope: &CoreMemoryScope) -> Result<Vec<CoreMemoryTopicEntry>> {
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    Ok(list_topic_records_unlocked(scope, &resolved)?
        .into_iter()
        .map(|record| record.entry)
        .collect())
}

pub fn list_topics_page(
    scope: &CoreMemoryScope,
    offset: usize,
    limit: usize,
) -> Result<CoreMemoryTopicPage> {
    let entries = list_topics(scope)?;
    let total = entries.len();
    let offset = offset.min(total);
    let limit = limit.clamp(1, 100);
    Ok(CoreMemoryTopicPage {
        entries: entries.into_iter().skip(offset).take(limit).collect(),
        total,
        offset,
        limit,
    })
}

pub fn read_topic(scope: &CoreMemoryScope, file_name: &str) -> Result<CoreMemoryTopicFile> {
    validate_topic_file_name(file_name)?;
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    read_topic_unlocked(scope, &resolved, file_name)
}

pub fn write_topic(
    scope: &CoreMemoryScope,
    input: CoreMemoryTopicWriteInput,
) -> Result<CoreMemoryTopicFile> {
    let name = input.name.trim();
    let description = one_line(&input.description, 500);
    if name.is_empty() || name.chars().count() > 120 {
        anyhow::bail!("Core Memory topic name must contain 1-120 characters");
    }
    if description.is_empty() {
        anyhow::bail!("Core Memory topic description cannot be empty");
    }
    let memory_type = input.memory_type.trim().to_ascii_lowercase();
    if !MEMORY_TYPES.contains(&memory_type.as_str()) {
        anyhow::bail!("invalid Core Memory topic type: {}", input.memory_type);
    }
    let requested_file_name = input
        .file_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let mut file_name = requested_file_name
        .clone()
        .unwrap_or_else(|| generated_file_name(name, &memory_type));
    validate_topic_file_name(&file_name)?;

    let body = strip_frontmatter(&input.content).trim().to_string();
    let document = format!(
        "---\nname: {}\ndescription: {}\nmetadata:\n  node_type: memory\n  type: {}\n---\n\n{}\n",
        yaml_scalar(name),
        yaml_scalar(&description),
        yaml_scalar(&memory_type),
        body
    );
    if document.len() > CORE_TOPIC_MAX_BYTES {
        anyhow::bail!("Core Memory topic exceeds {} bytes", CORE_TOPIC_MAX_BYTES);
    }

    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    ensure_real_dir(&resolved.dir.join(TOPICS_DIR))?;
    let _guard = acquire_repository_lock()?;
    let current_index = resolve_locked(scope, &resolved)?;
    if current_index.state == CoreMemoryMigrationState::Conflict {
        anyhow::bail!(
            "core memory migration conflict: resolve the lowercase/uppercase files before saving"
        );
    }
    let existing = find_topic_record_unlocked(scope, &resolved, &file_name)?;
    if requested_file_name.is_none() {
        file_name = unique_topic_file_name_unlocked(scope, &resolved, &file_name)?;
    }
    let existing = if requested_file_name.is_some() {
        existing
    } else {
        None
    };
    validate_update_precondition(
        existing.as_ref().map(|record| record.file_hash.as_str()),
        input.expected_file_hash.as_deref(),
    )?;
    let mut records = list_topic_records_unlocked(scope, &resolved)?;
    if existing.is_none() && records.len() >= CORE_MAX_TOPIC_FILES {
        anyhow::bail!(
            "Core Memory is limited to {} topic files",
            CORE_MAX_TOPIC_FILES
        );
    }
    let target = existing.map_or_else(
        || resolved.dir.join(TOPICS_DIR).join(&file_name),
        |record| record.path,
    );
    let relative_path = target
        .strip_prefix(&resolved.dir)
        .unwrap_or(&target)
        .to_string_lossy()
        .replace('\\', "/");
    records.retain(|record| record.entry.file_name != file_name);
    records.push(TopicRecord {
        entry: parse_topic_entry(&file_name, &relative_path, &document, document.len()),
        path: target.clone(),
    });
    sort_topic_records(&mut records)?;
    let prospective_index = render_topic_index(scope, current_index.content.as_deref(), &records);
    validate_index_content(&prospective_index)?;

    let previous = read_regular_optional_topic(&target)?;
    crate::platform::write_atomic(&target, document.as_bytes())?;
    if let Err(error) = synchronize_locked(
        scope,
        &resolved,
        prospective_index.as_bytes(),
        SyncKind::CanonicalWrite,
    ) {
        match previous {
            Some(bytes) => {
                let _ = crate::platform::write_atomic(&target, &bytes);
            }
            None => {
                let _ = fs::remove_file(&target);
            }
        }
        return Err(error);
    }
    let topic = read_topic_unlocked(scope, &resolved, &file_name)?;
    drop(_guard);
    emit_core_changed(scope, "write_topic");
    Ok(topic)
}

pub fn delete_topic(
    scope: &CoreMemoryScope,
    file_name: &str,
    expected_file_hash: Option<&str>,
) -> Result<bool> {
    validate_topic_file_name(file_name)?;
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current_index = resolve_locked(scope, &resolved)?;
    if current_index.state == CoreMemoryMigrationState::Conflict {
        anyhow::bail!(
            "core memory migration conflict: resolve the lowercase/uppercase files before saving"
        );
    }
    let Some(record) = find_topic_record_unlocked(scope, &resolved, file_name)? else {
        if expected_file_hash.is_some() {
            anyhow::bail!("Core Memory stale-write conflict: topic was already deleted");
        }
        return Ok(false);
    };
    validate_delete_precondition(&record.file_hash, expected_file_hash)?;
    let deleted_path = record.path;
    let deleted_bytes = fs::read(&deleted_path)?;
    fs::remove_file(&deleted_path)?;
    if let Err(error) = rebuild_index_unlocked(scope, &resolved) {
        if let Err(restore_error) = crate::platform::write_atomic(&deleted_path, &deleted_bytes) {
            app_warn!(
                "memory",
                "core_repository",
                "Failed to restore Core Memory topic {} after index rebuild error: {}",
                deleted_path.display(),
                restore_error
            );
        }
        return Err(error);
    }
    drop(_guard);
    emit_core_changed(scope, "delete_topic");
    Ok(true)
}

pub fn search_topics(
    scope: &CoreMemoryScope,
    query: &str,
    limit: usize,
) -> Result<Vec<CoreMemoryTopicSearchHit>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let mut hits = Vec::new();
    for record in list_topic_records_unlocked(scope, &resolved)? {
        let file = read_topic_record(&record)?;
        let haystack = format!(
            "{}\n{}\n{}",
            record.entry.name, record.entry.description, file.content
        );
        if haystack.to_lowercase().contains(&query) {
            let preview = one_line(
                haystack
                    .lines()
                    .find(|line| line.to_lowercase().contains(&query))
                    .unwrap_or(&record.entry.description),
                240,
            );
            hits.push(CoreMemoryTopicSearchHit {
                entry: record.entry,
                preview,
            });
        }
        if hits.len() >= limit.clamp(1, 50) {
            break;
        }
    }
    Ok(hits)
}

pub fn rebuild_topic_index(scope: &CoreMemoryScope) -> Result<String> {
    let resolved = paths(scope)?;
    ensure_repository_dirs(&resolved)?;
    let _guard = acquire_repository_lock()?;
    let current = resolve_locked(scope, &resolved)?;
    if current.state == CoreMemoryMigrationState::Conflict {
        anyhow::bail!(
            "core memory migration conflict: resolve the lowercase/uppercase files before saving"
        );
    }
    let index = rebuild_index_unlocked(scope, &resolved)?;
    drop(_guard);
    emit_core_changed(scope, "rebuild_index");
    Ok(index)
}

fn rebuild_index_unlocked(scope: &CoreMemoryScope, resolved: &CoreMemoryPaths) -> Result<String> {
    let records = list_topic_records_unlocked(scope, resolved)?;
    let current = resolve_locked(scope, resolved)?;
    let index = render_topic_index(scope, current.content.as_deref(), &records);
    validate_index_content(&index)?;
    synchronize_locked(scope, resolved, index.as_bytes(), SyncKind::CanonicalWrite)?;
    Ok(index)
}

fn list_topic_records_unlocked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
) -> Result<Vec<TopicRecord>> {
    let mut records = Vec::new();
    collect_topic_records(&resolved.dir.join(TOPICS_DIR), TOPICS_DIR, &mut records)?;
    // Project Auto Memory historically stored topics beside MEMORY.md. Keep
    // those files in place and readable; all newly created topics use topics/.
    if matches!(scope, CoreMemoryScope::Project { .. }) {
        collect_topic_records(&resolved.dir, "", &mut records)?;
    }
    sort_topic_records(&mut records)?;
    records.truncate(CORE_MAX_TOPIC_FILES);
    Ok(records)
}

fn sort_topic_records(records: &mut [TopicRecord]) -> Result<()> {
    let mut names = std::collections::BTreeSet::new();
    for record in records.iter() {
        if !names.insert(record.entry.file_name.clone()) {
            anyhow::bail!(
                "Core Memory topic conflict: {} exists in both legacy and topics directories",
                record.entry.file_name
            );
        }
    }
    records.sort_by(|left, right| {
        memory_type_order(&left.entry.memory_type)
            .cmp(&memory_type_order(&right.entry.memory_type))
            .then_with(|| left.entry.file_name.cmp(&right.entry.file_name))
    });
    Ok(())
}

fn collect_topic_records(
    dir: &Path,
    relative_dir: &str,
    records: &mut Vec<TopicRecord>,
) -> Result<()> {
    let Some(metadata) = symlink_metadata_optional(dir)? else {
        return Ok(());
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        anyhow::bail!("Core Memory topics directory must be a real directory");
    }
    for item in fs::read_dir(dir).with_context(|| format!("list {}", dir.display()))? {
        let item = item?;
        let file_type = item.file_type()?;
        if !file_type.is_file() || file_type.is_symlink() {
            continue;
        }
        let file_name = item.file_name().to_string_lossy().to_string();
        if file_name == CORE_INDEX_FILE || !is_valid_topic_file_name(&file_name) {
            continue;
        }
        let metadata = item.metadata()?;
        if metadata.len() as usize > CORE_TOPIC_MAX_BYTES {
            continue;
        }
        let bytes = fs::read(item.path())?;
        let content = std::str::from_utf8(&bytes).context("Core Memory topic must be UTF-8")?;
        let relative_path = if relative_dir.is_empty() {
            file_name.clone()
        } else {
            format!("{relative_dir}/{file_name}")
        };
        records.push(TopicRecord {
            entry: parse_topic_entry(&file_name, &relative_path, content, bytes.len()),
            path: item.path(),
        });
    }
    Ok(())
}

struct ExistingTopic {
    path: PathBuf,
    file_hash: String,
}

fn find_topic_record_unlocked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    file_name: &str,
) -> Result<Option<ExistingTopic>> {
    let canonical = resolved.dir.join(TOPICS_DIR).join(file_name);
    let canonical_hash = existing_regular_file_hash(&canonical)?;
    let legacy =
        matches!(scope, CoreMemoryScope::Project { .. }).then(|| resolved.dir.join(file_name));
    let legacy_hash = legacy
        .as_deref()
        .map(existing_regular_file_hash)
        .transpose()?
        .flatten();
    match (canonical_hash, legacy_hash, legacy) {
        (Some(_), Some(_), _) => anyhow::bail!(
            "Core Memory topic conflict: {file_name} exists in both legacy and topics directories"
        ),
        (Some(file_hash), _, _) => Ok(Some(ExistingTopic {
            path: canonical,
            file_hash,
        })),
        (_, Some(file_hash), Some(path)) => Ok(Some(ExistingTopic { path, file_hash })),
        _ => Ok(None),
    }
}

fn read_topic_unlocked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    file_name: &str,
) -> Result<CoreMemoryTopicFile> {
    let existing = find_topic_record_unlocked(scope, resolved, file_name)?
        .ok_or_else(|| anyhow::anyhow!("Core Memory topic not found"))?;
    let bytes = fs::read(&existing.path)?;
    if bytes.len() > CORE_TOPIC_MAX_BYTES {
        anyhow::bail!("Core Memory topic exceeds {} bytes", CORE_TOPIC_MAX_BYTES);
    }
    let content = String::from_utf8(bytes).context("Core Memory topic must be UTF-8")?;
    let relative_path = existing
        .path
        .strip_prefix(&resolved.dir)
        .unwrap_or(&existing.path)
        .to_string_lossy()
        .replace('\\', "/");
    Ok(CoreMemoryTopicFile {
        entry: parse_topic_entry(file_name, &relative_path, &content, content.len()),
        content: strip_frontmatter(&content).trim().to_string(),
        file_hash: existing.file_hash,
    })
}

fn read_topic_record(record: &TopicRecord) -> Result<CoreMemoryTopicFile> {
    let bytes = fs::read(&record.path)?;
    let content = String::from_utf8(bytes).context("Core Memory topic must be UTF-8")?;
    Ok(CoreMemoryTopicFile {
        entry: record.entry.clone(),
        content: strip_frontmatter(&content).trim().to_string(),
        file_hash: content_hash(content.as_bytes()),
    })
}

fn render_topic_index(
    scope: &CoreMemoryScope,
    current_index: Option<&str>,
    records: &[TopicRecord],
) -> String {
    let preserved = current_index
        .map(str::trim)
        .filter(|content| !content.is_empty())
        .and_then(|content| {
            if matches!(scope, CoreMemoryScope::Project { .. })
                && content.starts_with("# Memory Index")
            {
                None
            } else {
                Some(
                    content
                        .split("\n## Topics")
                        .next()
                        .unwrap_or(content)
                        .trim(),
                )
            }
        })
        .filter(|content| !content.is_empty())
        .unwrap_or("# Core Memory");
    let mut index = format!("{preserved}\n\n## Topics\n");
    for memory_type in MEMORY_TYPES {
        let matching = records
            .iter()
            .filter(|record| record.entry.memory_type == memory_type)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        index.push_str(&format!("\n### {}\n", title_case(memory_type)));
        for record in matching {
            index.push_str(&format!(
                "- [{}]({}) — {}\n",
                record.entry.name, record.entry.relative_path, record.entry.description
            ));
        }
    }
    index
}

fn resolve_locked(scope: &CoreMemoryScope, resolved: &CoreMemoryPaths) -> Result<CoreMemoryIndex> {
    let canonical = read_regular_optional(&resolved.canonical)?;
    let legacy = resolved
        .legacy
        .as_deref()
        .map(read_regular_optional)
        .transpose()?
        .flatten();
    let key = scope.key();
    let mut manifest = load_manifest()?;
    let entry = manifest.scopes.get(&key).cloned();

    match (canonical, legacy, entry) {
        (None, None, _) => Ok(index_result(
            resolved,
            None,
            CoreMemoryMigrationState::Empty,
        )),
        (Some(canonical), None, entry) if resolved.legacy.is_none() => {
            let hash = content_hash(&canonical);
            if entry.as_ref().is_none_or(|entry| entry.synced_hash != hash) {
                record_sync_best_effort(scope, resolved, &canonical, &mut manifest);
            }
            Ok(index_result(
                resolved,
                Some(canonical),
                CoreMemoryMigrationState::Canonical,
            ))
        }
        (None, Some(legacy), _) => {
            synchronize_locked(scope, resolved, &legacy, SyncKind::LegacyMigration)?;
            Ok(index_result(
                resolved,
                Some(legacy),
                CoreMemoryMigrationState::MigratedLegacy,
            ))
        }
        (Some(canonical), None, Some(entry)) => {
            if resolved.legacy.is_some() {
                synchronize_locked(scope, resolved, &canonical, SyncKind::CanonicalRecovery)?;
                Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::RecoveredCanonicalChange,
                ))
            } else {
                let _ = entry;
                Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::Canonical,
                ))
            }
        }
        (Some(canonical), None, None) => {
            synchronize_locked(scope, resolved, &canonical, SyncKind::CanonicalWrite)?;
            Ok(index_result(
                resolved,
                Some(canonical),
                CoreMemoryMigrationState::Mirrored,
            ))
        }
        (Some(canonical), Some(legacy), None) => {
            if content_hash(&canonical) == content_hash(&legacy) {
                record_sync_best_effort(scope, resolved, &canonical, &mut manifest);
                Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::Mirrored,
                ))
            } else {
                // There is no trusted common ancestor. Exposing either side
                // would silently choose a winner before the owner resolves
                // the conflict, so fail closed for prompt injection.
                Ok(index_result(
                    resolved,
                    None,
                    CoreMemoryMigrationState::Conflict,
                ))
            }
        }
        (Some(canonical), Some(legacy), Some(entry)) => {
            let canonical_hash = content_hash(&canonical);
            let legacy_hash = content_hash(&legacy);
            if canonical_hash == legacy_hash {
                if canonical_hash != entry.synced_hash {
                    record_sync_best_effort(scope, resolved, &canonical, &mut manifest);
                }
                return Ok(index_result(
                    resolved,
                    Some(canonical),
                    CoreMemoryMigrationState::Mirrored,
                ));
            }
            let canonical_changed = canonical_hash != entry.synced_hash;
            let legacy_changed = legacy_hash != entry.synced_hash;
            match (canonical_changed, legacy_changed) {
                (true, false) => {
                    synchronize_locked(scope, resolved, &canonical, SyncKind::CanonicalRecovery)?;
                    Ok(index_result(
                        resolved,
                        Some(canonical),
                        CoreMemoryMigrationState::RecoveredCanonicalChange,
                    ))
                }
                (false, true) => {
                    synchronize_locked(scope, resolved, &legacy, SyncKind::LegacyRecovery)?;
                    Ok(index_result(
                        resolved,
                        Some(legacy),
                        CoreMemoryMigrationState::RecoveredLegacyChange,
                    ))
                }
                _ => {
                    // The last synchronized snapshot is the only prompt-safe
                    // content while both writable surfaces disagree. If that
                    // snapshot is unavailable, inject nothing rather than
                    // picking canonical or legacy implicitly.
                    let snapshot = read_regular_optional(Path::new(&entry.snapshot_path))?;
                    Ok(index_result(
                        resolved,
                        snapshot,
                        CoreMemoryMigrationState::Conflict,
                    ))
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
enum SyncKind {
    LegacyMigration,
    LegacyRecovery,
    CanonicalRecovery,
    CanonicalWrite,
}

fn synchronize_locked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    bytes: &[u8],
    kind: SyncKind,
) -> Result<()> {
    validate_index_bytes(bytes)?;
    if matches!(kind, SyncKind::LegacyMigration) {
        if let Some(legacy) = resolved.legacy.as_deref() {
            backup_legacy_once(scope, legacy, bytes)?;
        }
    }
    crate::platform::write_atomic(&resolved.canonical, bytes)?;
    verify_hash(&resolved.canonical, bytes)?;
    if let Some(legacy) = resolved.legacy.as_deref() {
        let mirror_result: Result<()> = (|| {
            crate::platform::write_atomic(legacy, bytes)?;
            verify_hash(legacy, bytes)
        })();
        if let Err(error) = mirror_result {
            warn_compatibility_sync(scope, "legacy_mirror", &error);
            // The canonical uppercase file is the commit point. Leaving the
            // old manifest unchanged lets the next resolver identify and
            // repair this as a canonical-only change.
            return Ok(());
        }
    }
    let mut manifest = load_manifest()?;
    if let Err(error) = record_sync_locked(scope, resolved, bytes, &mut manifest) {
        warn_compatibility_sync(scope, "manifest", &error);
    }
    Ok(())
}

fn record_sync_best_effort(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    bytes: &[u8],
    manifest: &mut CoreMemoryMigrationManifest,
) {
    if let Err(error) = record_sync_locked(scope, resolved, bytes, manifest) {
        warn_compatibility_sync(scope, "manifest", &error);
    }
}

fn warn_compatibility_sync(scope: &CoreMemoryScope, stage: &str, error: &anyhow::Error) {
    app_warn!(
        "memory",
        "core_repository",
        "Core Memory canonical write for {} succeeded but {} sync is stale: {}",
        scope.key(),
        stage,
        error
    );
}

fn record_sync_locked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    bytes: &[u8],
    manifest: &mut CoreMemoryMigrationManifest,
) -> Result<()> {
    let key = scope.key();
    let snapshot = snapshot_path(&key)?;
    if let Some(parent) = snapshot.parent() {
        ensure_real_dir(parent)?;
    }
    crate::platform::write_atomic(&snapshot, bytes)?;
    let previous_revision = manifest.scopes.get(&key).map_or(0, |entry| entry.revision);
    manifest.version = MANIFEST_VERSION;
    manifest.scopes.insert(
        key,
        CoreMemoryMigrationEntry {
            canonical_path: resolved.canonical.display().to_string(),
            legacy_path: resolved
                .legacy
                .as_ref()
                .map(|path| path.display().to_string()),
            synced_hash: content_hash(bytes),
            snapshot_path: snapshot.display().to_string(),
            revision: previous_revision.saturating_add(1),
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    );
    save_manifest(manifest)
}

fn index_result(
    resolved: &CoreMemoryPaths,
    bytes: Option<Vec<u8>>,
    state: CoreMemoryMigrationState,
) -> CoreMemoryIndex {
    let (content, file_hash) = bytes.map_or((None, None), |bytes| {
        let hash = content_hash(&bytes);
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let content = (!text.trim().is_empty()).then_some(text);
        (content, Some(hash))
    });
    CoreMemoryIndex {
        content,
        file_hash,
        state,
        canonical_path: resolved.canonical.clone(),
        legacy_path: resolved.legacy.clone(),
    }
}

fn default_memory_type() -> String {
    "project".to_string()
}

fn is_valid_topic_file_name(file_name: &str) -> bool {
    file_name.ends_with(".md")
        && file_name != CORE_INDEX_FILE
        && file_name.len() <= 128
        && file_name.trim_end_matches(".md").chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

fn validate_topic_file_name(file_name: &str) -> Result<()> {
    if !is_valid_topic_file_name(file_name) {
        anyhow::bail!("invalid Core Memory topic file name");
    }
    Ok(())
}

fn validate_update_precondition(
    existing_hash: Option<&str>,
    expected_file_hash: Option<&str>,
) -> Result<()> {
    match (existing_hash, expected_file_hash) {
        (Some(current), Some(expected)) if current != expected => anyhow::bail!(
            "Core Memory stale-write conflict: topic changed on disk; read it again before saving"
        ),
        (Some(_), None) => anyhow::bail!(
            "expectedFileHash is required when updating an existing Core Memory topic"
        ),
        (None, Some(_)) => anyhow::bail!(
            "Core Memory stale-write conflict: topic was deleted; read the topic list again"
        ),
        _ => Ok(()),
    }
}

fn validate_delete_precondition(
    current_hash: &str,
    expected_file_hash: Option<&str>,
) -> Result<()> {
    let Some(expected) = expected_file_hash else {
        anyhow::bail!("expectedFileHash is required when deleting an existing Core Memory topic");
    };
    if current_hash != expected {
        anyhow::bail!(
            "Core Memory stale-write conflict: topic changed on disk; read it again before deleting"
        );
    }
    Ok(())
}

fn existing_regular_file_hash(path: &Path) -> Result<Option<String>> {
    let Some(metadata) = symlink_metadata_optional(path)? else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        anyhow::bail!("Core Memory topic path is not a regular file");
    }
    if metadata.len() as usize > CORE_TOPIC_MAX_BYTES {
        anyhow::bail!("Core Memory topic exceeds {} bytes", CORE_TOPIC_MAX_BYTES);
    }
    Ok(Some(content_hash(&fs::read(path)?)))
}

fn read_regular_optional_topic(path: &Path) -> Result<Option<Vec<u8>>> {
    let Some(metadata) = symlink_metadata_optional(path)? else {
        return Ok(None);
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        anyhow::bail!("Core Memory topic path is not a regular file");
    }
    if metadata.len() as usize > CORE_TOPIC_MAX_BYTES {
        anyhow::bail!("Core Memory topic exceeds {} bytes", CORE_TOPIC_MAX_BYTES);
    }
    Ok(Some(fs::read(path)?))
}

fn symlink_metadata_optional(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn parse_topic_entry(
    file_name: &str,
    relative_path: &str,
    content: &str,
    size_bytes: usize,
) -> CoreMemoryTopicEntry {
    let fields = parse_frontmatter(content);
    CoreMemoryTopicEntry {
        file_name: file_name.to_string(),
        relative_path: relative_path.to_string(),
        name: fields
            .get("name")
            .cloned()
            .unwrap_or_else(|| file_name.trim_end_matches(".md").to_string()),
        description: fields.get("description").cloned().unwrap_or_default(),
        memory_type: fields
            .get("type")
            .cloned()
            .filter(|value| MEMORY_TYPES.contains(&value.as_str()))
            .unwrap_or_else(default_memory_type),
        size_bytes,
    }
}

fn parse_frontmatter(content: &str) -> std::collections::HashMap<String, String> {
    let mut output = std::collections::HashMap::new();
    let mut lines = content.lines();
    if lines.next() != Some("---") {
        return output;
    }
    for line in lines.take_while(|line| *line != "---") {
        let Some((key, value)) = line.trim().split_once(':') else {
            continue;
        };
        let key = key.trim();
        if matches!(key, "name" | "description" | "type") {
            output.insert(key.to_string(), unquote(value.trim()));
        }
    }
    output
}

fn strip_frontmatter(content: &str) -> &str {
    if !content.starts_with("---\n") {
        return content;
    }
    content
        .get(4..)
        .and_then(|rest| rest.find("\n---\n").map(|end| &rest[end + 5..]))
        .unwrap_or(content)
}

fn generated_file_name(name: &str, memory_type: &str) -> String {
    let slug = name
        .chars()
        .filter_map(|character| {
            if character.is_ascii_alphanumeric() {
                Some(character.to_ascii_lowercase())
            } else if matches!(character, ' ' | '-' | '_') {
                Some('_')
            } else {
                None
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .chars()
        .take(72)
        .collect::<String>();
    let slug = if slug.is_empty() {
        uuid::Uuid::new_v4().simple().to_string()[..12].to_string()
    } else {
        slug
    };
    format!("{memory_type}_{slug}.md")
}

fn unique_topic_file_name_unlocked(
    scope: &CoreMemoryScope,
    resolved: &CoreMemoryPaths,
    preferred: &str,
) -> Result<String> {
    if find_topic_record_unlocked(scope, resolved, preferred)?.is_none() {
        return Ok(preferred.to_string());
    }
    let stem = preferred.trim_end_matches(".md");
    for suffix in 2..=99 {
        let candidate = format!("{stem}_{suffix}.md");
        if find_topic_record_unlocked(scope, resolved, &candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Ok(format!(
        "{}_{}.md",
        stem,
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ))
}

fn yaml_scalar(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string())
}

fn unquote(value: &str) -> String {
    serde_json::from_str::<String>(value).unwrap_or_else(|_| value.trim_matches('"').to_string())
}

fn one_line(value: &str, max_chars: usize) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max_chars)
        .collect()
}

fn memory_type_order(value: &str) -> usize {
    MEMORY_TYPES
        .iter()
        .position(|item| *item == value)
        .unwrap_or(99)
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}

fn validate_index_content(content: &str) -> Result<()> {
    validate_index_bytes(content.as_bytes())
}

fn validate_index_bytes(bytes: &[u8]) -> Result<()> {
    if bytes.len() > CORE_INDEX_MAX_BYTES {
        anyhow::bail!("Core MEMORY.md exceeds {} bytes", CORE_INDEX_MAX_BYTES);
    }
    let content = std::str::from_utf8(bytes).context("Core MEMORY.md must be UTF-8")?;
    if content.lines().count() > CORE_INDEX_MAX_LINES {
        anyhow::bail!("Core MEMORY.md exceeds {} lines", CORE_INDEX_MAX_LINES);
    }
    Ok(())
}

fn read_regular_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        anyhow::bail!("Core Memory path is not a regular file: {}", path.display());
    }
    if metadata.len() as usize > CORE_INDEX_MAX_BYTES {
        anyhow::bail!("Core MEMORY.md exceeds {} bytes", CORE_INDEX_MAX_BYTES);
    }
    let mut file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((CORE_INDEX_MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    validate_index_bytes(&bytes)?;
    Ok(Some(bytes))
}

fn verify_hash(path: &Path, expected: &[u8]) -> Result<()> {
    let actual = fs::read(path)?;
    if content_hash(&actual) != content_hash(expected) {
        anyhow::bail!(
            "Core Memory migration verification failed for {}",
            path.display()
        );
    }
    Ok(())
}

fn ensure_repository_dirs(resolved: &CoreMemoryPaths) -> Result<()> {
    ensure_real_dir(&resolved.dir)?;
    ensure_real_dir(&migration_dir()?)?;
    ensure_real_dir(&snapshot_dir()?)
}

fn ensure_real_dir(path: &Path) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
            anyhow::bail!(
                "Core Memory directory must be a real directory: {}",
                path.display()
            );
        }
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Core Memory directory has no parent"))?;
    if parent != path {
        ensure_real_dir(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        anyhow::bail!(
            "Core Memory directory must be a real directory: {}",
            path.display()
        );
    }
    Ok(())
}

struct RepositoryLock {
    _file: fs::File,
}

fn acquire_repository_lock() -> Result<RepositoryLock> {
    let dir = migration_dir()?;
    ensure_real_dir(&dir)?;
    let path = dir.join(".core-memory-v2.lock");
    let started = Instant::now();
    loop {
        match crate::platform::try_acquire_exclusive_lock(&path)? {
            Some(file) => return Ok(RepositoryLock { _file: file }),
            None if started.elapsed() < LOCK_TIMEOUT => std::thread::sleep(LOCK_POLL),
            None => anyhow::bail!("timed out waiting for the Core Memory repository lock"),
        }
    }
}

fn manifest_path() -> Result<PathBuf> {
    Ok(migration_dir()?.join("core-memory-v2.json"))
}

fn migration_dir() -> Result<PathBuf> {
    Ok(crate::paths::root_dir()?.join("memory").join("migrations"))
}

fn snapshot_dir() -> Result<PathBuf> {
    Ok(migration_dir()?.join("snapshots"))
}

fn snapshot_path(scope_key: &str) -> Result<PathBuf> {
    let digest = blake3::hash(scope_key.as_bytes()).to_hex();
    Ok(snapshot_dir()?.join(format!("{}.md", &digest[..20])))
}

fn load_manifest() -> Result<CoreMemoryMigrationManifest> {
    let path = manifest_path()?;
    let Some(bytes) = read_regular_optional_unbounded(&path)? else {
        return Ok(CoreMemoryMigrationManifest {
            version: MANIFEST_VERSION,
            scopes: BTreeMap::new(),
        });
    };
    let manifest: CoreMemoryMigrationManifest =
        serde_json::from_slice(&bytes).with_context(|| format!("parse {}", path.display()))?;
    if manifest.version > MANIFEST_VERSION {
        anyhow::bail!("Core Memory migration manifest is from a newer version");
    }
    Ok(manifest)
}

fn save_manifest(manifest: &CoreMemoryMigrationManifest) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(manifest)?;
    Ok(crate::platform::write_atomic(&manifest_path()?, &bytes)?)
}

fn read_regular_optional_unbounded(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        anyhow::bail!("Core Memory metadata path is not a regular file");
    }
    Ok(Some(fs::read(path)?))
}

fn backup_legacy_once(scope: &CoreMemoryScope, _legacy: &Path, bytes: &[u8]) -> Result<()> {
    let digest = blake3::hash(scope.key().as_bytes()).to_hex();
    let backup = migration_dir()?.join(format!(
        "legacy-{}-{}.bak",
        &digest[..12],
        content_hash(bytes)
    ));
    if backup.exists() {
        return Ok(());
    }
    crate::platform::write_atomic(&backup, bytes)?;
    Ok(())
}

fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global_scope() -> CoreMemoryScope {
        CoreMemoryScope::Global
    }

    #[test]
    fn legacy_file_migrates_and_stays_mirrored() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "legacy content").unwrap();
            let index = load_index(&global_scope()).unwrap();
            assert_eq!(index.content.as_deref(), Some("legacy content"));
            assert_eq!(index.state, CoreMemoryMigrationState::MigratedLegacy);
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "legacy content"
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("memory.md")).unwrap(),
                "legacy content"
            );
        });
    }

    #[test]
    fn old_version_legacy_write_is_recovered_into_canonical() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "v1").unwrap();
            load_index(&global_scope()).unwrap();
            fs::write(temp.path().join("memory.md"), "old binary changed this").unwrap();
            let index = load_index(&global_scope()).unwrap();
            assert_eq!(index.state, CoreMemoryMigrationState::RecoveredLegacyChange);
            assert_eq!(index.content.as_deref(), Some("old binary changed this"));
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "old binary changed this"
            );
        });
    }

    #[test]
    fn two_sided_change_returns_last_synced_snapshot_and_blocks_write() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "synced").unwrap();
            let initial = load_index(&global_scope()).unwrap();
            fs::write(temp.path().join("memory.md"), "legacy changed").unwrap();
            fs::write(temp.path().join("memory/MEMORY.md"), "canonical changed").unwrap();
            let conflict = load_index(&global_scope()).unwrap();
            assert_eq!(conflict.state, CoreMemoryMigrationState::Conflict);
            assert_eq!(conflict.content.as_deref(), Some("synced"));
            assert!(save_index(&global_scope(), "new", initial.file_hash.as_deref()).is_err());
        });
    }

    #[test]
    fn conflict_without_trusted_snapshot_injects_neither_side() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::create_dir_all(temp.path().join("memory")).unwrap();
            fs::write(temp.path().join("memory.md"), "legacy changed").unwrap();
            fs::write(temp.path().join("memory/MEMORY.md"), "canonical changed").unwrap();

            let conflict = load_index(&global_scope()).unwrap();

            assert_eq!(conflict.state, CoreMemoryMigrationState::Conflict);
            assert!(conflict.content.is_none());
        });
    }

    #[test]
    fn missing_last_synced_snapshot_fails_closed_during_conflict() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "synced").unwrap();
            load_index(&global_scope()).unwrap();
            let manifest = load_manifest().unwrap();
            let snapshot = manifest
                .scopes
                .get(&global_scope().key())
                .map(|entry| entry.snapshot_path.clone())
                .unwrap();
            fs::remove_file(snapshot).unwrap();
            fs::write(temp.path().join("memory.md"), "legacy changed").unwrap();
            fs::write(temp.path().join("memory/MEMORY.md"), "canonical changed").unwrap();

            let conflict = load_index(&global_scope()).unwrap();

            assert_eq!(conflict.state, CoreMemoryMigrationState::Conflict);
            assert!(conflict.content.is_none());
        });
    }

    #[test]
    fn owner_can_inspect_and_explicitly_merge_two_sided_conflict() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "synced").unwrap();
            load_index(&global_scope()).unwrap();
            fs::write(temp.path().join("memory.md"), "legacy changed").unwrap();
            fs::write(temp.path().join("memory/MEMORY.md"), "canonical changed").unwrap();

            let conflict = load_conflict(&global_scope()).unwrap().unwrap();
            assert_eq!(conflict.last_synced_content.as_deref(), Some("synced"));
            assert_eq!(conflict.canonical_content, "canonical changed");
            assert_eq!(conflict.legacy_content, "legacy changed");

            let saved = resolve_conflict(
                &global_scope(),
                CoreMemoryConflictResolution {
                    choice: CoreMemoryConflictChoice::Merged,
                    expected_canonical_hash: conflict.canonical_hash,
                    expected_legacy_hash: conflict.legacy_hash,
                    merged_content: Some("merged safely".into()),
                },
            )
            .unwrap();
            assert_eq!(saved.state, CoreMemoryMigrationState::Mirrored);
            assert_eq!(saved.content.as_deref(), Some("merged safely"));
            assert_eq!(
                fs::read_to_string(temp.path().join("memory.md")).unwrap(),
                "merged safely"
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "merged safely"
            );
        });
    }

    #[test]
    fn conflict_resolution_rejects_stale_source_hashes() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "synced").unwrap();
            load_index(&global_scope()).unwrap();
            fs::write(temp.path().join("memory.md"), "legacy changed").unwrap();
            fs::write(temp.path().join("memory/MEMORY.md"), "canonical changed").unwrap();
            let conflict = load_conflict(&global_scope()).unwrap().unwrap();
            fs::write(temp.path().join("memory.md"), "changed again").unwrap();
            assert!(resolve_conflict(
                &global_scope(),
                CoreMemoryConflictResolution {
                    choice: CoreMemoryConflictChoice::Canonical,
                    expected_canonical_hash: conflict.canonical_hash,
                    expected_legacy_hash: conflict.legacy_hash,
                    merged_content: None,
                },
            )
            .is_err());
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "canonical changed"
            );
        });
    }

    #[test]
    fn new_write_updates_both_files_and_manifest() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::write(temp.path().join("memory.md"), "v1").unwrap();
            let initial = load_index(&global_scope()).unwrap();
            let saved = save_index(&global_scope(), "v2", initial.file_hash.as_deref()).unwrap();
            assert_eq!(saved.content.as_deref(), Some("v2"));
            assert_eq!(
                fs::read_to_string(temp.path().join("memory.md")).unwrap(),
                "v2"
            );
            assert_eq!(
                fs::read_to_string(temp.path().join("memory/MEMORY.md")).unwrap(),
                "v2"
            );
            assert!(temp
                .path()
                .join("memory/migrations/core-memory-v2.json")
                .is_file());
        });
    }

    #[test]
    fn core_promotion_secret_guard_rejects_credentials_but_not_plain_preferences() {
        assert!(reject_secret_like_core_content("User prefers concise Chinese replies").is_ok());
        assert!(reject_secret_like_core_content("api_key = sk-1234567890abcdef").is_err());
        assert!(
            reject_secret_like_core_content("Authorization: Bearer this-is-a-sensitive-token")
                .is_err()
        );
        assert_eq!(
            append_core_fact(None, "Prefer concise replies"),
            "# Core Memory\n\n- Prefer concise replies\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn canonical_directory_symlink_is_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let outside = tempfile::tempdir().unwrap();
            symlink(outside.path(), temp.path().join("memory")).unwrap();
            assert!(load_index(&global_scope()).is_err());
        });
    }

    fn topic_input(name: &str) -> CoreMemoryTopicWriteInput {
        CoreMemoryTopicWriteInput {
            file_name: None,
            expected_file_hash: None,
            name: name.to_string(),
            description: format!("Durable details about {name}"),
            memory_type: "user".to_string(),
            content: format!("Full topic body for {name}"),
        }
    }

    #[test]
    fn global_topics_share_repository_and_preserve_manual_core_entries() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            save_index_owner(
                &global_scope(),
                "# Core Memory\n\n- Always answer in Chinese.",
            )
            .unwrap();
            let written = write_topic(&global_scope(), topic_input("Preferences")).unwrap();
            assert!(written.entry.relative_path.starts_with("topics/"));
            assert!(temp
                .path()
                .join("memory/topics/user_preferences.md")
                .is_file());

            let index = load_index(&global_scope()).unwrap().content.unwrap();
            assert!(index.contains("- Always answer in Chinese."));
            assert!(index.contains("## Topics"));
            assert!(index.contains("(topics/user_preferences.md)"));
            assert_eq!(list_topics(&global_scope()).unwrap().len(), 1);
            assert_eq!(
                read_topic(&global_scope(), &written.entry.file_name)
                    .unwrap()
                    .content,
                "Full topic body for Preferences"
            );
        });
    }

    #[test]
    fn topic_updates_and_deletes_require_current_hash() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let created = write_topic(&global_scope(), topic_input("Workflow")).unwrap();
            let mut update = topic_input("Workflow");
            update.file_name = Some(created.entry.file_name.clone());
            assert!(write_topic(&global_scope(), update.clone()).is_err());
            update.expected_file_hash = Some(created.file_hash.clone());
            let updated = write_topic(&global_scope(), update).unwrap();
            assert!(
                delete_topic(&global_scope(), &updated.entry.file_name, Some("stale")).is_err()
            );
            assert!(delete_topic(
                &global_scope(),
                &updated.entry.file_name,
                Some(&updated.file_hash)
            )
            .unwrap());
        });
    }

    #[test]
    fn canonical_index_enforces_the_documented_line_ceiling() {
        let too_many_lines = std::iter::repeat("- one")
            .take(CORE_INDEX_MAX_LINES + 1)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(validate_index_content(&too_many_lines).is_err());
    }

    #[test]
    fn promoted_fact_is_inserted_before_progressive_topic_index() {
        let current = "# Core Memory\n\n- existing\n\n## Topics\n\n- [Workflow](topics/workflow.md) — details\n";
        let next = append_core_fact(Some(current), "new preference");
        let fact = next.find("- new preference").unwrap();
        let topics = next.find("## Topics").unwrap();
        assert!(fact < topics);
        assert!(next.contains("[Workflow](topics/workflow.md)"));
    }

    #[test]
    fn session_snapshot_stays_frozen_until_explicit_invalidation() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            save_index_owner(&global_scope(), "first").unwrap();
            let first = session_snapshot("session-1", "ha-main", None, true).unwrap();
            save_index_owner(&global_scope(), "second").unwrap();
            let still_first = session_snapshot("session-1", "ha-main", None, true).unwrap();
            assert_eq!(
                still_first
                    .global
                    .as_ref()
                    .map(|layer| layer.content.as_str()),
                Some("first")
            );
            assert_eq!(first.fingerprint, still_first.fingerprint);

            invalidate_session_snapshot("session-1");
            let refreshed = session_snapshot("session-1", "ha-main", None, true).unwrap();
            assert_eq!(
                refreshed
                    .global
                    .as_ref()
                    .map(|layer| layer.content.as_str()),
                Some("second")
            );
            assert_ne!(first.fingerprint, refreshed.fingerprint);
        });
    }

    #[test]
    fn session_snapshot_is_not_evicted_by_unrelated_sessions() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            save_index_owner(&global_scope(), "first").unwrap();
            let first = session_snapshot("old-session", "ha-main", None, true).unwrap();

            // This exceeds the former 512-entry LRU. A semantic session
            // snapshot must survive unrelated activity until an explicit
            // refresh boundary removes it.
            for index in 0..520 {
                session_snapshot(&format!("other-session-{index}"), "ha-main", None, true).unwrap();
            }
            save_index_owner(&global_scope(), "second").unwrap();

            let still_first = session_snapshot("old-session", "ha-main", None, true).unwrap();
            assert_eq!(first.fingerprint, still_first.fingerprint);
            assert_eq!(
                still_first
                    .global
                    .as_ref()
                    .map(|layer| layer.content.as_str()),
                Some("first")
            );
        });
    }

    #[test]
    fn session_snapshot_excludes_global_when_agent_does_not_share_it() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            save_index_owner(&global_scope(), "global-only").unwrap();
            let private = session_snapshot("session-private", "ha-main", None, false).unwrap();
            assert!(private.global.is_none());

            // Changing the policy is part of the snapshot identity, so a
            // stale private snapshot cannot mask or expose Global Core.
            let shared = session_snapshot("session-private", "ha-main", None, true).unwrap();
            assert_eq!(
                shared.global.as_ref().map(|layer| layer.content.as_str()),
                Some("global-only")
            );
            assert_ne!(private.fingerprint, shared.fingerprint);
        });
    }

    #[test]
    fn project_legacy_topics_remain_readable_without_physical_migration() {
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            let project_id = "00000000-0000-0000-0000-000000000001";
            let scope = CoreMemoryScope::Project {
                id: project_id.to_string(),
            };
            let dir = temp.path().join("projects").join(project_id).join("memory");
            fs::create_dir_all(&dir).unwrap();
            fs::write(
                dir.join("project_architecture.md"),
                "---\nname: \"Architecture\"\ndescription: \"Legacy layout\"\nmetadata:\n  type: project\n---\n\nLegacy body\n",
            )
            .unwrap();
            let listed = list_topics(&scope).unwrap();
            assert_eq!(listed[0].relative_path, "project_architecture.md");
            assert_eq!(
                read_topic(&scope, "project_architecture.md")
                    .unwrap()
                    .content,
                "Legacy body"
            );

            let fresh = write_topic(&scope, topic_input("Commands")).unwrap();
            assert!(fresh.entry.relative_path.starts_with("topics/"));
            assert!(dir.join("project_architecture.md").is_file());
            assert!(dir.join(&fresh.entry.relative_path).is_file());
        });
    }

    #[cfg(unix)]
    #[test]
    fn agent_directory_symlink_ancestor_is_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        crate::test_support::with_env_vars(&[("HA_DATA_DIR", temp.path())], || {
            fs::create_dir(temp.path().join("agents")).unwrap();
            let outside = tempfile::tempdir().unwrap();
            symlink(outside.path(), temp.path().join("agents/ha-main")).unwrap();
            assert!(load_index(&CoreMemoryScope::Agent {
                id: "ha-main".to_string()
            })
            .is_err());
        });
    }
}
