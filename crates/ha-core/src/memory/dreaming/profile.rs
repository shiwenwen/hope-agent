//! Memory Profile synthesis (next-gen Dreaming Phase 4, design §4.7).
//!
//! Turns active claims into a displayable + injectable profile, layered by
//! scope (global / agent / project). The profile is "what it thinks it knows
//! about you", grounded in active claims — never free invention.
//!
//! Cost model (honours the Light single-call budget):
//! - **idle / cron** run a CHEAP rule-based aggregation (NO side_query): rank
//!   each scope's active claims by `confidence * salience`, render the top N as
//!   bullets.
//! - **manual** additionally runs ONE LLM side_query per scope to merge /
//!   reword the draft for fluency — strictly condensing, never adding facts.
//!
//! Snapshots persist to `memory_profile_snapshots`; the latest per scope is
//! what the system prompt injects (with the legacy profile-tagged section as
//! the fallback when no snapshot exists, so disabling this never blanks the
//! `## User Profile` section). Nothing here mutates claims.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use serde::Serialize;
use serde_json::json;

use super::store;
use super::triggers::{try_claim, DreamTrigger};
use super::types::{DreamPhase, DreamRunStatus};
use crate::agent::AssistantAgent;
use crate::memory::claims::{self, ResolveClaim};
use crate::truncate_utf8;

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Cap on scopes synthesised per run. A manual run issues up to one LLM
/// side_query per scope, so this bounds per-run cost + lock-hold time. Overflow
/// is left for the next run; scopes are processed oldest-snapshot-first (this
/// pipeline mutates no claims, so a fixed order would never cover overflow
/// scopes — staleness ordering lets reruns eventually reach them all).
const MAX_PROFILE_SCOPES: usize = 50;

/// Per-bullet content cap so one verbose claim can't dominate a profile.
const PROFILE_LINE_MAX_CHARS: usize = 240;

/// Claim types excluded from the profile: a `reference` is a resource pointer
/// (URL / file), not a stable trait about the user or project.
const PROFILE_EXCLUDED_CLAIM_TYPES: &[&str] = &["reference"];

const PROFILE_REWRITE_PROMPT: &str = "You are consolidating a long-term memory profile from already-extracted facts. \
Rewrite the draft below into a concise, de-duplicated Markdown bullet list of stable, useful facts about the user or project. \
Merge near-duplicates, drop trivia, keep each bullet to one short line. \
CRITICAL: do not invent anything not supported by the draft — you may only condense and reword existing facts. \
Output ONLY the bullet list (lines starting with '- '), no headings, no preamble.";

/// Terminal summary of a profile-synthesis cycle.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    /// Effective-active claims scanned across all scopes.
    pub scanned: usize,
    /// Distinct scopes considered this run (after the per-run cap).
    pub scopes: usize,
    /// Snapshots actually written (scopes with at least one profile bullet).
    pub snapshots_written: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ProfileReport {
    fn skipped(note: &str, started: Instant) -> Self {
        ProfileReport {
            run_id: None,
            scanned: 0,
            scopes: 0,
            snapshots_written: 0,
            duration_ms: started.elapsed().as_millis() as u64,
            note: Some(note.to_string()),
        }
    }
}

// ── Scope grouping + rule-based rendering (pure, unit-tested) ────

/// A scope key for aggregation: `(scope_type, scope_id)`. Global uses
/// `scope_id = ""` to match the DB storage + injection convention.
type ScopeKey = (String, String);

fn scope_key(c: &ResolveClaim) -> ScopeKey {
    (c.scope_type.clone(), c.scope_id.clone().unwrap_or_default())
}

fn scope_label(scope_type: &str, scope_id: &str) -> String {
    if scope_id.is_empty() {
        scope_type.to_string()
    } else {
        format!("{scope_type}:{scope_id}")
    }
}

/// Render one scope's active claims into a Markdown bullet body: rank by
/// `confidence * salience` (then newest), drop `reference` claims, keep the top
/// `max_lines`, one capped first-line per bullet. Returns "" when nothing
/// renders (caller skips writing an empty snapshot).
fn render_scope_body(claims_in_scope: &[&ResolveClaim], max_lines: usize) -> String {
    let mut items: Vec<&&ResolveClaim> = claims_in_scope
        .iter()
        .filter(|c| !PROFILE_EXCLUDED_CLAIM_TYPES.contains(&c.claim_type.as_str()))
        .collect();
    items.sort_by(|a, b| {
        let sa = a.confidence * a.salience;
        let sb = b.confidence * b.salience;
        sb.partial_cmp(&sa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.created_at.cmp(&a.created_at))
    });

    let mut out = String::new();
    let mut lines = 0usize;
    for c in items {
        if lines >= max_lines {
            break;
        }
        let first = c.content.lines().next().unwrap_or(&c.content).trim();
        if first.is_empty() {
            continue;
        }
        let capped = truncate_utf8(first, PROFILE_LINE_MAX_CHARS);
        out.push_str("- ");
        out.push_str(capped);
        out.push('\n');
        lines += 1;
    }
    out
}

// ── LLM rewrite (manual only, best-effort) ──────────────────────

async fn rewrite_body_llm(
    agent: &AssistantAgent,
    scope_label: &str,
    draft: &str,
    max_tokens: u32,
) -> Option<String> {
    let prompt =
        format!("{PROFILE_REWRITE_PROMPT}\n\nScope: {scope_label}\n\nDraft facts:\n{draft}");
    let resp = agent.side_query(&prompt, max_tokens).await.ok()?;
    let text = resp.text.trim();
    if text.is_empty() {
        return None;
    }
    Some(text.to_string())
}

// ── Orchestration ───────────────────────────────────────────────

/// Run one Profile Synthesis cycle. Mirrors `resolver::run_resolver_cycle`'s
/// lease + run lifecycle (phase = `profile`, lock key `profile:global`).
/// idle / cron → rule-based; manual → rule-based + LLM rewrite.
pub async fn run_profile_synthesis_cycle(trigger: DreamTrigger) -> ProfileReport {
    let started = Instant::now();

    let cfg = crate::config::cached_config().dreaming.clone();
    if !cfg.enabled {
        return ProfileReport::skipped("dreaming disabled in config", started);
    }
    if !cfg.profile_synthesis.enabled {
        return ProfileReport::skipped("profile synthesis disabled in config", started);
    }
    // Hiding the UI button isn't authorization — HTTP / Tauri callers must
    // respect the manual switch too (mirrors `resolver::run_resolver_cycle`).
    if matches!(trigger, DreamTrigger::Manual) && !cfg.manual_enabled {
        return ProfileReport::skipped("manual trigger disabled in config", started);
    }

    let Some(_guard) = try_claim() else {
        return ProfileReport::skipped("another dreaming cycle is already running", started);
    };

    let phase = DreamPhase::Profile;
    let lock_key = format!("{}:global", phase.as_str());
    let run_id = uuid::Uuid::new_v4().to_string();
    let lease_ttl = store::lease_ttl_secs(cfg.narrative_timeout_secs);
    let Some(_lease) = store::acquire_lease(&lock_key, &run_id, lease_ttl) else {
        return ProfileReport::skipped("another instance holds the dreaming lease", started);
    };

    if let Some(s) = store::store() {
        let scope_json = json!({ "phase": "profile" }).to_string();
        if let Err(e) = s.create_run(
            &run_id,
            trigger.as_str(),
            phase.as_str(),
            &scope_json,
            lease_ttl,
        ) {
            app_warn!(
                "memory",
                "dreaming::store",
                "failed to persist run row: {}",
                e
            );
        }
    }
    app_info!(
        "memory",
        "dreaming::profile",
        "profile synthesis started (run={}, trigger={})",
        run_id,
        trigger.as_str()
    );

    // 1. Load every active claim off the async runtime.
    let claims_all = tokio::task::spawn_blocking(|| {
        claims::list_active_claims_for_resolve().unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    // 2. Fold valid_until-expired claims out (effective status), group by scope.
    //    `list_active_claims_for_resolve` returns only stored status='active'
    //    rows (so `ResolveClaim` carries no status field) but does NOT apply
    //    effective_status, so a claim past `valid_until` can slip through — we
    //    re-derive it here with a literal "active", mirroring the read APIs.
    let now = now_rfc3339();
    let active: Vec<ResolveClaim> = claims_all
        .into_iter()
        .filter(|c| {
            claims::is_injectable_status(&claims::effective_status(
                "active",
                c.valid_until.as_deref(),
                &now,
            ))
        })
        .collect();
    let scanned = active.len();

    let mut by_scope: BTreeMap<ScopeKey, Vec<&ResolveClaim>> = BTreeMap::new();
    for c in &active {
        by_scope.entry(scope_key(c)).or_default().push(c);
    }
    let active_keys: HashSet<ScopeKey> = by_scope.keys().cloned().collect();

    // Existing latest snapshot per scope (empty tombstones already excluded by
    // the store), used for (a) staleness ordering and (b) tombstoning scopes
    // whose active claims have all disappeared.
    let existing =
        tokio::task::spawn_blocking(|| store::list_profile_snapshots().unwrap_or_default())
            .await
            .unwrap_or_default();
    let mut last_at: HashMap<ScopeKey, String> = HashMap::new();
    let mut stale_tombstones: Vec<ScopeKey> = Vec::new();
    for r in existing {
        let key = (r.scope_type.clone(), r.scope_id.clone().unwrap_or_default());
        // A scope no longer in the active set but still carrying a non-empty
        // snapshot would keep injecting a ghost profile — schedule an empty
        // tombstone so injection falls back to the legacy rendering.
        if !active_keys.contains(&key) {
            stale_tombstones.push(key.clone());
        }
        last_at.insert(key, r.created_at);
    }

    // Order scopes by staleness (missing/oldest snapshot first) so each capped
    // run advances the laggards. Profile synthesis mutates NO claims, so a fixed
    // BTreeMap order would reprocess the same first N scopes forever and never
    // cover overflow scopes; staleness ordering makes "rerun to continue" real.
    let mut scope_vec: Vec<(ScopeKey, Vec<&ResolveClaim>)> = by_scope.into_iter().collect();
    scope_vec.sort_by(|a, b| {
        let sa = last_at.get(&a.0).map(String::as_str).unwrap_or("");
        let sb = last_at.get(&b.0).map(String::as_str).unwrap_or("");
        sa.cmp(sb).then_with(|| a.0.cmp(&b.0))
    });
    let total_scopes = scope_vec.len();
    let truncated = total_scopes > MAX_PROFILE_SCOPES;
    scope_vec.truncate(MAX_PROFILE_SCOPES);
    let scopes_considered = scope_vec.len();

    // 3. Rule-based body per scope; LLM rewrite on manual (best-effort).
    let use_llm = matches!(trigger, DreamTrigger::Manual);
    let max_lines = cfg.profile_synthesis.max_lines_per_scope.clamp(1, 100);
    let agent = if use_llm {
        match super::pipeline::build_dreaming_agent(&cfg).await {
            Ok(a) => Some(a),
            Err(e) => {
                app_warn!(
                    "memory",
                    "dreaming::profile",
                    "could not build agent for rewrite (using rule-based bodies): {}",
                    e
                );
                None
            }
        }
    } else {
        None
    };

    let mut bodies: Vec<(ScopeKey, String)> = Vec::new();
    for (key, claims_in_scope) in &scope_vec {
        let mut body = render_scope_body(claims_in_scope, max_lines);
        if body.trim().is_empty() {
            // No profile-eligible active claims this round. If the scope had a
            // prior snapshot, write an empty tombstone so injection stops
            // surfacing a stale profile; otherwise write nothing.
            if last_at.contains_key(key) {
                bodies.push((key.clone(), String::new()));
            }
            continue;
        }
        if let Some(agent) = &agent {
            let label = scope_label(&key.0, &key.1);
            if let Some(rewritten) =
                rewrite_body_llm(agent, &label, &body, cfg.narrative_max_tokens).await
            {
                body = rewritten;
            }
        }
        bodies.push((key.clone(), body));
    }
    // Tombstone scopes that vanished from the active set entirely (their claims
    // all expired / merged / archived), so their old snapshot stops injecting.
    for key in stale_tombstones {
        bodies.push((key, String::new()));
    }

    // 4. Persist snapshots + audit decisions off the async runtime.
    let run_for_apply = run_id.clone();
    let written = tokio::task::spawn_blocking(move || {
        let Some(s) = store::store() else {
            return 0usize;
        };
        let mut n = 0usize;
        for (key, body) in &bodies {
            let rationale = if body.is_empty() {
                "profile cleared — no active claims remain for this scope"
            } else {
                "profile snapshot synthesised from active claims"
            };
            match s.insert_profile_snapshot(&key.0, &key.1, body, &run_for_apply) {
                Ok(version) => {
                    if let Err(e) = s.insert_profile_decision(
                        &run_for_apply,
                        &key.0,
                        &key.1,
                        version,
                        rationale,
                    ) {
                        app_warn!(
                            "memory",
                            "dreaming::profile",
                            "failed to record profile decision for {}: {}",
                            scope_label(&key.0, &key.1),
                            e
                        );
                    }
                    n += 1;
                }
                Err(e) => app_warn!(
                    "memory",
                    "dreaming::profile",
                    "failed to write snapshot for {}: {}",
                    scope_label(&key.0, &key.1),
                    e
                ),
            }
        }
        n
    })
    .await
    .unwrap_or(0);

    let duration_ms = started.elapsed().as_millis() as u64;
    let note = truncated.then(|| {
        format!("processed {MAX_PROFILE_SCOPES} of {total_scopes} scopes; rerun to continue")
    });
    if let Some(s) = store::store() {
        if let Err(e) = s.finish_resolver_run(
            &run_id,
            DreamRunStatus::Completed,
            scanned,
            written,
            duration_ms,
            note.as_deref(),
        ) {
            app_warn!("memory", "dreaming::store", "failed to finalise run: {}", e);
        }
    }

    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "dreaming:cycle_complete",
            json!({
                "runId": run_id,
                "trigger": trigger.as_str(),
                "phase": "profile",
                "scanned": scanned,
                "snapshots": written,
                "durationMs": duration_ms,
            }),
        );
    }

    app_info!(
        "memory",
        "dreaming::profile",
        "profile synthesis done (run={}, scanned={}, scopes={}, snapshots={}, duration={}ms)",
        run_id,
        scanned,
        scopes_considered,
        written,
        duration_ms
    );

    ProfileReport {
        run_id: Some(run_id),
        scanned,
        scopes: scopes_considered,
        snapshots_written: written,
        duration_ms,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(
        scope_type: &str,
        scope_id: Option<&str>,
        claim_type: &str,
        content: &str,
        confidence: f32,
        salience: f32,
        created_at: &str,
    ) -> ResolveClaim {
        ResolveClaim {
            id: uuid::Uuid::new_v4().to_string(),
            scope_type: scope_type.to_string(),
            scope_id: scope_id.map(|s| s.to_string()),
            claim_type: claim_type.to_string(),
            subject: "user".to_string(),
            predicate: "prefers".to_string(),
            object: content.to_string(),
            content: content.to_string(),
            confidence,
            confidence_source: "derived".to_string(),
            salience,
            valid_until: None,
            created_at: created_at.to_string(),
            updated_at: created_at.to_string(),
        }
    }

    #[test]
    fn render_ranks_by_confidence_salience_and_caps_lines() {
        let c1 = claim(
            "global",
            None,
            "user_profile",
            "low",
            0.5,
            0.5,
            "2026-01-01T00:00:00.000Z",
        );
        let c2 = claim(
            "global",
            None,
            "user_profile",
            "high",
            0.9,
            0.9,
            "2026-01-01T00:00:00.000Z",
        );
        let c3 = claim(
            "global",
            None,
            "preference",
            "mid",
            0.7,
            0.7,
            "2026-01-01T00:00:00.000Z",
        );
        let refs: Vec<&ResolveClaim> = vec![&c1, &c2, &c3];
        let body = render_scope_body(&refs, 2);
        // Top-2 by confidence*salience: high (0.81), mid (0.49).
        assert_eq!(body, "- high\n- mid\n");
    }

    #[test]
    fn render_excludes_reference_claims() {
        let c1 = claim(
            "global",
            None,
            "reference",
            "http://x",
            0.9,
            0.9,
            "2026-01-01T00:00:00.000Z",
        );
        let c2 = claim(
            "global",
            None,
            "user_profile",
            "keep",
            0.5,
            0.5,
            "2026-01-01T00:00:00.000Z",
        );
        let refs: Vec<&ResolveClaim> = vec![&c1, &c2];
        let body = render_scope_body(&refs, 10);
        assert_eq!(body, "- keep\n");
    }

    #[test]
    fn render_empty_when_no_eligible_claims() {
        let c1 = claim(
            "global",
            None,
            "reference",
            "http://x",
            0.9,
            0.9,
            "2026-01-01T00:00:00.000Z",
        );
        let refs: Vec<&ResolveClaim> = vec![&c1];
        assert!(render_scope_body(&refs, 10).is_empty());
    }

    #[test]
    fn scope_key_uses_empty_string_for_global() {
        let g = claim(
            "global",
            None,
            "user_profile",
            "x",
            0.5,
            0.5,
            "2026-01-01T00:00:00.000Z",
        );
        let a = claim(
            "agent",
            Some("ha-main"),
            "user_profile",
            "y",
            0.5,
            0.5,
            "2026-01-01T00:00:00.000Z",
        );
        assert_eq!(scope_key(&g), ("global".to_string(), String::new()));
        assert_eq!(scope_key(&a), ("agent".to_string(), "ha-main".to_string()));
    }

    #[test]
    fn scope_label_formats_global_and_scoped() {
        assert_eq!(scope_label("global", ""), "global");
        assert_eq!(scope_label("agent", "ha-main"), "agent:ha-main");
        assert_eq!(scope_label("project", "p1"), "project:p1");
    }
}
