//! Deep resolver — temporal expire + duplicate merge + conflict detection over
//! `active` claims (design §4.5). The next-gen difference: not just "remember",
//! but "know when to stop trusting".
//!
//! MVP policy:
//! - **expire** is DETERMINISTIC: any active claim whose `valid_until` has
//!   passed becomes `expired` (no LLM).
//! - **merge / conflict** are LLM-judged per group (same scope + claim_type +
//!   subject + predicate, ≥2 distinct objects). The LLM only classifies the
//!   group relationship; the landing is conservative:
//!   - `duplicates` → fold evidence into one survivor, archive the rest (merge).
//!   - `conflict`   → mark every member `needs_review` (NEVER auto-supersede —
//!     deterministic rules can't tell a real conflict from coexisting facts
//!     like `uses:rust` vs `uses:typescript`; the user decides).
//!   - `independent` → no-op.
//! - Nothing is hard-deleted (design N1); only status changes + an audit row.

use std::collections::HashSet;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::store;
use super::triggers::{try_claim, DreamTrigger};
use super::types::{DreamPhase, DreamRunStatus};
use crate::automation::{self, ModelTaskSpec};
use crate::memory::claims::{self, ResolveClaim};
use crate::provider::ActiveModel;

use crate::util::now_rfc3339;

/// Cap on conflict groups analyzed per resolver run. Each group is one LLM
/// side_query, so this bounds per-run LLM calls, cost, and lock-hold time (a
/// huge memory base can't turn one Dashboard click into unbounded calls / a
/// lease-overrunning run). Overflow is left for the next run — expire/merge
/// shrink the active set each pass, so it converges.
const MAX_RESOLVER_GROUPS: usize = 50;

/// Kind of resolver outcome for one claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolverDecisionType {
    Expire,
    Merge,
    NeedsReview,
}

impl ResolverDecisionType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ResolverDecisionType::Expire => "expire",
            ResolverDecisionType::Merge => "merge",
            ResolverDecisionType::NeedsReview => "needs_review",
        }
    }
}

/// One planned resolver mutation + its audit rationale.
#[derive(Debug, Clone)]
pub struct ResolverDecision {
    pub decision_type: ResolverDecisionType,
    pub claim_id: String,
    pub rationale: String,
    /// For `Merge`: the surviving claim the evidence folds into.
    pub merge_into: Option<String>,
}

/// Terminal summary of a resolver cycle.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolverReport {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub scanned: usize,
    pub expired: usize,
    pub merged: usize,
    pub needs_review: usize,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ResolverReport {
    fn skipped(note: &str, started: Instant) -> Self {
        ResolverReport {
            run_id: None,
            scanned: 0,
            expired: 0,
            merged: 0,
            needs_review: 0,
            duration_ms: started.elapsed().as_millis() as u64,
            note: Some(note.to_string()),
        }
    }
}

// ── Deterministic expiry (pure, unit-tested) ────────────────────

/// Every active claim whose `valid_until` has passed → an `Expire` decision.
/// `now` is RFC3339 millis+Z so the lexical compare matches the injection
/// filter / `effective_status`.
pub fn plan_expirations(claims: &[ResolveClaim], now: &str) -> Vec<ResolverDecision> {
    claims
        .iter()
        .filter_map(|c| {
            let vu = c.valid_until.as_deref()?;
            if !vu.is_empty() && vu < now {
                Some(ResolverDecision {
                    decision_type: ResolverDecisionType::Expire,
                    claim_id: c.id.clone(),
                    rationale: format!("valid_until {vu} has passed"),
                    merge_into: None,
                })
            } else {
                None
            }
        })
        .collect()
}

// ── Conflict grouping (pure, unit-tested) ───────────────────────

/// Group still-live claims (excluding the ones being expired) by
/// `(scope_type, scope_id, claim_type, subject, predicate)`. Only groups with
/// >1 member AND ≥2 distinct normalized objects are returned — same-object dups
/// are Light's canonicalize job, and singletons need no resolution.
pub fn group_conflicts<'a>(
    claims: &'a [ResolveClaim],
    expiring: &HashSet<String>,
) -> Vec<Vec<&'a ResolveClaim>> {
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<(String, String, String, String, String), Vec<&ResolveClaim>> =
        BTreeMap::new();
    for c in claims {
        if expiring.contains(&c.id) {
            continue;
        }
        let key = (
            c.scope_type.clone(),
            c.scope_id.clone().unwrap_or_default(),
            c.claim_type.clone(),
            c.subject.clone(),
            c.predicate.clone(),
        );
        groups.entry(key).or_default().push(c);
    }
    groups
        .into_values()
        .filter(|g| {
            g.len() > 1 && {
                let distinct: HashSet<String> = g
                    .iter()
                    .map(|c| claims::normalize_object(&c.object))
                    .collect();
                distinct.len() > 1
            }
        })
        .collect()
}

// ── LLM group analysis ──────────────────────────────────────────

const RESOLVER_GROUP_PROMPT: &str = r#"You are consolidating long-term memory claims that share the same subject and predicate but have different objects. Decide their relationship.

Claims:
{CLAIMS}

Reply with ONE JSON object, no markdown fences:
{
  "relation": "duplicates | conflict | independent",
  "keepId": "<the claim id to keep when relation=duplicates, otherwise null>",
  "rationale": "one short sentence"
}

- "duplicates": they state the SAME fact in different words → keep the clearest / most-confident one, the rest fold into it.
- "conflict": they CONTRADICT each other (only one can be true now) → the user will review them.
- "independent": they coexist (e.g. the user uses multiple tools / works on several projects) → leave all as-is.
Respond ONLY with the JSON object."#;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GroupVerdict {
    relation: String,
    #[serde(default)]
    keep_id: Option<String>,
    #[serde(default)]
    rationale: Option<String>,
}

fn render_group(group: &[&ResolveClaim]) -> String {
    group
        .iter()
        .map(|c| {
            format!(
                "- id={} confidence={:.2} source={} object=\"{}\" content=\"{}\"",
                c.id, c.confidence, c.confidence_source, c.object, c.content
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_verdict(resp: &str) -> Option<GroupVerdict> {
    let span = crate::extract_json_span(resp.trim(), Some('{'))?;
    serde_json::from_str(span).ok()
}

/// Highest confidence, then newest `created_at`, as the merge survivor when the
/// model didn't name a valid `keepId`.
fn best_keep(group: &[&ResolveClaim]) -> String {
    group
        .iter()
        .max_by(|a, b| {
            a.confidence
                .partial_cmp(&b.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.created_at.cmp(&b.created_at))
        })
        .map(|c| c.id.clone())
        .unwrap_or_default()
}

/// Map a group verdict to per-claim decisions. Pure + unit-tested.
fn map_verdict_to_decisions(group: &[&ResolveClaim], v: &GroupVerdict) -> Vec<ResolverDecision> {
    let rationale = v
        .rationale
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("resolver: {}", v.relation));
    match v.relation.as_str() {
        "duplicates" => {
            // keepId must name a real group member; else pick deterministically.
            let keep = v
                .keep_id
                .as_deref()
                .filter(|k| group.iter().any(|c| c.id == *k))
                .map(|s| s.to_string())
                .unwrap_or_else(|| best_keep(group));
            group
                .iter()
                .filter(|c| c.id != keep)
                .map(|c| ResolverDecision {
                    decision_type: ResolverDecisionType::Merge,
                    claim_id: c.id.clone(),
                    rationale: rationale.clone(),
                    merge_into: Some(keep.clone()),
                })
                .collect()
        }
        // Conservative: a real conflict is the user's call, never auto-supersede.
        "conflict" => group
            .iter()
            .map(|c| ResolverDecision {
                decision_type: ResolverDecisionType::NeedsReview,
                claim_id: c.id.clone(),
                rationale: rationale.clone(),
                merge_into: None,
            })
            .collect(),
        // independent / anything unexpected → no-op.
        _ => Vec::new(),
    }
}

/// Ask the LLM to classify one group, mapping the verdict to decisions.
/// Best-effort: on LLM / parse failure the group is left untouched (no-op).
async fn analyze_group(chain: &[ActiveModel], group: &[&ResolveClaim]) -> Vec<ResolverDecision> {
    let prompt = RESOLVER_GROUP_PROMPT.replace("{CLAIMS}", &render_group(group));
    let resp = match automation::run(ModelTaskSpec {
        purpose: "dreaming.resolver",
        chain: chain.to_vec(),
        session_key: "automation:dreaming",
        instruction: &prompt,
        max_tokens: 512,
    })
    .await
    {
        Ok(r) => r.text,
        Err(e) => {
            app_warn!(
                "memory",
                "dreaming::resolver",
                "group side_query failed: {}",
                e
            );
            return Vec::new();
        }
    };
    match parse_verdict(&resp) {
        Some(v) => map_verdict_to_decisions(group, &v),
        None => Vec::new(),
    }
}

// ── Apply (blocking) ────────────────────────────────────────────

#[derive(Debug, Default, Clone, Copy)]
struct AppliedCounts {
    expired: usize,
    merged: usize,
    needs_review: usize,
}

impl AppliedCounts {
    fn total(&self) -> usize {
        self.expired + self.merged + self.needs_review
    }
}

/// Apply decisions to claim status + write one `dreaming_decisions` audit row
/// per mutation that actually changed a row. Blocking (sync claim/store calls).
fn apply_decisions(run_id: &str, decisions: &[ResolverDecision]) -> AppliedCounts {
    let mut counts = AppliedCounts::default();
    for d in decisions {
        let changed = match d.decision_type {
            ResolverDecisionType::Expire => claims::expire_claim(&d.claim_id).unwrap_or(false),
            ResolverDecisionType::NeedsReview => {
                claims::mark_claim_needs_review(&d.claim_id).unwrap_or(false)
            }
            ResolverDecisionType::Merge => match &d.merge_into {
                Some(keep) => claims::merge_claims(keep, &d.claim_id).unwrap_or(false),
                None => false,
            },
        };
        if !changed {
            continue;
        }
        match d.decision_type {
            ResolverDecisionType::Expire => counts.expired += 1,
            ResolverDecisionType::Merge => counts.merged += 1,
            ResolverDecisionType::NeedsReview => counts.needs_review += 1,
        }
        if let Some(s) = store::store() {
            if let Err(e) = s.insert_claim_decision(
                run_id,
                d.decision_type.as_str(),
                &d.claim_id,
                &d.rationale,
                d.merge_into.as_deref(),
            ) {
                app_warn!(
                    "memory",
                    "dreaming::resolver",
                    "failed to write decision row: {}",
                    e
                );
            }
        }
    }
    counts
}

// ── Orchestration ───────────────────────────────────────────────

/// Run one Deep resolver cycle: expire (deterministic) + merge / conflict
/// (LLM-judged, conservative). Mirrors `pipeline::run_cycle`'s lease + run
/// lifecycle (phase = `deep`, lock key `deep:global`).
pub async fn run_resolver_cycle(trigger: DreamTrigger) -> ResolverReport {
    let started = Instant::now();

    let cfg = crate::config::cached_config().dreaming.clone();
    if !cfg.enabled {
        return ResolverReport::skipped("dreaming disabled in config", started);
    }
    // Honour the manual-trigger switch on the backend too — hiding the UI button
    // isn't authorization; HTTP / Tauri callers must respect it as well (mirrors
    // `pipeline::run_cycle`). The resolver only ever runs manual today.
    if matches!(trigger, DreamTrigger::Manual) && !cfg.manual_enabled {
        return ResolverReport::skipped("manual trigger disabled in config", started);
    }

    let Some(_guard) = try_claim() else {
        return ResolverReport::skipped("another dreaming cycle is already running", started);
    };

    let phase = DreamPhase::Deep;
    let lock_key = format!("{}:global", phase.as_str());
    let run_id = uuid::Uuid::new_v4().to_string();
    let lease_ttl = store::lease_ttl_secs(cfg.narrative_timeout_secs);
    let Some(_lease) = store::acquire_lease(&lock_key, &run_id, lease_ttl) else {
        return ResolverReport::skipped("another instance holds the dreaming lease", started);
    };

    if let Some(s) = store::store() {
        let scope_json = json!({ "phase": "deep", "resolver": true }).to_string();
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
        "dreaming::resolver",
        "deep resolver started (run={}, trigger={})",
        run_id,
        trigger.as_str()
    );

    // 1. Load every active claim off the async runtime.
    let claims = tokio::task::spawn_blocking(|| {
        claims::list_active_claims_for_resolve().unwrap_or_default()
    })
    .await
    .unwrap_or_default();

    // 2. Deterministic expiry, then conflict groups over what survives.
    let now = now_rfc3339();
    let exp_decisions = plan_expirations(&claims, &now);
    let expiring: HashSet<String> = exp_decisions.iter().map(|d| d.claim_id.clone()).collect();
    let mut groups = group_conflicts(&claims, &expiring);

    // Bound per-run LLM work: process at most MAX_RESOLVER_GROUPS this pass; the
    // rest carry to the next run (the active set shrinks each pass).
    let total_groups = groups.len();
    let truncated = total_groups > MAX_RESOLVER_GROUPS;
    if truncated {
        groups.truncate(MAX_RESOLVER_GROUPS);
    }

    // 3. LLM per group (only build the agent if there's a group to judge).
    let mut group_decisions: Vec<ResolverDecision> = Vec::new();
    if !groups.is_empty() {
        let chain = super::pipeline::resolve_dreaming_chain(&cfg);
        if chain.is_empty() {
            app_warn!(
                "memory",
                "dreaming::resolver",
                "no automation model configured for conflict resolution"
            );
        } else {
            for g in &groups {
                group_decisions.extend(analyze_group(&chain, g).await);
            }
        }
    }

    // 4. Apply everything off the async runtime.
    let mut all_decisions = exp_decisions;
    all_decisions.extend(group_decisions);
    let run_for_apply = run_id.clone();
    let applied =
        tokio::task::spawn_blocking(move || apply_decisions(&run_for_apply, &all_decisions))
            .await
            .unwrap_or_default();

    let duration_ms = started.elapsed().as_millis() as u64;
    let note = truncated.then(|| {
        format!(
            "processed {MAX_RESOLVER_GROUPS} of {total_groups} conflict groups; rerun to continue"
        )
    });
    if let Some(s) = store::store() {
        if let Err(e) = s.finish_resolver_run(
            &run_id,
            DreamRunStatus::Completed,
            claims.len(),
            applied.total(),
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
                "phase": "deep",
                "scanned": claims.len(),
                "expired": applied.expired,
                "merged": applied.merged,
                "needsReview": applied.needs_review,
                "durationMs": duration_ms,
            }),
        );
    }

    app_info!(
        "memory",
        "dreaming::resolver",
        "deep resolver done (run={}, scanned={}, expired={}, merged={}, needs_review={}, duration={}ms)",
        run_id,
        claims.len(),
        applied.expired,
        applied.merged,
        applied.needs_review,
        duration_ms
    );

    ResolverReport {
        run_id: Some(run_id),
        scanned: claims.len(),
        expired: applied.expired,
        merged: applied.merged,
        needs_review: applied.needs_review,
        duration_ms,
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claim(
        id: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        valid_until: Option<&str>,
    ) -> ResolveClaim {
        ResolveClaim {
            id: id.to_string(),
            scope_type: "global".to_string(),
            scope_id: None,
            claim_type: "preference".to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            content: format!("{subject} {predicate} {object}"),
            confidence: 0.5,
            confidence_source: "derived".to_string(),
            salience: 0.5,
            valid_until: valid_until.map(|s| s.to_string()),
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            updated_at: "2026-01-01T00:00:00.000Z".to_string(),
        }
    }

    #[test]
    fn plan_expirations_flags_only_past_valid_until() {
        let now = "2026-06-07T00:00:00.000Z";
        let claims = [
            claim(
                "a",
                "user",
                "visits",
                "singapore",
                Some("2026-01-01T00:00:00.000Z"),
            ), // past
            claim(
                "b",
                "user",
                "visits",
                "tokyo",
                Some("2027-01-01T00:00:00.000Z"),
            ), // future
            claim("c", "user", "prefers", "tea", None), // evergreen
            claim("d", "user", "visits", "berlin", Some("")), // empty
        ];
        let decisions = plan_expirations(&claims, now);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].claim_id, "a");
        assert_eq!(decisions[0].decision_type, ResolverDecisionType::Expire);
    }

    #[test]
    fn group_conflicts_needs_multiple_distinct_objects() {
        let expiring = HashSet::new();
        // Same SPO group with two distinct objects → a conflict group.
        let conflicting = [
            claim("a", "user", "uses", "pnpm", None),
            claim("b", "user", "uses", "bun", None),
        ];
        assert_eq!(group_conflicts(&conflicting, &expiring).len(), 1);

        // Same object (just casing/space) → NOT a group (Light's job).
        let dups = [
            claim("a", "user", "uses", "Bun", None),
            claim("b", "user", "uses", "  bun ", None),
        ];
        assert_eq!(group_conflicts(&dups, &expiring).len(), 0);

        // Singleton → no group.
        let single = [claim("a", "user", "uses", "pnpm", None)];
        assert_eq!(group_conflicts(&single, &expiring).len(), 0);
    }

    #[test]
    fn group_conflicts_excludes_expiring_members() {
        let mut expiring = HashSet::new();
        expiring.insert("b".to_string());
        // b is expiring → only a left in the group → no longer a conflict group.
        let claims = [
            claim("a", "user", "uses", "pnpm", None),
            claim("b", "user", "uses", "bun", Some("2020-01-01T00:00:00.000Z")),
        ];
        assert_eq!(group_conflicts(&claims, &expiring).len(), 0);
    }

    #[test]
    fn duplicates_verdict_merges_into_keep() {
        let g = [
            claim("a", "user", "uses", "pnpm", None),
            claim("b", "user", "uses", "pnpm package manager", None),
        ];
        let refs: Vec<&ResolveClaim> = g.iter().collect();
        let v = GroupVerdict {
            relation: "duplicates".to_string(),
            keep_id: Some("a".to_string()),
            rationale: Some("same fact".to_string()),
        };
        let decisions = map_verdict_to_decisions(&refs, &v);
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].claim_id, "b");
        assert_eq!(decisions[0].decision_type, ResolverDecisionType::Merge);
        assert_eq!(decisions[0].merge_into.as_deref(), Some("a"));
    }

    #[test]
    fn duplicates_with_bad_keep_id_falls_back_to_best() {
        let mut a = claim("a", "user", "uses", "pnpm", None);
        a.confidence = 0.9;
        let b = claim("b", "user", "uses", "pnpm pm", None);
        let g = [a, b];
        let refs: Vec<&ResolveClaim> = g.iter().collect();
        let v = GroupVerdict {
            relation: "duplicates".to_string(),
            keep_id: Some("nonexistent".to_string()),
            rationale: None,
        };
        let decisions = map_verdict_to_decisions(&refs, &v);
        // Highest-confidence "a" is kept; "b" merges into it.
        assert_eq!(decisions.len(), 1);
        assert_eq!(decisions[0].claim_id, "b");
        assert_eq!(decisions[0].merge_into.as_deref(), Some("a"));
    }

    #[test]
    fn conflict_verdict_marks_all_needs_review() {
        let g = [
            claim("a", "user", "uses", "pnpm", None),
            claim("b", "user", "uses", "bun", None),
        ];
        let refs: Vec<&ResolveClaim> = g.iter().collect();
        let v = GroupVerdict {
            relation: "conflict".to_string(),
            keep_id: None,
            rationale: Some("contradictory".to_string()),
        };
        let decisions = map_verdict_to_decisions(&refs, &v);
        assert_eq!(decisions.len(), 2);
        assert!(decisions
            .iter()
            .all(|d| d.decision_type == ResolverDecisionType::NeedsReview));
    }

    #[test]
    fn independent_verdict_is_noop() {
        let g = [
            claim("a", "user", "uses", "pnpm", None),
            claim("b", "user", "uses", "bun", None),
        ];
        let refs: Vec<&ResolveClaim> = g.iter().collect();
        let v = GroupVerdict {
            relation: "independent".to_string(),
            keep_id: None,
            rationale: None,
        };
        assert!(map_verdict_to_decisions(&refs, &v).is_empty());
    }
}
