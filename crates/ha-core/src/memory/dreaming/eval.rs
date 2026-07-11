//! Deterministic golden-fixture eval harness (design §9, PR #10).
//!
//! The next-gen Dreaming pipeline must be guarded by an offline eval, not by
//! feel. Evaluation is layered (design §9.3) to keep LLM non-determinism out of
//! CI: this module is the **deterministic layer** — it seeds known claim /
//! memory states and asserts the safety red-lines from §9.2 against the REAL
//! read paths (scope filter, effective-status / stale suppression, evidence
//! coverage, conflict→review, legacy-sync hidden-set, evidence fail-closed). No
//! LLM is involved, so it runs in the default CI suite. Claim extraction /
//! profile synthesis (which need a fixed model or mock) are the separate
//! "Golden LLM fixtures" layer (manual / nightly) and are out of scope here.
//!
//! Fixtures live in `crates/ha-core/tests/fixtures/dreaming/*.json` and are
//! driven by the `tests/dreaming_eval.rs` integration test, which initialises
//! the claim store global once (process-isolated) and calls [`evaluate`] per
//! fixture. Each fixture confines its seeds to a unique scope namespace, so a
//! single shared DB has no cross-fixture interference.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use crate::memory::claims::{
    self, ClaimCandidate, ClaimListFilter, ClaimScopeHint, ClaimTemporal, ResolveClaim,
};
use crate::memory::{MemoryBackend, MemoryScope, MemoryType, NewMemory};

// A claim past this fixed instant reads as effective-expired; a claim valid
// until this one never does. Both are far outside any real CI clock, so the
// lexical `valid_until` compare is deterministic regardless of when CI runs.
const PAST: &str = "2000-01-01T00:00:00.000Z";
const FUTURE: &str = "2999-01-01T00:00:00.000Z";
const EVAL_NOW: &str = "2026-06-07T00:00:00.000Z";

/// `{ "type": "global" | "agent" | "project", "id"?: "..." }`.
#[derive(Debug, Clone, Deserialize)]
pub struct ScopeSpec {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub id: Option<String>,
}

impl ScopeSpec {
    fn to_scope(&self) -> Result<MemoryScope> {
        match self.kind.as_str() {
            "global" => Ok(MemoryScope::Global),
            "agent" => Ok(MemoryScope::Agent {
                id: self
                    .id
                    .clone()
                    .ok_or_else(|| anyhow!("agent scope requires id"))?,
            }),
            "project" => Ok(MemoryScope::Project {
                id: self
                    .id
                    .clone()
                    .ok_or_else(|| anyhow!("project scope requires id"))?,
            }),
            other => Err(anyhow!("unknown scope type: {other}")),
        }
    }

    fn matches_resolve_claim(&self, claim: &ResolveClaim) -> bool {
        match self.kind.as_str() {
            "global" => {
                claim.scope_type == "global" && claim.scope_id.as_deref().unwrap_or("").is_empty()
            }
            "agent" => {
                claim.scope_type == "agent" && claim.scope_id.as_deref() == self.id.as_deref()
            }
            "project" => {
                claim.scope_type == "project" && claim.scope_id.as_deref() == self.id.as_deref()
            }
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedClaim {
    /// Local handle for cross-references in checks / links.
    pub key: String,
    pub scope: ScopeSpec,
    pub claim_type: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub content: String,
    #[serde(default)]
    pub evidence_class: Option<String>,
    #[serde(default)]
    pub salience: Option<f32>,
    /// `"past"` | `"future"` | literal RFC3339 | absent.
    #[serde(default)]
    pub valid_until: Option<String>,
    /// Post-write transition: `needs_review` | `expired` | `archived`.
    #[serde(default)]
    pub set_status: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedMemory {
    pub key: String,
    pub scope: ScopeSpec,
    pub content: String,
    #[serde(default)]
    pub pinned: bool,
}

fn default_sync() -> String {
    "managed".to_string()
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeedLink {
    pub claim: String,
    pub memory: String,
    #[serde(default = "default_sync")]
    pub sync_mode: String,
}

fn default_active() -> String {
    "active".to_string()
}

/// List the claims of a given effective status in a scope and assert membership
/// + content rules. `status` defaults to `active`; set it to `needs_review`
/// (etc.) to assert the review-queue side of a transition.
#[derive(Debug, Clone, Deserialize)]
pub struct ListActiveCheck {
    pub scope: ScopeSpec,
    #[serde(default = "default_active")]
    pub status: String,
    #[serde(default)]
    pub expect_present: Vec<String>,
    #[serde(default)]
    pub expect_absent: Vec<String>,
    /// Substrings that must NOT appear in any claim of this (scope, status)
    /// (scope-leakage / hold-out guard).
    #[serde(default)]
    pub forbidden_content: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct StatusCheck {
    pub claim: String,
    /// Expected EFFECTIVE status (read API maps `valid_until`-expired → expired).
    pub expect: String,
}

fn default_one() -> usize {
    1
}

fn default_resolver_group_cap() -> usize {
    8
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvidenceCheck {
    pub claim: String,
    #[serde(default = "default_one")]
    pub min: usize,
}

/// Assert the prompt-injection candidate set (legacy memories) for a scope.
#[derive(Debug, Clone, Deserialize)]
pub struct InjectableCheck {
    pub agent_id: String,
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub shared: bool,
    /// Memory keys that must NOT be injectable (hidden by a dead/expired claim).
    #[serde(default)]
    pub expect_absent: Vec<String>,
    #[serde(default)]
    pub expect_present: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EvidenceQuoteCheck {
    pub session_id: String,
    #[serde(default)]
    pub message_id: Option<i64>,
    /// Expected `available` flag (false = correctly fail-closed).
    pub expect_available: bool,
}

/// Assert the deterministic auto-expire subset of Deep Resolver. This checks
/// planning only: no expired claims means no Deep audit run; present expired
/// claims may only produce `Expire` decisions.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoExpirePlanCheck {
    pub scope: ScopeSpec,
    #[serde(default)]
    pub expect_expire: Vec<String>,
    #[serde(default)]
    pub expect_absent: Vec<String>,
    #[serde(default)]
    pub expect_no_run: bool,
}

/// Assert graph-first automatic resolver routing without invoking an LLM.
/// Claim keys are grouped exactly as the production planner sees them after
/// deterministic expiry has been removed.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoResolverGraphPlanCheck {
    pub scope: ScopeSpec,
    #[serde(default = "default_resolver_group_cap")]
    pub group_cap: usize,
    #[serde(default)]
    pub expect_llm_groups: Vec<Vec<String>>,
    #[serde(default)]
    pub expect_graph_noop_groups: Vec<Vec<String>>,
    #[serde(default)]
    pub expect_truncated: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct FixtureChecks {
    #[serde(default)]
    pub list_active: Vec<ListActiveCheck>,
    #[serde(default)]
    pub status: Vec<StatusCheck>,
    #[serde(default)]
    pub evidence: Vec<EvidenceCheck>,
    #[serde(default)]
    pub injectable: Vec<InjectableCheck>,
    #[serde(default)]
    pub evidence_quote: Vec<EvidenceQuoteCheck>,
    #[serde(default)]
    pub auto_expire_plan: Vec<AutoExpirePlanCheck>,
    #[serde(default)]
    pub auto_resolver_graph_plan: Vec<AutoResolverGraphPlanCheck>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DreamingFixture {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub claims: Vec<SeedClaim>,
    #[serde(default)]
    pub memories: Vec<SeedMemory>,
    #[serde(default)]
    pub links: Vec<SeedLink>,
    pub checks: FixtureChecks,
}

/// One assertion's result.
#[derive(Debug, Clone)]
pub struct CheckOutcome {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}

/// All assertions for one fixture.
#[derive(Debug, Clone)]
pub struct FixtureReport {
    pub name: String,
    pub outcomes: Vec<CheckOutcome>,
}

impl FixtureReport {
    pub fn passed(&self) -> bool {
        self.outcomes.iter().all(|o| o.passed)
    }
    pub fn failures(&self) -> Vec<&CheckOutcome> {
        self.outcomes.iter().filter(|o| !o.passed).collect()
    }
}

/// Directory holding the golden fixtures (`tests/fixtures/dreaming/`).
pub fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dreaming")
}

/// Load + parse every `*.json` fixture, sorted by filename for stable order.
pub fn load_fixtures() -> Result<Vec<DreamingFixture>> {
    let dir = fixtures_dir();
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .with_context(|| format!("reading fixtures dir {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        let raw = std::fs::read_to_string(&p)
            .with_context(|| format!("reading fixture {}", p.display()))?;
        let fx: DreamingFixture = serde_json::from_str(&raw)
            .with_context(|| format!("parsing fixture {}", p.display()))?;
        out.push(fx);
    }
    Ok(out)
}

fn resolve_valid_until(raw: &str) -> String {
    match raw {
        "past" => PAST.to_string(),
        "future" => FUTURE.to_string(),
        other => other.to_string(),
    }
}

/// Seed a fixture's claims / memories / links, then run its checks against the
/// real read paths. Requires the claim store global to be initialised with the
/// same backend (`init_claim_store`) — the integration test does this once.
pub fn evaluate(backend: &dyn MemoryBackend, fx: &DreamingFixture) -> Result<FixtureReport> {
    // ── Seed claims ──
    let mut claim_ids: HashMap<String, String> = HashMap::new();
    for sc in &fx.claims {
        let scope = sc.scope.to_scope()?;
        let candidate = ClaimCandidate {
            claim_type: sc.claim_type.clone(),
            subject: sc.subject.clone(),
            predicate: sc.predicate.clone(),
            object: sc.object.clone(),
            content: sc.content.clone(),
            // `None` → the write path uses the extraction scope (our `scope`),
            // never a model hint (mirrors production).
            scope: None::<ClaimScopeHint>,
            evidence_class: sc.evidence_class.clone(),
            salience: sc.salience,
            temporal: sc.valid_until.as_ref().map(|v| ClaimTemporal {
                valid_from: None,
                valid_until: Some(resolve_valid_until(v)),
            }),
            evidence_refs: Vec::new(),
            tags: sc.tags.clone(),
        };
        let outcome = claims::write_claim_candidate(&candidate, &scope, "eval-session", None)
            .with_context(|| format!("seeding claim {}", sc.key))?;
        let id = outcome.claim_id;
        match sc.set_status.as_deref() {
            Some("needs_review") => {
                claims::mark_claim_needs_review(&id)?;
            }
            Some("expired") => {
                claims::expire_claim(&id)?;
            }
            Some("archived") => {
                claims::forget_claim(&id, false, None)?;
            }
            Some(other) => return Err(anyhow!("unsupported set_status: {other}")),
            None => {}
        }
        claim_ids.insert(sc.key.clone(), id);
    }

    // ── Seed memories + links ──
    let mut mem_ids: HashMap<String, i64> = HashMap::new();
    for sm in &fx.memories {
        let id = backend.add(NewMemory {
            memory_type: MemoryType::User,
            scope: sm.scope.to_scope()?,
            content: sm.content.clone(),
            tags: Vec::new(),
            source: "eval".to_string(),
            source_session_id: None,
            pinned: sm.pinned,
            attachment_path: None,
            attachment_mime: None,
        })?;
        mem_ids.insert(sm.key.clone(), id);
    }
    for ln in &fx.links {
        let cid = claim_ids
            .get(&ln.claim)
            .ok_or_else(|| anyhow!("link references unknown claim key {}", ln.claim))?;
        let mid = mem_ids
            .get(&ln.memory)
            .ok_or_else(|| anyhow!("link references unknown memory key {}", ln.memory))?;
        claims::link_claim_memory(cid, *mid, &ln.sync_mode)?;
    }

    // ── Run checks ──
    let mut outcomes = Vec::new();
    let claim_key_of = |id: &str| -> Option<&str> {
        claim_ids
            .iter()
            .find(|(_, v)| v.as_str() == id)
            .map(|(k, _)| k.as_str())
    };

    for c in &fx.checks.list_active {
        let scope = c.scope.to_scope()?;
        let list = claims::list_claims(ClaimListFilter {
            scope: Some(scope),
            status: Some(c.status.clone()),
            claim_type: None,
            confidence_source: None,
            evidence_class: None,
            evidence_source_type: None,
            query: None,
            sort: None,
            limit: Some(500),
            offset: None,
        })?;
        let present_keys: Vec<&str> = list.iter().filter_map(|r| claim_key_of(&r.id)).collect();
        for k in &c.expect_present {
            let ok = present_keys.contains(&k.as_str());
            outcomes.push(CheckOutcome {
                name: format!("list[{}] present {k}", c.status),
                passed: ok,
                detail: if ok {
                    String::new()
                } else {
                    format!("expected {k} in {} scope, got {present_keys:?}", c.status)
                },
            });
        }
        for k in &c.expect_absent {
            let absent = !present_keys.contains(&k.as_str());
            outcomes.push(CheckOutcome {
                name: format!("list[{}] absent {k}", c.status),
                passed: absent,
                detail: if absent {
                    String::new()
                } else {
                    format!("{k} leaked into {} scope list", c.status)
                },
            });
        }
        for needle in &c.forbidden_content {
            let leaked = list.iter().any(|r| r.content.contains(needle));
            outcomes.push(CheckOutcome {
                name: format!("list[{}] forbidden_content {needle:?}", c.status),
                passed: !leaked,
                detail: if leaked {
                    format!(
                        "forbidden content {needle:?} leaked into {} scope",
                        c.status
                    )
                } else {
                    String::new()
                },
            });
        }
    }

    for c in &fx.checks.status {
        let id = claim_ids
            .get(&c.claim)
            .ok_or_else(|| anyhow!("status check references unknown claim {}", c.claim))?;
        let got = claims::get_claim(id)?.map(|d| d.claim.status);
        let passed = got.as_deref() == Some(c.expect.as_str());
        outcomes.push(CheckOutcome {
            name: format!("status {}={}", c.claim, c.expect),
            passed,
            detail: if passed {
                String::new()
            } else {
                format!("expected {}, got {:?}", c.expect, got)
            },
        });
    }

    for c in &fx.checks.evidence {
        let id = claim_ids
            .get(&c.claim)
            .ok_or_else(|| anyhow!("evidence check references unknown claim {}", c.claim))?;
        let count = claims::get_claim(id)?
            .map(|d| d.evidence.len())
            .unwrap_or(0);
        let passed = count >= c.min;
        outcomes.push(CheckOutcome {
            name: format!("evidence {}>={}", c.claim, c.min),
            passed,
            detail: if passed {
                String::new()
            } else {
                format!("claim {} has {count} evidence, need {}", c.claim, c.min)
            },
        });
    }

    for c in &fx.checks.injectable {
        let candidates = backend.load_prompt_candidates_with_project(
            &c.agent_id,
            c.project_id.as_deref(),
            c.shared,
        )?;
        let ids: Vec<i64> = candidates.iter().map(|m| m.id).collect();
        for k in &c.expect_absent {
            let mid = mem_ids
                .get(k)
                .ok_or_else(|| anyhow!("injectable check references unknown memory {k}"))?;
            let absent = !ids.contains(mid);
            outcomes.push(CheckOutcome {
                name: format!("injectable absent {k}"),
                passed: absent,
                detail: if absent {
                    String::new()
                } else {
                    format!("memory {k} (#{mid}) still injectable despite dead claim")
                },
            });
        }
        for k in &c.expect_present {
            let mid = mem_ids
                .get(k)
                .ok_or_else(|| anyhow!("injectable check references unknown memory {k}"))?;
            let present = ids.contains(mid);
            outcomes.push(CheckOutcome {
                name: format!("injectable present {k}"),
                passed: present,
                detail: if present {
                    String::new()
                } else {
                    format!("memory {k} (#{mid}) unexpectedly hidden")
                },
            });
        }
    }

    for c in &fx.checks.evidence_quote {
        let q = crate::memory::dreaming::evidence_quote(&c.session_id, c.message_id);
        let passed = q.available == c.expect_available;
        outcomes.push(CheckOutcome {
            name: format!("evidence_quote available={}", c.expect_available),
            passed,
            detail: if passed {
                String::new()
            } else {
                format!(
                    "expected available={}, got available={} (reason {:?})",
                    c.expect_available, q.available, q.reason
                )
            },
        });
    }

    for c in &fx.checks.auto_expire_plan {
        let all = claims::list_active_claims_for_resolve()?;
        let scoped: Vec<ResolveClaim> = all
            .into_iter()
            .filter(|claim| c.scope.matches_resolve_claim(claim))
            .collect();
        let planned = super::resolver::plan_auto_expiration_sweep(&scoped, EVAL_NOW);
        let planned_keys: Vec<&str> = planned
            .as_ref()
            .into_iter()
            .flatten()
            .filter_map(|d| claim_key_of(&d.claim_id))
            .collect();

        outcomes.push(CheckOutcome {
            name: "auto_expire no_run".to_string(),
            passed: c.expect_no_run == planned.is_none(),
            detail: if c.expect_no_run == planned.is_none() {
                String::new()
            } else {
                format!(
                    "expect_no_run={}, planned={planned_keys:?}",
                    c.expect_no_run
                )
            },
        });

        for k in &c.expect_expire {
            let present = planned_keys.contains(&k.as_str());
            outcomes.push(CheckOutcome {
                name: format!("auto_expire plans {k}"),
                passed: present,
                detail: if present {
                    String::new()
                } else {
                    format!("expected {k} in auto-expire plan, got {planned_keys:?}")
                },
            });
        }

        for k in &c.expect_absent {
            let absent = !planned_keys.contains(&k.as_str());
            outcomes.push(CheckOutcome {
                name: format!("auto_expire omits {k}"),
                passed: absent,
                detail: if absent {
                    String::new()
                } else {
                    format!("{k} unexpectedly entered auto-expire plan")
                },
            });
        }
    }

    for c in &fx.checks.auto_resolver_graph_plan {
        let all = claims::list_active_claims_for_resolve()?;
        let scoped: Vec<ResolveClaim> = all
            .into_iter()
            .filter(|claim| c.scope.matches_resolve_claim(claim))
            .collect();
        let expiring = super::resolver::plan_auto_expiration_sweep(&scoped, EVAL_NOW)
            .unwrap_or_default()
            .into_iter()
            .map(|decision| decision.claim_id)
            .collect::<std::collections::HashSet<_>>();
        let plan = super::resolver::plan_auto_resolution_groups(&scoped, &expiring, c.group_cap);

        let normalize_groups = |groups: &[Vec<String>]| {
            let mut normalized = groups
                .iter()
                .map(|group| {
                    let mut keys = group
                        .iter()
                        .map(|id| claim_key_of(id).unwrap_or(id.as_str()).to_string())
                        .collect::<Vec<_>>();
                    keys.sort();
                    keys
                })
                .collect::<Vec<_>>();
            normalized.sort();
            normalized
        };
        let normalize_expected = |groups: &[Vec<String>]| {
            let mut normalized = groups.to_vec();
            for group in &mut normalized {
                group.sort();
            }
            normalized.sort();
            normalized
        };
        let llm_groups = normalize_groups(&plan.llm_group_ids);
        let graph_noop_groups = normalize_groups(&plan.graph_noop_group_ids);
        let expected_llm = normalize_expected(&c.expect_llm_groups);
        let expected_graph_noop = normalize_expected(&c.expect_graph_noop_groups);

        outcomes.push(CheckOutcome {
            name: format!("auto_resolver llm_groups cap={}", c.group_cap),
            passed: llm_groups == expected_llm,
            detail: if llm_groups == expected_llm {
                String::new()
            } else {
                format!("expected {expected_llm:?}, got {llm_groups:?}")
            },
        });
        outcomes.push(CheckOutcome {
            name: format!("auto_resolver graph_noop_groups cap={}", c.group_cap),
            passed: graph_noop_groups == expected_graph_noop,
            detail: if graph_noop_groups == expected_graph_noop {
                String::new()
            } else {
                format!("expected {expected_graph_noop:?}, got {graph_noop_groups:?}")
            },
        });
        outcomes.push(CheckOutcome {
            name: format!("auto_resolver truncated={}", c.expect_truncated),
            passed: plan.truncated == c.expect_truncated,
            detail: if plan.truncated == c.expect_truncated {
                String::new()
            } else {
                format!(
                    "expected truncated={}, got {}",
                    c.expect_truncated, plan.truncated
                )
            },
        });
    }

    Ok(FixtureReport {
        name: fx.name.clone(),
        outcomes,
    })
}
