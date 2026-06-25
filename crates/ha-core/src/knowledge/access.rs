//! KB access scope (design D10): default-deny + explicit attach, source-aware
//! with a lineage cap and an incognito short-circuit.
//!
//! Why source-aware: the same session can be shared between GUI and IM, and an
//! IM turn can spawn a subagent — so `session_id` alone can't tell whether the
//! current turn (or its origin) is an IM turn. The cap therefore takes the
//! **strictest value over the whole call chain** (`source` ∪ `origin_source`).
//!
//! WS8 relaxes the IM cap from "always zero" to "zero unless the IM origin
//! opted in": an IM hop in the lineage is allowed only when its **origin**
//! channel account has `kbAccessOptIn` set (and, for group/non-DM chats, the
//! specific chat is separately confirmed). The opt-in is evaluated against the
//! *origin* identity (carried via [`ChannelKbContext`]) so an IM-origin subagent
//! is judged by the account that started the chain, not by the neutral
//! `Subagent` source — an IM turn still can't launder access by spawning.

use std::collections::HashMap;

use super::types::KbAccess;

/// Call-chain source for a KB access check. Mapped from the chat-engine
/// `ChatSource` at the call site (kept decoupled to avoid a module cycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KbAccessSource {
    /// Desktop GUI / HTTP UI operating as the owner.
    Gui,
    Http,
    /// IM channel turn. Zero KB access **unless** the origin account opted in (WS8).
    Im,
    /// Spawned subagent — inherits its origin's cap.
    Subagent,
    Cron,
    Other,
}

impl KbAccessSource {
    fn is_im(self) -> bool {
        matches!(self, KbAccessSource::Im)
    }
}

/// The IM identity of the lineage's origin, for the WS8 opt-in gate. Built at the
/// channel worker (top-level IM turn) and carried unchanged down a subagent
/// chain so the opt-in is always judged against the account/chat that started
/// the lineage. `None` for non-IM turns.
#[derive(Debug, Clone)]
pub struct ChannelKbContext {
    pub channel_id: String,
    pub account_id: String,
    pub chat_id: String,
    /// Non-DM chat (group / forum / broadcast channel). Groups require explicit
    /// per-chat confirmation on top of the account-level opt-in.
    pub is_group: bool,
}

/// Inputs to [`effective_kb_access`]. Built from the tool-exec context at the
/// call site; `is_incognito` is sourced from `sessions.incognito` (single truth)
/// and `im_access_allowed` from the origin channel account config (WS8).
pub struct KnowledgeAccessContext {
    pub session_id: Option<String>,
    pub project_id: Option<String>,
    /// Source of the current hop.
    pub source: KbAccessSource,
    /// Source of the spawn-chain root (an IM-origin subagent stays IM-capped).
    pub origin_source: KbAccessSource,
    pub is_incognito: bool,
    /// Whether the IM origin (if the lineage contains an IM hop) opted into KB
    /// access. Resolved in [`Self::resolve`] from the channel account config;
    /// always `false` for non-IM lineages (where it is never consulted). Kept as
    /// a precomputed bool so [`effective_kb_access`] is pure over this struct and
    /// unit-testable without global config / registry.
    pub im_access_allowed: bool,
}

impl KnowledgeAccessContext {
    /// Build a context, resolving incognito from the session (single truth) and
    /// the IM opt-in from the origin channel account (WS8).
    pub fn resolve(
        session_id: Option<String>,
        project_id: Option<String>,
        source: KbAccessSource,
        origin_source: KbAccessSource,
        channel_info: Option<ChannelKbContext>,
    ) -> Self {
        let is_incognito = crate::session::is_session_incognito(session_id.as_deref());
        // Only consult the channel config when the lineage actually has an IM hop
        // — a stray context shouldn't grant anything for non-IM turns.
        let im_access_allowed = match &channel_info {
            Some(ch) if source.is_im() || origin_source.is_im() => {
                crate::channel::im_kb_access_allowed(
                    &ch.channel_id,
                    &ch.account_id,
                    &ch.chat_id,
                    ch.is_group,
                )
            }
            _ => false,
        };
        Self {
            session_id,
            project_id,
            source,
            origin_source,
            is_incognito,
            im_access_allowed,
        }
    }
}

/// Whether an IM hop in the lineage denies all access (the WS8 gate). Pure over
/// the precomputed `im_access_allowed`, so it is unit-testable without globals.
/// A lineage with no IM hop is never denied here (returns `false`).
fn im_lineage_denied(ctx: &KnowledgeAccessContext) -> bool {
    (ctx.source.is_im() || ctx.origin_source.is_im()) && !ctx.im_access_allowed
}

/// Compute the effective `kb_id → access` map for a context.
///
/// Rules (D10 + WS8):
/// 1. **incognito → {}** (short-circuit, "close = burn").
/// 2. **IM hop in the lineage → {} unless the IM origin opted in** (WS8); a
///    subagent can't launder access via `source=Subagent` because the opt-in is
///    judged against the *origin* identity.
/// 3. `granted = max(session_attach, project_attach)` (write > read).
/// 4. archived KBs are excluded (attach rows kept; un-archive restores).
/// 5. external (bound) roots are capped to `read` unless they opted into
///    external writes (WS7); an opted-in external root needs a write attach to
///    reach `write` and the filesystem scope re-checks at write time (D11).
pub fn effective_kb_access(ctx: &KnowledgeAccessContext) -> HashMap<String, KbAccess> {
    let empty = HashMap::new();

    // (1) incognito short-circuit.
    if ctx.is_incognito {
        return empty;
    }
    // (2) lineage IM cap → zero unless the IM origin opted in (WS8). Even when
    // opted in, the turn still has to clear the attach / archived / external
    // rules below — opt-in only lifts the blanket IM denial.
    if im_lineage_denied(ctx) {
        return empty;
    }

    let Some(registry) = crate::get_knowledge_db() else {
        return empty;
    };

    // (3) max(session, project).
    let mut granted: HashMap<String, KbAccess> = HashMap::new();
    if let Some(pid) = ctx.project_id.as_deref() {
        if let Ok(rows) = registry.list_project_attachments(pid) {
            for (kb_id, access) in rows {
                merge_max(&mut granted, kb_id, access);
            }
        }
    }
    if let Some(sid) = ctx.session_id.as_deref() {
        if let Ok(rows) = registry.list_session_attachments(sid) {
            for (kb_id, access) in rows {
                merge_max(&mut granted, kb_id, access);
            }
        }
    }
    if granted.is_empty() {
        return empty;
    }

    // (4) + (5): drop archived, cap external to read unless the KB opted into
    // external writes (WS7). An opted-in external root still needs a write attach
    // (owner-granted) to reach `Write` here, and the filesystem scope re-checks
    // `read_only` at write time — two independent owner gates.
    let mut out = HashMap::new();
    for (kb_id, access) in granted {
        let Ok(Some(kb)) = registry.get(&kb_id) else {
            continue;
        };
        if kb.archived {
            continue;
        }
        let capped = if kb.is_read_only_root() {
            KbAccess::Read
        } else {
            access
        };
        out.insert(kb_id, capped);
    }
    out
}

fn merge_max(map: &mut HashMap<String, KbAccess>, kb_id: String, access: KbAccess) {
    map.entry(kb_id)
        .and_modify(|cur| {
            if access > *cur {
                *cur = access;
            }
        })
        .or_insert(access);
}

#[cfg(test)]
mod tests {
    use super::*;

    // Build a context directly (bypassing `resolve`'s session + channel lookups)
    // so these tests are deterministic and parallel-safe — the deny rules (1)
    // and (2) short-circuit before any registry hit, so no global `KNOWLEDGE_DB`
    // is touched. The registry-backed rules (max / archived / external cap) are
    // exercised by `registry.rs`'s attach/list tests; the WS8 opt-in *resolution*
    // (config read) is exercised by `channel`'s `im_kb_access_allowed` tests.
    fn ctx(
        source: KbAccessSource,
        origin: KbAccessSource,
        is_incognito: bool,
    ) -> KnowledgeAccessContext {
        ctx_im(source, origin, is_incognito, false)
    }

    fn ctx_im(
        source: KbAccessSource,
        origin: KbAccessSource,
        is_incognito: bool,
        im_access_allowed: bool,
    ) -> KnowledgeAccessContext {
        KnowledgeAccessContext {
            session_id: Some("s".into()),
            project_id: None,
            source,
            origin_source: origin,
            is_incognito,
            im_access_allowed,
        }
    }

    #[test]
    fn incognito_denies_even_owner() {
        // "Close = burn": an incognito session gets zero KB access even for a GUI
        // owner turn, regardless of any attach.
        let c = ctx(KbAccessSource::Gui, KbAccessSource::Gui, true);
        assert!(effective_kb_access(&c).is_empty());
    }

    #[test]
    fn im_turn_denied_without_opt_in() {
        // Default: a direct IM turn with no opt-in is zeroed before any registry
        // lookup (WS8 keeps the old "IM = zero" default).
        let c = ctx(KbAccessSource::Im, KbAccessSource::Im, false);
        assert!(effective_kb_access(&c).is_empty());
        assert!(im_lineage_denied(&c));
    }

    #[test]
    fn im_origin_subagent_denied_without_opt_in() {
        // The lineage cap (G1): a subagent carries a *neutral* `Subagent` source,
        // but its IM origin must still zero KB access without opt-in — otherwise
        // an IM turn could launder access by spawning a subagent.
        let c = ctx(KbAccessSource::Subagent, KbAccessSource::Im, false);
        assert!(effective_kb_access(&c).is_empty());
        assert!(im_lineage_denied(&c));
    }

    #[test]
    fn default_deny_without_attach() {
        // Default-deny baseline: a non-incognito, non-IM owner turn with no
        // attachment resolves to empty (no ambient access).
        let c = ctx(KbAccessSource::Gui, KbAccessSource::Gui, false);
        assert!(effective_kb_access(&c).is_empty());
        assert!(!im_lineage_denied(&c));
    }

    // ── WS8 IM opt-in gate ───────────────────────────────────────────

    #[test]
    fn im_opt_in_lifts_the_blanket_deny() {
        // An opted-in IM turn passes the gate (the lineage is no longer denied
        // outright); whatever it ultimately gets is still subject to the
        // attach/archived/external rules below.
        let c = ctx_im(KbAccessSource::Im, KbAccessSource::Im, false, true);
        assert!(!im_lineage_denied(&c));
    }

    #[test]
    fn im_origin_subagent_opt_in_lifts_deny() {
        // An IM-origin subagent whose origin account opted in also passes the
        // gate — the opt-in follows the origin identity, not the hop source.
        let c = ctx_im(KbAccessSource::Subagent, KbAccessSource::Im, false, true);
        assert!(!im_lineage_denied(&c));
    }

    #[test]
    fn im_group_unconfirmed_stays_denied() {
        // A group chat without per-chat confirmation resolves `im_access_allowed`
        // to false in `resolve()`, so the gate denies exactly like no opt-in.
        let c = ctx_im(KbAccessSource::Im, KbAccessSource::Im, false, false);
        assert!(im_lineage_denied(&c));
        assert!(effective_kb_access(&c).is_empty());
    }

    #[test]
    fn incognito_beats_im_opt_in() {
        // Incognito short-circuits first: even an opted-in IM turn gets zero in an
        // incognito session ("close = burn" outranks the opt-in).
        let c = ctx_im(KbAccessSource::Im, KbAccessSource::Im, true, true);
        assert!(effective_kb_access(&c).is_empty());
    }

    // ── Cron lineage (§3) ────────────────────────────────────────────

    #[test]
    fn cron_lineage_is_owner_not_im_capped() {
        // §3: cron runs map to `KbAccessSource::Cron`, which must NOT be IM-capped
        // — otherwise the WS8 gate would deny KB access exactly as it did when cron
        // borrowed the `Channel`/`Im` bucket. `im_lineage_denied` is the gate under
        // test; a pure-cron lineage passes it and reaches the owner
        // `max(session, project)` path.
        assert!(!KbAccessSource::Cron.is_im());
        let c = ctx(KbAccessSource::Cron, KbAccessSource::Cron, false);
        assert!(!im_lineage_denied(&c));
        // A subagent spawned by a cron run carries origin=Cron (non-IM): neither
        // IM-denied nor launderable into denial (mirror of the IM-origin case).
        let sub = ctx(KbAccessSource::Subagent, KbAccessSource::Cron, false);
        assert!(!im_lineage_denied(&sub));
        // Incognito still beats everything: a cron turn in an incognito session is
        // zeroed before the owner path ("close = burn").
        let incog = ctx(KbAccessSource::Cron, KbAccessSource::Cron, true);
        assert!(effective_kb_access(&incog).is_empty());
    }
}
