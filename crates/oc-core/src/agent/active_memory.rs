//! Active Memory — pre-reply recall injection (Phase B1).
//!
//! Each user turn, before the main chat request, the agent asks a bounded
//! side_query to distill the single most relevant memory for the current
//! message. The resulting sentence is exposed to the provider layer as an
//! independent cache block (alongside the static system prompt and the
//! awareness suffix), so its churn does not invalidate the prefix cache.
//!
//! Design principles:
//! - **Bounded**: hard timeout from `ActiveMemoryConfig.timeout_ms` (default 3s).
//!   On timeout we silently skip injection and fall back to the passive memory
//!   section already baked into the system prompt.
//! - **Cache-friendly**: `side_query` reuses the main conversation's prompt
//!   prefix, so the incremental cost is a short suffix + short output.
//! - **Shortlist first**: a cheap FTS/vector search on the local memory
//!   backend produces up to `candidate_limit` candidates; only then do we
//!   ask the LLM to pick one. If the shortlist is empty we skip the LLM
//!   call entirely.
//! - **TTL cache**: repeating the same user message within `cache_ttl_secs`
//!   reuses the last recall without another LLM call.
//!
//! The Active Memory engine does not mutate conversation history, the system
//! prompt, or any persisted state. Its only side effect is updating the
//! `active_memory_suffix` slot on `AssistantAgent`, which providers read
//! when constructing the API request.

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::agent_config::ActiveMemoryConfig;
use crate::memory::{MemoryEntry, MemoryScope, MemorySearchQuery};

/// Soft cap for the per-session recall cache. Large enough that typical
/// usage never evicts inside the TTL window; small enough that the O(n)
/// eviction scan is trivially cheap.
const MAX_CACHE_ENTRIES: usize = 32;

/// Snapshot of the agent-level config fields Active Memory needs every
/// user turn. Cached on `ActiveMemoryState` so the hot path doesn't hit
/// disk (reading `agent.json` + associated markdown files) per turn.
#[derive(Clone)]
pub struct CachedAgentConfig {
    pub active_memory: ActiveMemoryConfig,
    pub shared_global: bool,
}

/// Per-agent Active Memory runtime state: recall cache + cached agent
/// config snapshot.
pub struct ActiveMemoryState {
    /// LRU-ish cache keyed by hash(user_message). Kept small (<= 32 entries)
    /// because this is a per-session state and users rarely revisit the
    /// exact same phrasing after the TTL expires.
    cache: Mutex<HashMap<u64, CachedRecall>>,
    /// Cached config snapshot. Lazily filled on the first turn and
    /// invalidated by [`ActiveMemoryState::invalidate_config`] (called
    /// from `AssistantAgent::set_agent_id`).
    agent_config: Mutex<Option<CachedAgentConfig>>,
}

struct CachedRecall {
    /// `None` is a valid cached value meaning "we ran the side_query for
    /// this user message and the LLM decided nothing was worth recalling
    /// (returned NONE / empty)". A cache miss (no entry at all) is
    /// signalled separately by `get_cached` returning `None`.
    text: Option<String>,
    created_at: Instant,
}

impl ActiveMemoryState {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            agent_config: Mutex::new(None),
        }
    }

    /// Return the cached agent-config snapshot, or fetch + cache it.
    /// The loader is invoked at most once per agent-id lifetime; callers
    /// must invalidate via [`Self::invalidate_config`] when the agent id
    /// changes so the next turn re-reads disk.
    pub fn agent_config_or_load<F>(&self, load: F) -> CachedAgentConfig
    where
        F: FnOnce() -> CachedAgentConfig,
    {
        let mut guard = self.agent_config.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(cfg) = guard.as_ref() {
            return cfg.clone();
        }
        let cfg = load();
        *guard = Some(cfg.clone());
        cfg
    }

    /// Drop the cached agent-config snapshot. Also clears the recall
    /// cache because the shortlist scopes and TTL both derive from
    /// config and may have changed.
    pub fn invalidate_config(&self) {
        *self.agent_config.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Return the cached recall for this user-text hash if still valid.
    /// `None` return value means "cache miss — go compute".
    pub fn get_cached(&self, hash: u64, ttl: Duration) -> Option<Option<String>> {
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = guard.get(&hash) {
            if entry.created_at.elapsed() <= ttl {
                return Some(entry.text.clone());
            }
            // Expired — drop it lazily.
            guard.remove(&hash);
        }
        None
    }

    pub fn put_cached(&self, hash: u64, text: Option<String>) {
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        // Cheap cap to avoid unbounded growth when users cycle through
        // many different phrasings. When over capacity, evict the single
        // oldest entry by `created_at` (O(n) but n <= 32, and eviction
        // happens at most once per put).
        if guard.len() >= MAX_CACHE_ENTRIES {
            if let Some(oldest_key) = guard
                .iter()
                .min_by_key(|(_, v)| v.created_at)
                .map(|(k, _)| *k)
            {
                guard.remove(&oldest_key);
            }
        }
        guard.insert(
            hash,
            CachedRecall {
                text,
                created_at: Instant::now(),
            },
        );
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
return at most ONE sentence summarizing the single most relevant memory.\n\n\
Rules:\n\
- Max {max_chars} characters; no bullets, no JSON, no XML tags\n\
- Focus on user preferences, project facts, or explicit standing instructions\n\
- Skip trivial recalls already implied by the message\n\
- If none of the candidates meaningfully helps, return exactly the literal text: NONE\n\n\
Candidate memories (top matches from local store):\n\
{candidates}\n\n\
User's latest message:\n\
{user_msg}\n";

/// Build the recall prompt from user text and a shortlist of candidates.
pub fn build_recall_prompt(
    user_msg: &str,
    candidates: &[MemoryEntry],
    max_chars: usize,
) -> String {
    let rendered_candidates = if candidates.is_empty() {
        "(none)".to_string()
    } else {
        candidates
            .iter()
            .enumerate()
            .map(|(i, m)| {
                // Trim each candidate to keep the prompt bounded even if
                // someone has an unusually long memory entry. The LLM only
                // needs the gist to decide relevance.
                let content = crate::truncate_utf8(&m.content, 500);
                let tags = if m.tags.is_empty() {
                    String::new()
                } else {
                    format!(" [tags: {}]", m.tags.join(","))
                };
                format!(
                    "{:>2}. ({:?}) {}{}",
                    i + 1,
                    m.memory_type,
                    content,
                    tags
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };

    RECALL_PROMPT_TEMPLATE
        .replace("{max_chars}", &max_chars.to_string())
        .replace("{candidates}", &rendered_candidates)
        .replace("{user_msg}", user_msg.trim())
}

/// Resolve the set of memory scopes to search against for Active Memory
/// recall. Mirrors the passive memory-injection priority order so recall
/// stays consistent with what the system prompt already showed the model.
///
/// Returns the union **Project → Agent → Global** (when project is set),
/// or just Agent → Global otherwise.
pub fn scopes_for_session(session_id: &str, agent_id: &str, shared_global: bool) -> Vec<MemoryScope> {
    let mut scopes = Vec::new();

    // Project scope (if session belongs to one).
    if let Some(db) = crate::get_session_db() {
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
pub fn shortlist_candidates(
    query: &str,
    scopes: &[MemoryScope],
    limit: usize,
) -> Vec<MemoryEntry> {
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

/// Format the final Active Memory suffix section that gets injected into
/// the provider request. Matches the markdown heading style used by the
/// other dynamic blocks (awareness suffix, etc.) so the LLM can tell
/// them apart at a glance.
pub fn format_suffix(text: &str) -> String {
    format!("## Active Memory\n\n{}", text.trim())
}
