#![cfg_attr(test, allow(clippy::needless_return))]

//! Context Pack assembly for the chat hot path (next-gen Dreaming Phase 5,
//! design §4.8).
//!
//! The chat hot path does not run Deep. It consumes a prompt-ready **Pinned
//! Claims** segment rendered from the structured claim layer: high-salience
//! active claims that inject regardless of the current query — the stable "core
//! facts" of a scope (design §4.5: high confidence + high salience → prompt
//! candidate).
//!
//! ## Static vs dynamic split (why Pinned is here but Relevant is not)
//!
//! Pinned Claims are **query-independent**: for a given (agent, project)
//! session they only change at Dreaming cadence, so they fold into the system
//! prompt's cache-stable prefix via `system_prompt::build_memory_section` and
//! cache alongside it. Two reinforcing reasons keep them on the static path:
//! (1) Anthropic's 4 `cache_control` breakpoints are already full (prefix +
//! awareness + active_memory + last-tool), so a new *cacheable* dynamic block
//! would 400; (2) static content belongs with the static prefix.
//!
//! The §4.8 "Relevant Claims" segment is **query-dependent** (it changes every
//! turn with the user message), so it must NOT enter the static prefix — doing
//! so would invalidate the prompt cache on every turn. Per-turn claim recall is
//! served by **Active Memory v2** (its candidate set extends to claims), which
//! already owns a per-turn dynamic suffix channel. So this module renders only
//! the static Pinned segment; dynamic recall lives in `agent::active_memory`.
//!
//! Profile renders on its own existing path (`profile_snapshot`); the legacy
//! SQLite memory section is deduped against active-claim-covered memories
//! upstream (`covered_by_active_claim_memory_ids`) so a fact never
//! double-injects (design §4.8 single-source rule).
//!
//! Every claim that enters the pack is sanitized (`sanitize_for_prompt`) before
//! it reaches the cache-stable prefix — claim content is LLM-derived and must
//! not bypass the prompt-injection filter (red line).

use serde::{Deserialize, Serialize};

use crate::memory::MemoryScope;

/// Salience threshold for a claim to count as "pinned" and inject via the
/// Context Pack (design §4.5). Single source of truth: both
/// [`ContextPackOptions::default`] AND the legacy `# Memory` single-source dedup
/// (`covered_by_active_claim_memory_ids`) read this, so a claim's shadow memory
/// is dropped from the legacy section ONLY when the claim actually clears the pin
/// bar and injects via Pinned — otherwise the shadow stays as the legacy
/// fallback so no fact loses its only static prompt outlet (the dedup threshold
/// must never be more aggressive than the Pinned injection threshold). Baseline
/// salience is 0.5, so 0.7 keeps clearly-above-average facts.
pub const PINNED_MIN_SALIENCE: f32 = 0.7;

/// Provenance for one entry that made it into the Context Pack. Lets the owner
/// plane / future correction loop trace an injected prompt line back to its
/// claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRef {
    pub claim_id: String,
    pub scope_type: String,
    pub scope_id: Option<String>,
    pub claim_type: String,
    /// "pinned" today; future sections (e.g. "relevant") reuse this tag.
    pub section: String,
    /// First sanitized prompt line that was actually rendered.
    pub preview: String,
}

/// Prompt-ready static claim segment for the chat hot path (design §4.8).
/// Profile, Deep-resolver warnings, and dynamic Relevant recall live on their
/// own paths; the struct stays focused on the static Pinned segment plus
/// provenance.
#[derive(Debug, Clone, Default)]
pub struct MemoryContextPack {
    /// Rendered Pinned claim bullets (no heading; the injection site adds
    /// `## Pinned Memory`). Empty when no pinned claims.
    pub pinned_claims_md: String,
    /// What entered the pack, by section (for owner-plane traceability).
    pub source_digest: Vec<SourceRef>,
}

impl MemoryContextPack {
    /// True when the Pinned segment carries no content — lets the caller skip
    /// injection (and the budget math) on the dual-track default where no claims
    /// exist yet.
    pub fn is_empty(&self) -> bool {
        self.pinned_claims_md.is_empty()
    }
}

/// Tunables for pack assembly. Constants today (not user-config): the per-section
/// char cap folds into the system prompt's shared budget downstream, so these
/// only bound how many candidates we fetch/render before that budget trims them.
#[derive(Debug, Clone)]
pub struct ContextPackOptions {
    /// Salience threshold for a claim to count as "pinned" (design §4.5).
    /// Baseline salience is 0.5; 0.7 keeps only clearly-above-average facts.
    pub min_salience: f32,
    pub pinned_limit: usize,
    /// Per-claim first-line char cap before sanitize.
    pub entry_max_chars: usize,
}

impl Default for ContextPackOptions {
    fn default() -> Self {
        Self {
            min_salience: PINNED_MIN_SALIENCE,
            pinned_limit: 12,
            entry_max_chars: 300,
        }
    }
}

/// Build the static Context Pack for a session. `scopes` is the session's
/// effective scope union (Project → Agent → Global). Query-independent by
/// design — the Pinned segment is cache-stable, so this is safe to call once
/// when building the system prompt prefix. Best-effort: a claim-store error on
/// any scope degrades to fewer claims, never an error — the chat path must not
/// break on memory.
/// Feature-owned Context Pack implementation.
#[derive(Clone, Copy)]
pub struct ContextPackRuntime {
    pub build: fn(&[MemoryScope], &ContextPackOptions) -> MemoryContextPack,
}

static RUNTIME: std::sync::OnceLock<ContextPackRuntime> = std::sync::OnceLock::new();

pub fn register_context_pack_runtime(
    runtime: ContextPackRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("memory context pack runtime"))
}

#[cfg(test)]
#[path = "../../../../ha-memory/src/dreaming_context_pack.rs"]
mod test_context_pack;

/// Build the static Context Pack for a session. Missing feature wiring
/// degrades to an empty observer result; it never injects unverified content.
pub fn build_context_pack(scopes: &[MemoryScope], opts: &ContextPackOptions) -> MemoryContextPack {
    if let Some(runtime) = RUNTIME.get() {
        return (runtime.build)(scopes, opts);
    }
    #[cfg(test)]
    {
        return test_context_pack::build_context_pack(scopes, opts);
    }
    #[cfg(not(test))]
    {
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            app_warn!(
                "memory",
                "context_pack_runtime_unavailable",
                "Memory Context Pack runtime is not wired; pinned claim injection is disabled"
            );
        }
        MemoryContextPack::default()
    }
}
