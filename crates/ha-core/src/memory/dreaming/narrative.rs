//! Narrative — LLM-driven promotion + diary generation.
//!
//! A single side_query call per cycle: we give the LLM the candidate
//! list, ask for a JSON array of promotion nominations AND a human-
//! readable diary paragraph. The caller then filters nominations against
//! the configured thresholds and writes the diary to disk.

use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Local;

use crate::automation::{self, ModelTaskSpec};
use crate::memory::MemoryEntry;
use crate::provider::ActiveModel;

use std::collections::HashMap;

use super::config::DreamingConfig;
use super::scanner::{evidence_for_candidate, render_candidates_for_prompt};
use super::scoring::{filter_and_rank, parse_nominations};
use super::types::{EvidenceRef, PromotionRecord};

/// Result of the narrative step. `promotions` is already filtered
/// against `DreamingConfig.promotion.min_score` / `max_promote`.
pub struct NarrativeOutput {
    pub promotions: Vec<PromotionRecord>,
    pub promotions_nominated: usize,
    pub diary_markdown: String,
}

/// Build the full prompt handed to `side_query`. Asks for a JSON envelope
/// so we can parse both the nominations and the diary prose in one shot.
pub fn build_prompt(candidates: &[MemoryEntry], cfg: &DreamingConfig) -> String {
    let candidate_block = render_candidates_for_prompt(candidates);
    let min_score = cfg.promotion.min_score;
    let max_promote = cfg.promotion.max_promote;

    format!(
        "You are the agent's offline memory-consolidation process (\"dreaming\"). \
Review the candidate memories below and decide which are worth promoting \
into pinned core memory. Pinned memories are always injected into the \
system prompt, so be conservative — only promote items that will remain \
useful across many future sessions.\n\n\
Return a single JSON object with exactly these two keys:\n\
  - `promotions`: array of objects `{{id: number, score: number 0-1, \
title: short headline, rationale: one-sentence why-it-matters}}`. Omit \
candidates that should NOT be promoted; don't explain the omissions.\n\
  - `diary`: a short markdown paragraph (2-6 sentences) narrating what \
the user focused on recently, what's being consolidated, and any \
emerging themes. Write it as a first-person reflection from the agent.\n\n\
Hard cutoffs (server applies them after parsing):\n\
  - score threshold: {min_score:.2}\n\
  - max promotions: {max_promote}\n\n\
Only return the JSON object — no code fences, no prose outside JSON.\n\n\
Candidate memories (most recent first):\n\
{candidate_block}\n",
        min_score = min_score,
        max_promote = max_promote,
        candidate_block = candidate_block,
    )
}

/// Execute the side_query and parse both the promotion list and the diary
/// narrative. Applies thresholds server-side.
pub async fn run_side_query(
    chain: Vec<ActiveModel>,
    candidates: &[MemoryEntry],
    cfg: &DreamingConfig,
) -> Result<NarrativeOutput> {
    let prompt = build_prompt(candidates, cfg);
    let result = tokio::time::timeout(
        Duration::from_secs(cfg.narrative_timeout_secs.max(5)),
        automation::run(ModelTaskSpec {
            purpose: "dreaming.narrative",
            chain,
            session_key: "automation:dreaming",
            instruction: &prompt,
            max_tokens: cfg.narrative_max_tokens,
        }),
    )
    .await
    .context("dreaming narrative side_query timed out")?
    .context("dreaming narrative side_query failed")?;

    // Extract the JSON envelope. The LLM sometimes wraps in code fences
    // despite the instruction — parse_nominations already tolerates
    // that for the promotions list, but the diary needs dedicated
    // handling.
    let (promotions_raw, diary) = split_envelope(&result.text);
    let nominated = parse_nominations(&promotions_raw);
    let promotions_nominated = nominated.len();
    let mut promoted = filter_and_rank(
        nominated,
        cfg.promotion.min_score,
        cfg.promotion.max_promote,
    );

    attach_evidence(&mut promoted, candidates);

    Ok(NarrativeOutput {
        promotions: promoted,
        promotions_nominated,
        diary_markdown: diary,
    })
}

/// Attach provenance to each promotion by matching its memory id back to
/// the scanned candidate it came from (Evidence Layer). The LLM only
/// returns ids; the source session lives on the candidate, so the join
/// happens here. A nomination with no matching candidate (shouldn't
/// normally happen) still anchors to its own memory id.
fn attach_evidence(promoted: &mut [PromotionRecord], candidates: &[MemoryEntry]) {
    let by_id: HashMap<i64, &MemoryEntry> = candidates.iter().map(|c| (c.id, c)).collect();
    for p in promoted.iter_mut() {
        p.evidence = match by_id.get(&p.memory_id) {
            Some(candidate) => evidence_for_candidate(candidate),
            None => vec![EvidenceRef::memory(p.memory_id)],
        };
    }
}

/// Render evidence refs into a stable, grep-able anchor for the diary
/// comment, e.g. `memory:42,session:abc123`. Empty refs → empty string.
pub(crate) fn render_evidence_anchor(refs: &[EvidenceRef]) -> String {
    refs.iter()
        .filter_map(|r| match r.source_type.as_str() {
            "memory" => r.memory_id.map(|id| format!("memory:{}", id)),
            "session_message" => r.session_id.as_ref().map(|sid| match r.message_id {
                Some(mid) => format!("session:{}#{}", sid, mid),
                None => format!("session:{}", sid),
            }),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Pull out (promotions_json, diary_markdown) from the LLM response.
/// Defensive: if the envelope doesn't parse, returns (full_response, "").
fn split_envelope(raw: &str) -> (String, String) {
    let trimmed = raw
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim();
    let trimmed = trimmed.trim_end_matches("```").trim();
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(serde_json::Value::Object(map)) => {
            let promotions_json = map
                .get("promotions")
                .map(|v| v.to_string())
                .unwrap_or_else(|| "[]".to_string());
            let diary = map
                .get("diary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            (promotions_json, diary)
        }
        _ => (trimmed.to_string(), String::new()),
    }
}

/// Render the final Dream Diary markdown to write to disk.
/// Includes a `<!-- ha-dream-promotion: ... -->` comment per promotion
/// so tooling can later grep-index which memories were pinned when.
pub fn render_diary_markdown(output: &NarrativeOutput) -> String {
    let date = Local::now().format("%Y-%m-%d %H:%M:%S %Z").to_string();
    let mut md = String::new();
    md.push_str(&format!("# Dream Diary — {}\n\n", date));

    if output.diary_markdown.is_empty() {
        md.push_str("_(No narrative generated.)_\n\n");
    } else {
        md.push_str(&output.diary_markdown);
        if !output.diary_markdown.ends_with('\n') {
            md.push('\n');
        }
        md.push('\n');
    }

    md.push_str(&format!(
        "## Promoted memories ({})\n\n",
        output.promotions.len()
    ));

    if output.promotions.is_empty() {
        md.push_str("_(None this cycle.)_\n");
    } else {
        for p in &output.promotions {
            md.push_str(&format!("### {}\n\n", p.title));
            let evidence_anchor = render_evidence_anchor(&p.evidence);
            md.push_str(&format!(
                "<!-- ha-dream-promotion: memory_id={} score={:.2} evidence={} -->\n",
                p.memory_id, p.score, evidence_anchor
            ));
            if !p.rationale.is_empty() {
                md.push_str(&p.rationale);
                if !p.rationale.ends_with('\n') {
                    md.push('\n');
                }
                md.push('\n');
            }
        }
    }

    md
}

/// Write the diary to `~/.hope-agent/memory/dreams/{timestamp}.md` and
/// return the absolute path.
pub fn write_diary(md: &str) -> Result<std::path::PathBuf> {
    let dir = crate::paths::dreams_dir()?;
    std::fs::create_dir_all(&dir).context("creating dreams_dir")?;
    // Use date + time so multiple cycles in one day don't clobber each
    // other. Local time mirrors what the user sees in the UI.
    let stamp = Local::now().format("%Y-%m-%d_%H%M%S").to_string();
    let path = dir.join(format!("{}.md", stamp));
    crate::platform::write_atomic(&path, md.as_bytes()).context("writing diary markdown")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(id: i64, session: Option<&str>) -> MemoryEntry {
        MemoryEntry {
            id,
            memory_type: crate::memory::MemoryType::User,
            scope: crate::memory::MemoryScope::Global,
            content: format!("memory {id}"),
            tags: Vec::new(),
            source: "auto".to_string(),
            source_session_id: session.map(|s| s.to_string()),
            pinned: false,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            relevance_score: None,
            attachment_path: None,
            attachment_mime: None,
        }
    }

    fn promo(memory_id: i64) -> PromotionRecord {
        PromotionRecord {
            memory_id,
            score: 0.9,
            title: format!("t{memory_id}"),
            rationale: String::new(),
            evidence: Vec::new(),
        }
    }

    #[test]
    fn attach_evidence_anchors_each_promotion_to_its_memory() {
        let candidates = vec![candidate(1, Some("sess-1")), candidate(2, None)];
        let mut promoted = vec![promo(2), promo(1)];
        attach_evidence(&mut promoted, &candidates);

        // The join maps each promotion to the right candidate's memory id,
        // regardless of order. Session refs fail closed without a live
        // session DB (that rule is covered purely in scanner::build_evidence),
        // so here we assert the join + that every promotion is anchored.
        assert_eq!(promoted[0].memory_id, 2);
        assert_eq!(promoted[0].evidence[0].memory_id, Some(2));
        assert_eq!(promoted[1].memory_id, 1);
        assert_eq!(promoted[1].evidence[0].memory_id, Some(1));
        assert!(promoted.iter().all(|p| !p.evidence.is_empty()));
    }

    #[test]
    fn attach_evidence_falls_back_to_memory_id_when_unmatched() {
        let candidates = vec![candidate(1, Some("sess-1"))];
        let mut promoted = vec![promo(99)];
        attach_evidence(&mut promoted, &candidates);
        assert_eq!(promoted[0].evidence.len(), 1);
        assert_eq!(promoted[0].evidence[0].memory_id, Some(99));
    }

    #[test]
    fn render_anchor_is_stable_and_grepable() {
        let refs = vec![EvidenceRef::memory(42), EvidenceRef::session("abc")];
        assert_eq!(render_evidence_anchor(&refs), "memory:42,session:abc");
        assert_eq!(render_evidence_anchor(&[]), "");
    }
}
