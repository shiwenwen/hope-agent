//! Draft-skill consolidation pass.
//!
//! Walks the managed skills directory, finds clusters of `status: draft`
//! skills whose descriptions and body excerpts are topically close
//! (Jaccard ≥ threshold), and surfaces them to the user as merge
//! proposals. The user decides whether to apply — we never silently
//! collapse drafts.
//!
//! Apply path (`apply_merge_keep_id`) keeps a chosen "winner" and
//! discards the rest. The discarded ids land on the auto-review pipeline's
//! recent-discards blacklist (`delete_skill` → `learning_events`) so the
//! same near-duplicate can't immediately come back through gate 2.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::skills::author::delete_skill;
use crate::skills::{load_all_skills_with_extra, SkillEntry, SkillStatus};
use crate::truncate_utf8;

use super::heuristics::{jaccard, tokenize};

/// Minimum Jaccard similarity between two drafts' (description + body
/// excerpt) bags to call them topical duplicates. Picked conservatively
/// — false-positive merges feel worse than a missed cluster.
pub const CLUSTER_THRESHOLD: f32 = 0.4;

pub const EVENT_CURATOR_PROPOSALS_READY: &str = "skills:curator_proposals_ready";

/// Body chars per draft fed into the Jaccard bag. Big enough to capture
/// the meat of a draft, small enough that 50+ drafts don't blow tokens.
const BODY_EXCERPT_BYTES: usize = 500;

/// Maximum cluster size to surface. Beyond this we'd be guessing.
const MAX_CLUSTER_SIZE: usize = 6;

static AUTO_CURATOR_LOOP_SPAWNED: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterMember {
    pub skill_id: String,
    pub description: String,
    /// Pairwise Jaccard against the seed draft of this cluster.
    pub similarity_to_seed: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeProposal {
    /// Unique identifier for this proposal (`cluster-<seed>-<count>`).
    pub id: String,
    /// Lowest similarity in the cluster — a proxy for "how confident
    /// are we?".
    pub min_similarity: f32,
    /// Members of the cluster, seed first. UI lets the user pick which
    /// id to keep; the rest will be discarded on apply.
    pub members: Vec<ClusterMember>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CuratorReport {
    pub proposals: Vec<MergeProposal>,
    /// Total drafts scanned. Lets the UI render "x of N drafts are
    /// flagged as duplicates" without a second round-trip.
    pub drafts_scanned: usize,
}

/// Spawn the optional auto-curator loop. Idempotent and Primary-only by
/// caller convention (`app_init::start_background_tasks`), but guarded here
/// too so dev hot reloads cannot duplicate the timer in one process.
pub fn spawn_auto_curator_loop() {
    if AUTO_CURATOR_LOOP_SPAWNED.swap(true, Ordering::SeqCst) {
        return;
    }

    let notify = Arc::new(Notify::new());

    if let Some(bus) = crate::get_event_bus() {
        let mut rx = bus.subscribe();
        let notify_for_sub = notify.clone();
        tokio::spawn(async move {
            while let Ok(evt) = rx.recv().await {
                if evt.name == "config:changed" {
                    notify_for_sub.notify_one();
                }
            }
        });
    } else {
        app_warn!(
            "skills",
            "auto_curator",
            "EventBus not initialized — auto-curator will not react to live config edits"
        );
    }

    tokio::spawn(async move {
        let mut last_run: Option<Instant> = None;

        loop {
            let cfg = crate::config::cached_config()
                .skills
                .auto_review
                .clone()
                .sanitize();

            if !cfg.enabled || !cfg.auto_curator_enabled {
                notify.notified().await;
                continue;
            }

            let interval =
                Duration::from_secs(cfg.auto_curator_interval_days * crate::SECS_PER_DAY);
            let now = Instant::now();
            let elapsed = last_run.map(|t| now.saturating_duration_since(t));

            if elapsed.is_none_or(|e| e >= interval) {
                last_run = Some(now);
                run_auto_curator_pass().await;
                continue;
            }

            let wait = interval.saturating_sub(elapsed.unwrap_or_default());
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = notify.notified() => {}
            }
        }
    });
}

async fn run_auto_curator_pass() {
    let started = Instant::now();
    match tokio::task::spawn_blocking(run_curator_pass).await {
        Ok(Ok(report)) => {
            let proposals = report.proposals.len();
            let drafts_scanned = report.drafts_scanned;
            if let Some(bus) = crate::get_event_bus() {
                bus.emit(
                    EVENT_CURATOR_PROPOSALS_READY,
                    serde_json::to_value(&report).unwrap_or(Value::Null),
                );
            }
            app_info!(
                "skills",
                "auto_curator",
                "auto-curator pass completed: proposals={}, drafts_scanned={}, duration_ms={}",
                proposals,
                drafts_scanned,
                started.elapsed().as_millis()
            );
        }
        Ok(Err(e)) => app_warn!("skills", "auto_curator", "auto-curator pass failed: {}", e),
        Err(e) => app_warn!(
            "skills",
            "auto_curator",
            "auto-curator task join failed: {}",
            e
        ),
    }
}

/// Run a single curator scan. No LLM calls, no disk writes — pure read
/// of the managed skills tree. Suitable for both the manual button
/// (`run_skills_curator_now`) and the optional periodic task.
pub fn run_curator_pass() -> Result<CuratorReport> {
    let config = crate::config::cached_config();
    let entries: Vec<SkillEntry> = load_all_skills_with_extra(&config.extra_skills_dirs)
        .into_iter()
        // Only managed (~/.hope-agent/skills/) drafts; we never touch
        // bundled / project / extra-dir skills via the curator.
        .filter(|s| s.source == "managed" && s.status == SkillStatus::Draft)
        .collect();

    if entries.len() < 2 {
        return Ok(CuratorReport {
            proposals: Vec::new(),
            drafts_scanned: entries.len(),
        });
    }

    // Build a (skill_id, description, tokenset) tuple for each draft.
    // Read each SKILL.md directly via the prebuilt `entry.file_path` —
    // `get_skill_content` would re-scan the whole skill tree per call.
    let mut indexed: Vec<(String, String, std::collections::HashSet<String>)> = Vec::new();
    for entry in &entries {
        let body = std::fs::read_to_string(&entry.file_path).unwrap_or_default();
        let mut hay = String::new();
        hay.push_str(&entry.description);
        hay.push(' ');
        hay.push_str(truncate_utf8(&body, BODY_EXCERPT_BYTES));
        indexed.push((
            entry.name.clone(),
            entry.description.clone(),
            tokenize(&hay),
        ));
    }

    // Greedy clustering: seed = first un-claimed draft; pull in anything
    // with Jaccard ≥ threshold against the seed. O(N²) for N drafts is
    // fine — managed drafts top out at "tens" in practice.
    let mut claimed = vec![false; indexed.len()];
    let mut proposals = Vec::new();

    for i in 0..indexed.len() {
        if claimed[i] {
            continue;
        }
        let mut members = vec![ClusterMember {
            skill_id: indexed[i].0.clone(),
            description: indexed[i].1.clone(),
            similarity_to_seed: 1.0,
        }];
        let mut min_sim = 1.0f32;
        for j in (i + 1)..indexed.len() {
            if claimed[j] {
                continue;
            }
            let s = jaccard(&indexed[i].2, &indexed[j].2);
            if s >= CLUSTER_THRESHOLD {
                claimed[j] = true;
                if s < min_sim {
                    min_sim = s;
                }
                members.push(ClusterMember {
                    skill_id: indexed[j].0.clone(),
                    description: indexed[j].1.clone(),
                    similarity_to_seed: s,
                });
                if members.len() >= MAX_CLUSTER_SIZE {
                    break;
                }
            }
        }
        if members.len() >= 2 {
            claimed[i] = true;
            proposals.push(MergeProposal {
                id: format!("cluster-{}-{}", indexed[i].0, members.len()),
                min_similarity: min_sim,
                members,
            });
        }
    }

    Ok(CuratorReport {
        proposals,
        drafts_scanned: entries.len(),
    })
}

/// Apply a merge by keeping `keep_id` and deleting every other id in
/// `member_ids`. Validates that `keep_id` is one of the listed members
/// and that all involved skills are still drafts before doing anything
/// destructive — between scan and apply the user might have already
/// activated or hand-deleted one.
pub fn apply_merge_keep_id(keep_id: &str, member_ids: &[String]) -> Result<usize> {
    if !member_ids.iter().any(|id| id == keep_id) {
        return Err(anyhow!("keep_id `{}` is not in the member list", keep_id));
    }
    let config = crate::config::cached_config();
    let entries = load_all_skills_with_extra(&config.extra_skills_dirs);

    let is_managed_draft = |id: &str| -> bool {
        entries
            .iter()
            .any(|e| e.name == id && e.source == "managed" && e.status == SkillStatus::Draft)
    };

    // Refuse to apply the merge if the user already activated or
    // hand-deleted `keep_id` between scan and apply — otherwise we'd
    // happily delete the other members and end the merge with nothing
    // retained. The UI should re-run the scan and let the user pick a
    // new winner.
    if !is_managed_draft(keep_id) {
        return Err(anyhow!(
            "keep_id `{}` is no longer a managed draft; aborting merge — please re-run the scan",
            keep_id
        ));
    }

    let mut discarded = 0usize;
    for id in member_ids {
        if id == keep_id {
            continue;
        }
        if !is_managed_draft(id) {
            app_warn!(
                "skills",
                "curator",
                "skipping merge target {}: no longer a managed draft",
                id
            );
            continue;
        }
        delete_skill(id).with_context(|| format!("delete {}", id))?;
        discarded += 1;
    }
    Ok(discarded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn clusters_two_near_duplicates() {
        let a: HashSet<String> = tokenize("audit rust clippy warnings cargo check workspace");
        let b: HashSet<String> = tokenize("audit rust clippy warnings cargo workspace check");
        assert!(jaccard(&a, &b) >= CLUSTER_THRESHOLD);
    }

    #[test]
    fn does_not_cluster_unrelated() {
        let a: HashSet<String> = tokenize("audit rust clippy warnings cargo workspace");
        let b: HashSet<String> =
            tokenize("draw architecture diagrams from yaml describing system components");
        assert!(jaccard(&a, &b) < CLUSTER_THRESHOLD);
    }

    #[test]
    fn apply_merge_rejects_keep_not_in_members() {
        let r = apply_merge_keep_id("foo", &["bar".to_string()]);
        assert!(r.is_err());
    }
}
