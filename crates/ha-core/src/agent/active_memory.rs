//! Active Memory — query-dependent recall outside the stable prompt prefix.
//!
//! Memory UX v2 runs deterministic local Fast Recall by default and can add a
//! bounded LLM Deep Recall pass when the user enables it. The result is exposed
//! to the provider layer as an independent dynamic block, so recall churn does
//! not rewrite Core Memory or invalidate the stable `MEMORY.md` prefix.
//!
//! Design principles:
//! - **Fast by default**: lexical/vector/claim/profile/procedure/graph sources
//!   are fused locally with deterministic relevance and token gates. There is
//!   no extra model call on the normal path.
//! - **Deep is opt-in**: `ActiveMemoryConfig.enabled` is retained as the legacy
//!   compatibility switch for the bounded side-query reranker. V2's explicit
//!   `deepRecall` setting controls this path and defaults off because it adds
//!   latency and tokens.
//! - **Core stays static**: short Global / Agent / Project `MEMORY.md` snapshots
//!   are built once per session and never replaced by dynamic database recall.
//! - **Bounded**: the optional Deep pass has a hard timeout from
//!   `ActiveMemoryConfig.timeout_ms` (default 8s). Timeout falls back to the
//!   already selected Fast Recall candidates.
//! - **Cache-friendly**: `side_query` reuses the main conversation's prompt
//!   prefix, so the incremental cost is a short suffix + short output.
//! - **Shortlist first**: local retrieval produces up to `candidate_limit`
//!   candidates; Deep Recall, when enabled, only reranks that bounded set.
//! - **TTL cache**: repeating the same user message within `cache_ttl_secs`
//!   reuses the last recall without another LLM call.
//!
//! The Active Memory engine does not mutate conversation history, the system
//! prompt, or any persisted state. Its only side effect is updating the
//! `active_memory_suffix` slot on `AssistantAgent`, which providers read
//! when constructing the API request.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::agent_config::ActiveMemoryConfig;
use crate::memory::{MemoryEntry, MemoryScope, MemorySearchQuery};
use crate::ttl_cache::TtlCache;

/// Soft cap for the per-session recall cache. Large enough that typical
/// usage never evicts inside the TTL window; small enough that the O(n)
/// eviction scan is trivially cheap.
const MAX_CACHE_ENTRIES: usize = 32;

/// Snapshot of the agent-level config fields Active Memory needs every
/// user turn. Cached on `ActiveMemoryState` so the hot path only stats
/// `agent.json` and avoids parsing the config unless the file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentConfigFingerprint {
    pub modified_at: Option<SystemTime>,
    pub len: u64,
}

pub fn agent_config_fingerprint(agent_id: &str) -> Option<AgentConfigFingerprint> {
    let path = crate::paths::agent_dir(agent_id).ok()?.join("agent.json");
    let metadata = std::fs::metadata(path).ok()?;
    Some(AgentConfigFingerprint {
        modified_at: metadata.modified().ok(),
        len: metadata.len(),
    })
}

#[derive(Clone)]
pub struct CachedAgentConfig {
    pub fingerprint: Option<AgentConfigFingerprint>,
    pub memory_enabled: bool,
    pub active_memory: ActiveMemoryConfig,
    pub shared_global: bool,
    pub procedure_memory: crate::agent_config::ProcedureMemoryConfig,
    pub graph_memory: crate::agent_config::GraphMemoryConfig,
    pub retrieval_planner: crate::agent_config::RetrievalPlannerConfig,
    pub prompt_budget: usize,
}

/// Frontend- and log-safe reference to a memory candidate considered by Active
/// Memory. It intentionally carries previews and scores, never raw evidence
/// quotes, so the trace can be emitted through EventBus without widening data
/// access beyond what was already injected into this turn's prompt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMemoryCandidateRef {
    /// `memory` for legacy rows, `claim` for structured next-gen memory.
    pub kind: String,
    /// Stable source id: integer memory id or claim uuid.
    pub id: String,
    /// User-facing type label (`user`, `feedback`, `preference`, ...).
    pub source_type: String,
    /// Scope label (`global`, `agent:<id>`, `project:<id>`).
    pub scope: String,
    /// Short sanitized-ish preview suitable for chips and diagnostics.
    pub preview: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salience: Option<f32>,
}

/// Unified memory-reference metadata attached to assistant messages. Today the
/// first producer is Active Memory; Retrieval Planner can append static memory,
/// pinned claims, profile, or knowledge references without changing the message
/// storage contract again.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsedMemoryRef {
    /// `memory` for legacy rows, `claim` for structured next-gen memory.
    pub kind: String,
    /// Stable source id: integer memory id or claim uuid.
    pub id: String,
    /// User-facing type label (`user`, `feedback`, `preference`, ...).
    pub source_type: String,
    /// Scope label (`global`, `agent:<id>`, `project:<id>`).
    pub scope: String,
    /// Which retrieval layer produced this reference.
    pub origin: String,
    /// How this layer used the item (`selected`, `candidate`, `injected`, ...).
    pub role: String,
    /// Short preview suitable for chips and diagnostics.
    pub preview: String,
    /// Optional source path for stores whose stable source id is not directly
    /// openable by the frontend (for example Knowledge notes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Optional precise source coordinates for openable stores. Line is 1-based;
    /// column is 0-based, matching the Knowledge D14 contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col: Option<u32>,
    /// Optional Markdown heading path / Obsidian block id for Knowledge notes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub salience: Option<f32>,
}

/// The recall object cached per user-text hash and emitted to the UI when a
/// turn receives Active Memory context.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveMemoryRecall {
    pub summary: String,
    #[serde(default = "default_recall_mode")]
    pub mode: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected: Option<ActiveMemoryCandidateRef>,
    /// V2 fast recall can inject more than one bounded candidate. `selected`
    /// remains the first item for wire/UI compatibility.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_candidates: Vec<ActiveMemoryCandidateRef>,
    pub candidates: Vec<ActiveMemoryCandidateRef>,
    pub total_candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    pub cached: bool,
}

fn default_recall_mode() -> String {
    "legacy".to_string()
}

impl ActiveMemoryRecall {
    pub fn used_memory_refs(&self) -> Vec<UsedMemoryRef> {
        let mut selected_keys = std::collections::HashSet::new();
        for candidate in &self.selected_candidates {
            selected_keys.insert((candidate.kind.as_str(), candidate.id.as_str()));
        }
        if let Some(candidate) = self.selected.as_ref() {
            selected_keys.insert((candidate.kind.as_str(), candidate.id.as_str()));
        }
        let mut refs = Vec::new();

        if self.selected_candidates.is_empty() {
            if let Some(selected) = self.selected.as_ref() {
                refs.push(used_memory_ref_from_candidate(selected, "selected"));
            }
        } else {
            refs.extend(
                self.selected_candidates
                    .iter()
                    .map(|candidate| used_memory_ref_from_candidate(candidate, "selected")),
            );
        }

        for candidate in &self.candidates {
            let is_selected =
                selected_keys.contains(&(candidate.kind.as_str(), candidate.id.as_str()));
            if is_selected {
                continue;
            }
            refs.push(used_memory_ref_from_candidate(candidate, "candidate"));
        }

        refs
    }
}

fn used_memory_ref_from_candidate(
    candidate: &ActiveMemoryCandidateRef,
    role: &str,
) -> UsedMemoryRef {
    let origin = match candidate.kind.as_str() {
        "profile" => "profile",
        "procedure" | "episode" => "experience",
        "graph" => "graph",
        _ => "active_memory",
    };
    UsedMemoryRef {
        kind: candidate.kind.clone(),
        id: candidate.id.clone(),
        source_type: candidate.source_type.clone(),
        scope: candidate.scope.clone(),
        origin: origin.to_string(),
        role: role.to_string(),
        preview: candidate.preview.clone(),
        path: None,
        line: None,
        col: None,
        heading_path: None,
        block_id: None,
        score: candidate.score,
        confidence: candidate.confidence,
        salience: candidate.salience,
    }
}

#[derive(Clone, Debug)]
pub struct ParsedRecallResponse {
    pub summary: String,
    pub selected_index: Option<usize>,
}

/// Per-agent Active Memory runtime state: recall cache + cached agent
/// config snapshot.
///
/// The recall cache stores `Option<ActiveMemoryRecall>` so that "we ran the
/// side_query and got NONE" is distinct from "no entry at all" — the
/// outer `Option` returned by `get_cached` is the cache hit/miss signal,
/// while the inner option is the recalled trace (or its LLM-confirmed absence).
pub struct ActiveMemoryState {
    /// Per-state TtlCache keyed by hash(user_message). TTL is supplied
    /// per-`get_cached` call so config changes (`cache_ttl_secs`) take
    /// effect immediately without restamping existing entries.
    cache: TtlCache<u64, Option<ActiveMemoryRecall>>,
    /// Cached config snapshot. Lazily filled on the first turn and
    /// invalidated by [`ActiveMemoryState::invalidate_config`] (called
    /// from `AssistantAgent::set_agent_id`) or by an `agent.json`
    /// fingerprint change.
    agent_config: Mutex<Option<CachedAgentConfig>>,
}

impl ActiveMemoryState {
    pub fn new() -> Self {
        Self {
            cache: TtlCache::new(MAX_CACHE_ENTRIES),
            agent_config: Mutex::new(None),
        }
    }

    /// Return the cached agent-config snapshot, or fetch + cache it.
    /// The loader is invoked at most once per stable `agent.json`
    /// fingerprint; callers must invalidate via [`Self::invalidate_config`]
    /// when the agent id changes so the next turn re-reads disk.
    pub fn agent_config_or_load<F>(
        &self,
        fingerprint: Option<AgentConfigFingerprint>,
        load: F,
    ) -> CachedAgentConfig
    where
        F: FnOnce() -> CachedAgentConfig,
    {
        let mut guard = self.agent_config.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cfg) = guard.as_ref() {
            if cfg.fingerprint == fingerprint {
                return cfg.clone();
            }
        }
        let cfg = load();
        *guard = Some(cfg.clone());
        self.cache.clear();
        cfg
    }

    pub fn cached_agent_config(
        &self,
        fingerprint: Option<AgentConfigFingerprint>,
    ) -> Option<CachedAgentConfig> {
        self.agent_config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|cfg| cfg.fingerprint == fingerprint)
            .cloned()
    }

    pub fn current_agent_config(&self) -> Option<CachedAgentConfig> {
        self.agent_config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Drop the cached agent-config snapshot. Also clears the recall
    /// cache because the shortlist scopes and TTL both derive from
    /// config and may have changed.
    pub fn invalidate_config(&self) {
        *self.agent_config.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.cache.clear();
    }

    /// Return the cached recall for this user-text hash if still valid.
    /// `None` return value means "cache miss — go compute".
    pub fn get_cached(&self, hash: u64, ttl: Duration) -> Option<Option<ActiveMemoryRecall>> {
        self.cache.get(&hash, ttl)
    }

    pub fn put_cached(&self, hash: u64, recall: Option<ActiveMemoryRecall>) {
        self.cache.put(hash, recall);
    }
}

impl Default for ActiveMemoryState {
    fn default() -> Self {
        Self::new()
    }
}

/// Stable FNV-ish hash for a user message — doesn't need to be
/// cryptographically strong, just consistent within a process.
pub fn hash_user_text(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    // Trim + lower to treat cosmetic variations as the same query.
    text.trim().to_lowercase().hash(&mut h);
    h.finish()
}

/// Recall prompt template. `{candidates}` is a bulleted list with one
/// candidate per line; `{user_msg}` is the raw user turn; `{max_chars}`
/// is inlined so the LLM respects the length budget.
const RECALL_PROMPT_TEMPLATE: &str = "\
You are a memory retrieval assistant for the user's assistant agent.\n\
Given the user's latest message and a shortlist of candidate memories, \
choose at most ONE candidate and summarize why it is useful for the next reply.\n\n\
Rules:\n\
- Return ONLY JSON: {\"selected\": <candidate number or null>, \"summary\": \"...\"}\n\
- `summary` max {max_chars} characters; no bullets, no XML tags\n\
- Focus on user preferences, project facts, or explicit standing instructions\n\
- Skip trivial recalls already implied by the message\n\
- If none of the candidates meaningfully helps, return {\"selected\": null, \"summary\": \"NONE\"}\n\n\
Candidate memories (top matches from local store):\n\
{candidates}\n\n\
User's latest message:\n\
{user_msg}\n";

pub(crate) fn scope_label(scope: &MemoryScope) -> String {
    match scope {
        MemoryScope::Global => "global".to_string(),
        MemoryScope::Agent { id } => format!("agent:{id}"),
        MemoryScope::Project { id } => format!("project:{id}"),
    }
}

pub(crate) fn preview_line(content: &str) -> String {
    crate::truncate_utf8(content.lines().next().unwrap_or(content).trim(), 180).to_string()
}

/// Build the candidate references that power UI chips and retrieval
/// diagnostics. Order exactly matches the numbered prompt rendered by
/// [`build_recall_prompt`].
pub fn candidate_refs(
    candidates: &[MemoryEntry],
    claims: &[crate::memory::claims::ClaimRecord],
) -> Vec<ActiveMemoryCandidateRef> {
    let mut refs = Vec::with_capacity(candidates.len() + claims.len());
    for m in candidates {
        refs.push(ActiveMemoryCandidateRef {
            kind: "memory".to_string(),
            id: m.id.to_string(),
            source_type: m.memory_type.as_str().to_string(),
            scope: scope_label(&m.scope),
            preview: preview_line(&m.content),
            score: m.relevance_score,
            confidence: None,
            salience: None,
        });
    }
    for c in claims {
        refs.push(ActiveMemoryCandidateRef {
            kind: "claim".to_string(),
            id: c.id.clone(),
            source_type: c.claim_type.clone(),
            scope: if c.scope_type == "global" {
                "global".to_string()
            } else {
                format!("{}:{}", c.scope_type, c.scope_id.as_deref().unwrap_or("?"))
            },
            preview: preview_line(&c.content),
            score: None,
            confidence: Some(c.confidence),
            salience: Some(c.salience),
        });
    }
    refs
}

/// Build the recall prompt from user text and a shortlist of candidate memories
/// plus (Active Memory v2) candidate claims. Both render into one numbered list
/// so the LLM picks the single most relevant entry regardless of source; claims
/// carry a `claim:<type>` tag to disambiguate them from legacy memory.
pub fn build_recall_prompt(
    user_msg: &str,
    candidates: &[MemoryEntry],
    claims: &[crate::memory::claims::ClaimRecord],
    max_chars: usize,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    // Trim each candidate to keep the prompt bounded even if someone has an
    // unusually long entry. The LLM only needs the gist to decide relevance.
    for (i, m) in candidates.iter().enumerate() {
        let content = crate::truncate_utf8(&m.content, 500);
        let tags = if m.tags.is_empty() {
            String::new()
        } else {
            format!(" [tags: {}]", m.tags.join(","))
        };
        lines.push(format!(
            "{:>2}. [memory:{}] ({:?}|{}) {}{}",
            i + 1,
            m.id,
            m.memory_type,
            scope_label(&m.scope),
            content,
            tags
        ));
    }
    for (j, c) in claims.iter().enumerate() {
        let content = crate::truncate_utf8(&c.content, 500);
        let tags = if c.tags.is_empty() {
            String::new()
        } else {
            format!(" [tags: {}]", c.tags.join(","))
        };
        lines.push(format!(
            "{:>2}. [claim:{}] (claim:{}|{}) {}{}",
            candidates.len() + j + 1,
            c.id,
            c.claim_type,
            if c.scope_type == "global" {
                "global".to_string()
            } else {
                format!("{}:{}", c.scope_type, c.scope_id.as_deref().unwrap_or("?"))
            },
            content,
            tags
        ));
    }
    let rendered_candidates = if lines.is_empty() {
        "(none)".to_string()
    } else {
        lines.join("\n")
    };

    RECALL_PROMPT_TEMPLATE
        .replace("{max_chars}", &max_chars.to_string())
        .replace("{candidates}", &rendered_candidates)
        .replace("{user_msg}", user_msg.trim())
}

/// Parse the Active Memory sub-query response. Newer prompts ask for JSON with
/// a selected candidate number so the UI can explain what was injected, but we
/// keep a tolerant text fallback to avoid regressing existing model behavior.
pub fn parse_recall_response(raw: &str, max_chars: usize) -> Option<ParsedRecallResponse> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("NONE")
        || trimmed.eq_ignore_ascii_case("NONE.")
    {
        return None;
    }

    if let Some(span) = crate::extract_json_span(trimmed, Some('{')) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(span) {
            let summary = value
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if summary.is_empty()
                || summary.eq_ignore_ascii_case("NONE")
                || summary.eq_ignore_ascii_case("NONE.")
            {
                return None;
            }
            let selected_index = value
                .get("selected")
                .and_then(|v| v.as_u64())
                .and_then(|n| usize::try_from(n).ok())
                .and_then(|n| n.checked_sub(1));
            return Some(ParsedRecallResponse {
                summary: crate::truncate_utf8(summary, max_chars).to_string(),
                selected_index,
            });
        }
    }

    Some(ParsedRecallResponse {
        summary: crate::truncate_utf8(trimmed, max_chars).to_string(),
        selected_index: None,
    })
}

/// Resolve the set of memory scopes to search against for Active Memory
/// recall. Mirrors the passive memory-injection priority order so recall
/// stays consistent with what the system prompt already showed the model.
///
/// Returns the union **Project → Agent → Global** (when project is set),
/// or just Agent → Global otherwise.
pub fn scopes_for_session(
    session_id: &str,
    agent_id: &str,
    shared_global: bool,
    bound_db: Option<&crate::session::SessionDB>,
) -> Vec<MemoryScope> {
    let mut scopes = Vec::new();

    // Project scope (if session belongs to one).
    let global_db = crate::get_session_db();
    if let Some(db) = bound_db.or_else(|| global_db.map(|db| db.as_ref())) {
        if let Ok(Some(session)) = db.get_session(session_id) {
            if let Some(pid) = session.project_id {
                scopes.push(MemoryScope::Project { id: pid });
            }
        }
    }

    // Agent scope (always).
    scopes.push(MemoryScope::Agent {
        id: agent_id.to_string(),
    });

    // Global scope (when the agent is configured to include shared memories).
    if shared_global {
        scopes.push(MemoryScope::Global);
    }

    scopes
}

/// Shortlist candidate memories from the backend for the given user text.
/// Runs the backend `search` once per scope and flattens results, capped
/// at `candidate_limit` total. Returns an empty vec if no backend or no
/// hits — caller should skip the LLM call in that case.
///
/// This is a synchronous call; the caller wraps it in `spawn_blocking`
/// so it doesn't stall the async runtime on slow disks.
pub fn shortlist_candidates(query: &str, scopes: &[MemoryScope], limit: usize) -> Vec<MemoryEntry> {
    let Some(backend) = crate::get_memory_backend() else {
        return Vec::new();
    };

    let mut out: Vec<MemoryEntry> = Vec::new();
    let mut seen_ids: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let per_scope = limit.max(1);

    for scope in scopes {
        let q = MemorySearchQuery {
            query: query.to_string(),
            scope: Some(scope.clone()),
            types: None,
            sources: None,
            agent_id: None,
            limit: Some(per_scope),
        };
        if let Ok(results) = backend.search(&q) {
            for entry in results {
                if seen_ids.insert(entry.id) {
                    out.push(entry);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
    }

    out
}

/// Active Memory v2 (design §7.5): shortlist active claims as recall candidates,
/// mirroring [`shortlist_candidates`] but over the structured claim store.
/// `search_claims` already returns effective-active, scope-filtered claims, so
/// an expired / superseded claim can never loop back into the prompt via Active
/// Memory (§7.5 red line). Returns claims to merge into the recall prompt
/// alongside legacy memories.
pub fn shortlist_claim_candidates(
    query: &str,
    scopes: &[MemoryScope],
    limit: usize,
) -> Vec<crate::memory::claims::ClaimRecord> {
    let mut out: Vec<crate::memory::claims::ClaimRecord> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let per_scope = limit.max(1);
    for scope in scopes {
        if let Ok(found) =
            crate::memory::claims::search_claims(query, Some(scope.clone()), per_scope)
        {
            for c in found {
                if seen.insert(c.id.clone()) {
                    out.push(c);
                    if out.len() >= limit {
                        return out;
                    }
                }
            }
        }
    }
    out
}

/// Format the final Active Memory suffix section that gets injected into
/// the provider request. Matches the markdown heading style used by the
/// other dynamic blocks (awareness suffix, etc.) so the LLM can tell
/// them apart at a glance.
pub fn format_suffix(text: &str) -> String {
    format!("## Active Memory\n\n{}", text.trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::claims::ClaimRecord;

    fn claim(content: &str, claim_type: &str) -> ClaimRecord {
        ClaimRecord {
            id: "c1".into(),
            scope_type: "global".into(),
            scope_id: None,
            claim_type: claim_type.into(),
            subject: "s".into(),
            predicate: "p".into(),
            object: "o".into(),
            content: content.into(),
            tags: vec![],
            confidence: 0.9,
            confidence_source: "derived".into(),
            salience: 0.9,
            freshness_policy: serde_json::json!({}),
            status: "active".into(),
            valid_from: None,
            valid_until: None,
            supersedes_claim_id: None,
            source_run_id: None,
            created_at: "2026-01-01T00:00:00.000Z".into(),
            updated_at: "2026-01-01T00:00:00.000Z".into(),
            retrieval_evidence: None,
        }
    }

    #[test]
    fn recall_prompt_renders_claim_candidates() {
        // Active Memory v2: claims merge into the numbered candidate list with a
        // `claim:<type>` tag, after any legacy memory candidates.
        let claims = vec![claim("User prefers dark mode", "preference")];
        let prompt = build_recall_prompt("what theme do I like?", &[], &claims, 220);
        assert!(prompt.contains("[claim:c1] (claim:preference|global) User prefers dark mode"));
        assert!(prompt.contains("what theme do I like?"));
    }

    #[test]
    fn recall_prompt_none_when_no_candidates() {
        let prompt = build_recall_prompt("hi", &[], &[], 220);
        assert!(prompt.contains("(none)"));
    }

    #[test]
    fn candidate_refs_match_claim_prompt_order() {
        let claims = vec![claim("User prefers dark mode", "preference")];
        let refs = candidate_refs(&[], &claims);
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].kind, "claim");
        assert_eq!(refs[0].id, "c1");
        assert_eq!(refs[0].source_type, "preference");
        assert_eq!(refs[0].scope, "global");
    }

    #[test]
    fn active_memory_exports_unified_used_refs() {
        let selected = ActiveMemoryCandidateRef {
            kind: "memory".into(),
            id: "1".into(),
            source_type: "user".into(),
            scope: "global".into(),
            preview: "Prefers concise Chinese replies.".into(),
            score: Some(0.8),
            confidence: None,
            salience: None,
        };
        let fallback = ActiveMemoryCandidateRef {
            kind: "claim".into(),
            id: "c1".into(),
            source_type: "preference".into(),
            scope: "global".into(),
            preview: "Likes short answers.".into(),
            score: None,
            confidence: Some(0.7),
            salience: Some(0.6),
        };
        let recall = ActiveMemoryRecall {
            summary: "Use the concise-answer preference.".into(),
            mode: "legacy".into(),
            selected: Some(selected.clone()),
            selected_candidates: Vec::new(),
            candidates: vec![selected, fallback],
            total_candidates: 2,
            latency_ms: Some(12),
            cached: false,
        };

        let refs = recall.used_memory_refs();
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].id, "1");
        assert_eq!(refs[0].origin, "active_memory");
        assert_eq!(refs[0].role, "selected");
        assert_eq!(refs[1].id, "c1");
        assert_eq!(refs[1].role, "candidate");
    }

    #[test]
    fn active_memory_config_cache_reloads_when_agent_json_fingerprint_changes() {
        let state = ActiveMemoryState::new();
        let fp1 = AgentConfigFingerprint {
            modified_at: None,
            len: 10,
        };
        let fp2 = AgentConfigFingerprint {
            modified_at: None,
            len: 11,
        };
        let calls = std::cell::Cell::new(0);

        let first = state.agent_config_or_load(Some(fp1), || {
            calls.set(calls.get() + 1);
            CachedAgentConfig {
                fingerprint: Some(fp1),
                memory_enabled: true,
                active_memory: ActiveMemoryConfig::default(),
                shared_global: true,
                procedure_memory: crate::agent_config::ProcedureMemoryConfig::default(),
                graph_memory: crate::agent_config::GraphMemoryConfig::default(),
                retrieval_planner: crate::agent_config::RetrievalPlannerConfig::default(),
                prompt_budget: 5_000,
            }
        });
        let second = state.agent_config_or_load(Some(fp1), || {
            calls.set(calls.get() + 1);
            CachedAgentConfig {
                fingerprint: Some(fp1),
                memory_enabled: false,
                active_memory: ActiveMemoryConfig::default(),
                shared_global: false,
                procedure_memory: crate::agent_config::ProcedureMemoryConfig::default(),
                graph_memory: crate::agent_config::GraphMemoryConfig::default(),
                retrieval_planner: crate::agent_config::RetrievalPlannerConfig::default(),
                prompt_budget: 5_000,
            }
        });
        let third = state.agent_config_or_load(Some(fp2), || {
            calls.set(calls.get() + 1);
            CachedAgentConfig {
                fingerprint: Some(fp2),
                memory_enabled: false,
                active_memory: ActiveMemoryConfig::default(),
                shared_global: false,
                procedure_memory: crate::agent_config::ProcedureMemoryConfig::default(),
                graph_memory: crate::agent_config::GraphMemoryConfig::default(),
                retrieval_planner: crate::agent_config::RetrievalPlannerConfig::default(),
                prompt_budget: 5_000,
            }
        });

        assert_eq!(calls.get(), 2);
        assert!(first.memory_enabled);
        assert!(second.memory_enabled);
        assert!(!third.memory_enabled);
        assert_eq!(third.fingerprint, Some(fp2));
    }

    #[test]
    fn parse_recall_response_accepts_json_and_legacy_text() {
        let parsed = parse_recall_response(
            r#"{"selected":1,"summary":"Use the dark mode preference."}"#,
            220,
        )
        .expect("json recall");
        assert_eq!(parsed.selected_index, Some(0));
        assert_eq!(parsed.summary, "Use the dark mode preference.");

        let legacy =
            parse_recall_response("User prefers concise answers.", 220).expect("legacy recall");
        assert_eq!(legacy.selected_index, None);
        assert_eq!(legacy.summary, "User prefers concise answers.");

        assert!(parse_recall_response(r#"{"selected":null,"summary":"NONE"}"#, 220).is_none());
    }
}
