//! `run_review_cycle` — analyze recent messages, invoke the review
//! side_query, parse the JSON decision, then run the deterministic gates
//! (2, 4, 5) before any skill hits disk.
//!
//! Layout follows the five-gate waterfall:
//!   gate 2 (pre_gate)   — runs BEFORE the LLM call
//!   gate 3 (LLM)        — side_query against the configured review model
//!   gate 4 (self-score) — model-reported `reuse_probability` floor +
//!                         `class_level_name` + scenario quality
//!   gate 5 (post_lint)  — deterministic body lint
//!
//! Any skip writes a `skill_review_skipped` learning event with the
//! `reason` field so the UI can show "why this didn't produce a draft".

use std::collections::HashSet;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::AssistantAgent;
use crate::config::cached_config;
use crate::dashboard::{emit_learning_event, EVT_SKILL_DISCARDED};
use crate::skills::author::{
    create_skill, patch_skill_fuzzy, security_scan, CreateOpts, FuzzyOpts, PatchResult,
};
use crate::skills::{load_all_skills_with_extra, SkillEntry, SkillStatus};
use crate::truncate_utf8;

use super::config::{AutoReviewPromotion, SkillsAutoReviewConfig};
use super::heuristics::{jaccard, post_lint, pre_gate, tokenize, PostLintOutcome, PreGateOutcome};
use super::prompts::{render_review_user_prompt, REVIEW_SYSTEM};
use super::triggers::AutoReviewGate;

/// Learning-event kind written whenever a review cycle ends without
/// producing a create/patch (regardless of which gate fired).
pub const EVT_SKILL_REVIEW_SKIPPED: &str = "skill_review_skipped";

// Reason codes for the pipeline's own skip paths (gates 2 and 5 own their
// own consts in `heuristics.rs`). Keep all reject reasons centralised so
// `learning_events.meta_json.reject_reason` is a stable enum the UI can
// localise — see `settings.skillsEvolution.rejectReasons.*` in i18n.
pub const REASON_NO_RECENT_MESSAGES: &str = "no_recent_messages";
pub const REASON_MODEL_DECIDED_SKIP: &str = "model_decided_skip";
pub const REASON_PATCH_TARGET_NOT_FOUND: &str = "patch_target_not_found";
pub const REASON_GATE4_LOW_PROB: &str = "gate4_low_reuse_probability";
pub const REASON_GATE4_MISSING_PROB: &str = "gate4_missing_reuse_probability";
pub const REASON_GATE4_NOT_CLASS_LEVEL: &str = "gate4_not_class_level";
pub const REASON_GATE4_SCENARIOS_MISSING: &str = "gate4_scenarios_missing";
pub const REASON_GATE4_SCENARIOS_TOO_SHORT: &str = "gate4_scenarios_too_short";
pub const REASON_GATE4_SCENARIOS_PARAPHRASE: &str = "gate4_scenarios_paraphrase";

/// Which path fired the review.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewTrigger {
    /// Automatic: cooldown + threshold satisfied.
    PostTurn,
    /// User pressed "Run review now" in the GUI (or a slash command).
    Manual,
}

/// Parsed shape of the review agent's JSON response.
///
/// snake_case canonical; camelCase aliases for models that cargo-cult JS
/// conventions.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewDecision {
    pub decision: String, // "create" | "patch" | "skip"
    #[serde(default, alias = "skillId")]
    pub skill_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default, alias = "oldApprox")]
    pub old_approx: Option<String>,
    #[serde(default, alias = "newText")]
    pub new_text: Option<String>,
    #[serde(default)]
    pub rationale: Option<String>,
    /// 3 concrete future scenarios the model expects this skill to cover.
    #[serde(default, alias = "reuseScenarios")]
    pub reuse_scenarios: Vec<String>,
    /// Model-reported self-estimate that the skill gets used in the next
    /// 30 days. 0.0..=1.0.
    #[serde(default, alias = "reuseProbability")]
    pub reuse_probability: Option<f32>,
    /// Model self-assessment: is `skill_id` class-level (not session-specific).
    #[serde(default, alias = "classLevelName")]
    pub class_level_name: Option<bool>,
}

/// Summary emitted to the EventBus + used for logging/tests.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReport {
    pub trigger: ReviewTrigger,
    pub session_id: String,
    /// `created` | `patched` | `skipped` | `error`.
    pub outcome: String,
    pub skill_id: Option<String>,
    pub similarity: Option<f32>,
    pub rationale: Option<String>,
    /// Stable identifier for *why* a `skipped` outcome happened
    /// (`pre_gate_too_few_messages`, `gate4_low_reuse_probability`, …) —
    /// the UI maps this to a localized one-liner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    /// What in gate 1 fired this run (`tool_use` | `bulk` | `correction`
    /// | `manual`). Useful for diagnostics in the rejects panel.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fire_reason: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Entry point: run the review pipeline for `session_id`. The caller is
/// expected to hand in an `AutoReviewGate` acquired from
/// `triggers::touch_and_maybe_trigger` or `triggers::acquire_manual`.
pub async fn run_review_cycle(
    session_id: &str,
    trigger: ReviewTrigger,
    gate: AutoReviewGate,
    main_agent: Option<&AssistantAgent>,
) -> Result<ReviewReport> {
    let started = Instant::now();
    let fire_reason = gate.fire_reason().to_string();
    let _gate = gate; // hold for the duration of the cycle
    let cfg = cached_config().skills.auto_review.clone().sanitize();

    let outcome = match run_inner(session_id, &cfg, main_agent).await {
        Ok(report) => report,
        Err(err) => ReviewReport {
            trigger,
            session_id: session_id.to_string(),
            outcome: "error".to_string(),
            skill_id: None,
            similarity: None,
            rationale: None,
            reject_reason: None,
            fire_reason: Some(fire_reason.clone()),
            duration_ms: started.elapsed().as_millis() as u64,
            error: Some(err.to_string()),
        },
    };

    let with_trigger = ReviewReport {
        trigger,
        duration_ms: started.elapsed().as_millis() as u64,
        fire_reason: Some(fire_reason),
        ..outcome
    };

    // Surface every "skipped" decision into the learning event stream so
    // the UI can show "recent reject reasons" without scraping logs.
    if with_trigger.outcome == "skipped" {
        let meta = serde_json::json!({
            "reject_reason": with_trigger.reject_reason,
            "rationale": with_trigger.rationale,
            "fire_reason": with_trigger.fire_reason,
            "session_id": with_trigger.session_id,
        });
        emit_learning_event(
            EVT_SKILL_REVIEW_SKIPPED,
            Some(&with_trigger.session_id),
            with_trigger.skill_id.as_deref(),
            Some(&meta),
        );
    }

    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "skills:auto_review_complete",
            serde_json::to_value(&with_trigger).unwrap_or(Value::Null),
        );
    }

    Ok(with_trigger)
}

fn skip_report(session_id: &str, reason: &str, rationale: Option<String>) -> ReviewReport {
    ReviewReport {
        trigger: ReviewTrigger::PostTurn,
        session_id: session_id.to_string(),
        outcome: "skipped".to_string(),
        skill_id: None,
        similarity: None,
        rationale,
        reject_reason: Some(reason.to_string()),
        fire_reason: None,
        duration_ms: 0,
        error: None,
    }
}

async fn run_inner(
    session_id: &str,
    cfg: &SkillsAutoReviewConfig,
    main_agent: Option<&AssistantAgent>,
) -> Result<ReviewReport> {
    // ── Collect transcript ─────────────────────────────────────────────
    let (conversation, message_count) = collect_recent_messages(session_id, cfg.candidate_limit)
        .context("collect recent messages")?;
    if conversation.trim().is_empty() {
        return Ok(skip_report(
            session_id,
            REASON_NO_RECENT_MESSAGES,
            Some("no recent messages".to_string()),
        ));
    }

    // Conversation tokens are reused by gate 2 (`pre_gate`) and the
    // dedup-block builder; tokenize once.
    let conv_keys = tokenize(&conversation);

    // ── Gate 2: pre-LLM heuristics ─────────────────────────────────────
    let recent_discards = load_recent_discards(cfg);
    let discard_topics: Vec<(String, String)> = recent_discards
        .iter()
        .map(|e| (e.id.clone(), e.topic_text.clone()))
        .collect();
    if let PreGateOutcome {
        allow: false,
        reason,
        hit,
    } = pre_gate(cfg, message_count, &conv_keys, &discard_topics)
    {
        let rationale = hit
            .as_ref()
            .map(|h| format!("blocked by discard topic: {}", h));
        return Ok(skip_report(
            session_id,
            reason.as_deref().unwrap_or("pre_gate"),
            rationale,
        ));
    }

    // ── Build top-K dedup candidates with full body ────────────────────
    let entries: Vec<SkillEntry> = load_all_skills_with_extra(&cached_config().extra_skills_dirs)
        .into_iter()
        .filter(|s| s.source != "bundled")
        .collect();
    let dedup_block = build_dedup_block(&entries, &conv_keys, cfg);
    let blacklist_block = recent_discards
        .iter()
        .map(|e| {
            if e.topic_text != e.id {
                format!("- {} — {}", e.id, truncate_utf8(&e.topic_text, 120))
            } else {
                format!("- {}", e.id)
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    // ── Build prompt; route to cached side_query if main_agent is here ─
    let system_prompt = cfg
        .review_system_override
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(REVIEW_SYSTEM);
    let user_prompt = render_review_user_prompt(
        &dedup_block,
        &blacklist_block,
        &cfg.extra_reject_categories,
        &conversation,
    );
    let instruction = format!("{}\n\n{}", system_prompt, user_prompt);

    // ── Gate 3: LLM review ─────────────────────────────────────────────
    let response_text = query_review_agent(&instruction, cfg, main_agent).await?;
    let decision = parse_review_response(&response_text).context("parse review decision JSON")?;

    // ── Route ──────────────────────────────────────────────────────────
    match decision.decision.as_str() {
        "create" => apply_create(session_id, cfg, decision),
        "patch" => apply_patch(session_id, decision),
        _ => Ok(skip_report(
            session_id,
            REASON_MODEL_DECIDED_SKIP,
            decision.rationale,
        )),
    }
}

async fn query_review_agent(
    instruction: &str,
    cfg: &SkillsAutoReviewConfig,
    main_agent: Option<&AssistantAgent>,
) -> Result<String> {
    let timeout = Duration::from_secs(cfg.timeout_secs);

    // Precedence: explicit override (`model_override`, or the deprecated
    // `review_model` string) > main agent's cached prefix > automation
    // default > chat default. The override path intentionally skips
    // main_agent so users pinning a cheap model for review aren't
    // double-charged via the main chat's cache.
    let override_chain = cfg.model_override.clone().or_else(|| {
        cfg.review_model
            .as_deref()
            .and_then(crate::automation::parse_legacy_model_string)
    });
    if let Some(chain) = override_chain {
        let fut = crate::automation::run(crate::automation::ModelTaskSpec {
            purpose: "skills.auto_review",
            chain: chain.into_vec(),
            session_key: "automation:skills_auto_review",
            instruction,
            max_tokens: 4096,
        });
        let res = tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow::anyhow!("review side_query timed out (override model)"))??;
        return Ok(res.text);
    }

    if let Some(agent) = main_agent {
        let fut = agent.side_query(instruction, 4096);
        let res = tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow::anyhow!("review side_query timed out"))??;
        return Ok(res.text);
    }

    let config = cached_config();
    let chain = crate::automation::effective_chain(&config, None);
    let fut = crate::automation::run(crate::automation::ModelTaskSpec {
        purpose: "skills.auto_review",
        chain,
        session_key: "automation:skills_auto_review",
        instruction,
        max_tokens: 4096,
    });
    let res = tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| anyhow::anyhow!("review side_query timed out (fallback)"))??;
    Ok(res.text)
}

fn parse_review_response(text: &str) -> Result<ReviewDecision> {
    let span = crate::extract_json_span(text, Some('{'))
        .ok_or_else(|| anyhow::anyhow!("no JSON object found in review response"))?;
    let value: ReviewDecision = serde_json::from_str(span).context("decode review decision")?;
    Ok(value)
}

fn apply_create(
    session_id: &str,
    cfg: &SkillsAutoReviewConfig,
    d: ReviewDecision,
) -> Result<ReviewReport> {
    let skill_id = d
        .skill_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(sanitize_id)
        .ok_or_else(|| anyhow::anyhow!("create decision missing skill_id"))?;
    let name = d
        .name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(&skill_id);
    let description = d
        .description
        .as_deref()
        .map(str::trim)
        .unwrap_or("")
        .to_string();
    let body = d.body.as_deref().map(str::trim).unwrap_or("").to_string();
    if body.is_empty() {
        return Err(anyhow::anyhow!("create decision missing body"));
    }
    security_scan(&body)?;

    // ── Gate 4: model self-score floor ─────────────────────────────────
    if let Some(p) = d.reuse_probability {
        if p < cfg.min_reuse_probability {
            return Ok(skip_report(
                session_id,
                REASON_GATE4_LOW_PROB,
                Some(format!(
                    "model reported reuse_probability={:.2}; floor={:.2}",
                    p, cfg.min_reuse_probability
                )),
            ));
        }
    } else {
        return Ok(skip_report(
            session_id,
            REASON_GATE4_MISSING_PROB,
            Some("model omitted required `reuse_probability` field".to_string()),
        ));
    }
    let class_level = d.class_level_name.unwrap_or(false);
    if !class_level {
        return Ok(skip_report(
            session_id,
            REASON_GATE4_NOT_CLASS_LEVEL,
            Some("model self-flagged skill_id as session-specific".to_string()),
        ));
    }
    if let Some(reason) = validate_reuse_scenarios(&d.reuse_scenarios) {
        return Ok(skip_report(session_id, reason, None));
    }

    // ── Gate 5: deterministic body lint ────────────────────────────────
    let lint = post_lint(cfg, &skill_id, &body, class_level);
    if let PostLintOutcome {
        allow: false,
        reason,
        detail,
    } = lint
    {
        return Ok(skip_report(
            session_id,
            reason.as_deref().unwrap_or("post_lint"),
            detail,
        ));
    }

    let status = match cfg.promotion {
        AutoReviewPromotion::Draft => SkillStatus::Draft,
        AutoReviewPromotion::Auto => SkillStatus::Active,
    };
    let opts = CreateOpts {
        status,
        authored_by: "auto-review".to_string(),
        rationale: d.rationale.clone(),
    };

    let _ = create_skill(&skill_id, &description, &rebody(&body, name), opts)?;
    Ok(ReviewReport {
        trigger: ReviewTrigger::PostTurn, // set by caller
        session_id: session_id.to_string(),
        outcome: "created".to_string(),
        skill_id: Some(skill_id),
        similarity: None,
        rationale: d.rationale,
        reject_reason: None,
        fire_reason: None,
        duration_ms: 0,
        error: None,
    })
}

fn apply_patch(session_id: &str, d: ReviewDecision) -> Result<ReviewReport> {
    let skill_id = d
        .skill_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(sanitize_id)
        .ok_or_else(|| anyhow::anyhow!("patch decision missing skill_id"))?;
    let old = d
        .old_approx
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("patch decision missing old_approx"))?
        .to_string();
    let new = d
        .new_text
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("patch decision missing new_text"))?
        .to_string();
    security_scan(&new)?;

    match patch_skill_fuzzy(&skill_id, &old, &new, FuzzyOpts::default())? {
        PatchResult::Exact => Ok(ReviewReport {
            trigger: ReviewTrigger::PostTurn,
            session_id: session_id.to_string(),
            outcome: "patched".to_string(),
            skill_id: Some(skill_id),
            similarity: Some(1.0),
            rationale: d.rationale,
            reject_reason: None,
            fire_reason: None,
            duration_ms: 0,
            error: None,
        }),
        PatchResult::Fuzzy { similarity } => Ok(ReviewReport {
            trigger: ReviewTrigger::PostTurn,
            session_id: session_id.to_string(),
            outcome: "patched".to_string(),
            skill_id: Some(skill_id),
            similarity: Some(similarity),
            rationale: d.rationale,
            reject_reason: None,
            fire_reason: None,
            duration_ms: 0,
            error: None,
        }),
        PatchResult::NotFound { best_similarity } => Ok(ReviewReport {
            trigger: ReviewTrigger::PostTurn,
            session_id: session_id.to_string(),
            outcome: "skipped".to_string(),
            skill_id: Some(skill_id),
            similarity: Some(best_similarity),
            rationale: Some(format!(
                "patch target not found (best similarity {:.2})",
                best_similarity
            )),
            reject_reason: Some(REASON_PATCH_TARGET_NOT_FOUND.to_string()),
            fire_reason: None,
            duration_ms: 0,
            error: None,
        }),
    }
}

/// Validate that `reuse_scenarios` is a list of 3 distinct, non-trivial
/// future scenarios. Returns a `Some(reason)` to short-circuit gate 4.
fn validate_reuse_scenarios(scenarios: &[String]) -> Option<&'static str> {
    if scenarios.len() < 3 {
        return Some(REASON_GATE4_SCENARIOS_MISSING);
    }
    for s in scenarios {
        if s.chars().count() < 20 {
            return Some(REASON_GATE4_SCENARIOS_TOO_SHORT);
        }
    }
    // Pairwise Jaccard: if any pair >= 0.8 the model is paraphrasing
    // itself.
    let toks: Vec<HashSet<String>> = scenarios.iter().map(|s| tokenize(s)).collect();
    for i in 0..toks.len() {
        for j in (i + 1)..toks.len() {
            if jaccard(&toks[i], &toks[j]) >= 0.8 {
                return Some(REASON_GATE4_SCENARIOS_PARAPHRASE);
            }
        }
    }
    None
}

/// Turn a potentially free-form body into one that always has a top-level
/// `# {name}` header. The author layer will inject YAML frontmatter for us.
fn rebody(body: &str, name: &str) -> String {
    let trimmed = body.trim_start();
    if trimmed.starts_with('#') {
        trimmed.to_string()
    } else {
        format!("# {}\n\n{}", name, trimmed)
    }
}

fn sanitize_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for c in raw.trim().to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if c.is_whitespace() {
            if !out.ends_with('-') {
                out.push('-');
            }
        }
    }
    out.trim_matches(|c: char| c == '-' || c == '_').to_string()
}

/// One entry of the discard blacklist used by gate 2 and the LLM prompt.
/// `topic_text` is the most language-rich representation we have for that
/// skill: `description` if it was captured at delete-time, else `id`.
#[derive(Debug, Clone)]
struct DiscardEntry {
    id: String,
    topic_text: String,
}

/// Read recent `skill_discarded` events from `session.db`. Empty when the
/// blacklist is disabled (`discard_blacklist_days == 0`).
fn load_recent_discards(cfg: &SkillsAutoReviewConfig) -> Vec<DiscardEntry> {
    if cfg.discard_blacklist_days == 0 {
        return Vec::new();
    }
    let Some(db) = crate::get_session_db() else {
        return Vec::new();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let since = now.saturating_sub((cfg.discard_blacklist_days * 86_400) as i64);
    let rows = db
        .recent_learning_event_rows(EVT_SKILL_DISCARDED, since, 100)
        .unwrap_or_default();
    rows.into_iter()
        .map(|(id, meta)| {
            let description = meta
                .as_deref()
                .and_then(|m| serde_json::from_str::<Value>(m).ok())
                .and_then(|v| {
                    v.get("description")
                        .and_then(|d| d.as_str())
                        .map(String::from)
                })
                .filter(|s| !s.trim().is_empty());
            let topic_text = description.unwrap_or_else(|| id.clone());
            DiscardEntry { id, topic_text }
        })
        .collect()
}

/// Pre-format the top-K dedup candidates. Each entry: header line +
/// truncated frontmatter+body so the model can judge "is this the same
/// territory" without us shipping the entire library. Reads each skill's
/// `SKILL.md` directly via the prebuilt `entry.file_path` — avoids the
/// N+1 re-discovery `get_skill_content` would otherwise trigger.
fn build_dedup_block(
    entries: &[SkillEntry],
    conv_keys: &HashSet<String>,
    cfg: &SkillsAutoReviewConfig,
) -> String {
    if entries.is_empty() {
        return String::new();
    }
    // Score each entry by Jaccard against the transcript.
    let mut scored: Vec<(f32, &SkillEntry)> = entries
        .iter()
        .map(|e| {
            let mut hay = String::new();
            hay.push_str(&e.name);
            hay.push(' ');
            hay.push_str(&e.description);
            (jaccard(conv_keys, &tokenize(&hay)), e)
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    let mut out = String::new();
    let mut taken = 0usize;
    for (score, entry) in scored.iter() {
        if taken >= cfg.top_k_for_dedup {
            break;
        }
        let raw = match std::fs::read_to_string(&entry.file_path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let body = truncate_utf8(raw.as_str(), 1200);
        out.push_str(&format!(
            "─── {} (score {:.2}) — {}\n{}\n\n",
            entry.name,
            score,
            truncate_utf8(&entry.description, 120),
            body
        ));
        taken += 1;
    }
    out
}

/// Grab the most recent N messages from `session.db` and format them as
/// a plain-text transcript for the prompt. Returns the trimmed transcript
/// and the count of role-bearing entries (used by gate 2).
fn collect_recent_messages(session_id: &str, limit: usize) -> Result<(String, usize)> {
    let db =
        crate::get_session_db().ok_or_else(|| anyhow::anyhow!("session DB not initialized"))?;
    let raw = match db.load_context(session_id)? {
        Some(s) => s,
        None => return Ok((String::new(), 0)),
    };
    let messages: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
    let mut lines: Vec<String> = Vec::new();
    for msg in messages
        .iter()
        .rev()
        .take(limit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let role = msg.get("role").and_then(|v| v.as_str()).unwrap_or("?");
        let content = extract_text(msg);
        let trimmed = content.trim();
        if trimmed.is_empty() {
            continue;
        }
        let one_line = truncate_utf8(trimmed, 800);
        lines.push(format!("[{}]: {}", role, one_line));
    }
    let count = lines.len();
    Ok((lines.join("\n\n"), count))
}

fn extract_text(msg: &Value) -> String {
    if let Some(s) = msg.get("content").and_then(|v| v.as_str()) {
        return s.to_string();
    }
    if let Some(arr) = msg.get("content").and_then(|v| v.as_array()) {
        let parts: Vec<&str> = arr
            .iter()
            .filter_map(|b| {
                let ty = b.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match ty {
                    "text" | "output_text" => b.get("text").and_then(|t| t.as_str()),
                    "tool_use" => Some("(tool_use)"),
                    "tool_result" => Some("(tool_result)"),
                    _ => None,
                }
            })
            .collect();
        return parts.join("\n");
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_decision_basic() {
        let text = r##"Here's my call:
```json
{
  "decision":"create",
  "skill_id":"foo-bar",
  "name":"Foo Bar",
  "description":"desc",
  "body":"# Body\n",
  "rationale":"reusable",
  "reuse_scenarios":["a","b","c"],
  "reuse_probability":0.8,
  "class_level_name":true
}
```"##;
        let d = parse_review_response(text).unwrap();
        assert_eq!(d.decision, "create");
        assert_eq!(d.skill_id.as_deref(), Some("foo-bar"));
        assert_eq!(d.reuse_probability, Some(0.8));
        assert_eq!(d.class_level_name, Some(true));
        assert_eq!(d.reuse_scenarios.len(), 3);
    }

    #[test]
    fn parse_decision_skip() {
        let text = r#"{"decision":"skip","rationale":"nothing reusable"}"#;
        let d = parse_review_response(text).unwrap();
        assert_eq!(d.decision, "skip");
    }

    #[test]
    fn sanitize_id_basic() {
        assert_eq!(sanitize_id("Foo Bar!"), "foo-bar");
        assert_eq!(sanitize_id("  leading  "), "leading");
        assert_eq!(sanitize_id("foo--bar"), "foo--bar");
    }

    #[test]
    fn rebody_adds_header() {
        assert_eq!(rebody("content", "name"), "# name\n\ncontent");
        assert_eq!(rebody("# already\n\nbody", "name"), "# already\n\nbody");
    }

    fn cfg_strict() -> SkillsAutoReviewConfig {
        SkillsAutoReviewConfig::default()
    }

    #[test]
    fn gate4_rejects_low_reuse_probability() {
        let cfg = cfg_strict();
        let body = "## Steps\n1. run `cargo check`\n2. read `Cargo.toml`\n3. fix";
        let d = ReviewDecision {
            decision: "create".into(),
            skill_id: Some("audit-rust-clippy-warnings".into()),
            name: Some("audit".into()),
            description: Some("x".into()),
            body: Some(body.into()),
            old_approx: None,
            new_text: None,
            rationale: None,
            reuse_scenarios: vec![
                "scenario one ............ enough length".into(),
                "scenario two ............ enough length".into(),
                "scenario three .......... enough length".into(),
            ],
            reuse_probability: Some(0.4),
            class_level_name: Some(true),
        };
        let r = apply_create("sid", &cfg, d).unwrap();
        assert_eq!(r.outcome, "skipped");
        assert_eq!(r.reject_reason.as_deref(), Some(REASON_GATE4_LOW_PROB));
    }

    #[test]
    fn gate4_rejects_not_class_level() {
        let cfg = cfg_strict();
        let body = "## Steps\n1. run `cargo check`\n2. read `Cargo.toml`\n3. fix";
        let d = ReviewDecision {
            decision: "create".into(),
            skill_id: Some("ok-name".into()),
            name: Some("name".into()),
            description: Some("x".into()),
            body: Some(body.into()),
            old_approx: None,
            new_text: None,
            rationale: None,
            reuse_scenarios: vec![
                "scenario one ............ enough length".into(),
                "scenario two ............ enough length".into(),
                "scenario three .......... enough length".into(),
            ],
            reuse_probability: Some(0.9),
            class_level_name: Some(false),
        };
        let r = apply_create("sid", &cfg, d).unwrap();
        assert_eq!(r.outcome, "skipped");
        assert_eq!(
            r.reject_reason.as_deref(),
            Some(REASON_GATE4_NOT_CLASS_LEVEL)
        );
    }

    #[test]
    fn gate4_rejects_paraphrased_scenarios() {
        let cfg = cfg_strict();
        let body = "## Steps\n1. run `cargo check`\n2. read `Cargo.toml`\n3. fix";
        let same = "run cargo clippy on the workspace before pushing";
        let d = ReviewDecision {
            decision: "create".into(),
            skill_id: Some("ok-name".into()),
            name: Some("name".into()),
            description: Some("x".into()),
            body: Some(body.into()),
            old_approx: None,
            new_text: None,
            rationale: None,
            reuse_scenarios: vec![same.into(), same.into(), same.into()],
            reuse_probability: Some(0.9),
            class_level_name: Some(true),
        };
        let r = apply_create("sid", &cfg, d).unwrap();
        assert_eq!(r.outcome, "skipped");
        assert_eq!(
            r.reject_reason.as_deref(),
            Some(REASON_GATE4_SCENARIOS_PARAPHRASE)
        );
    }

    #[test]
    fn gate5_rejects_session_artifact_name() {
        let cfg = cfg_strict();
        let body = "## Steps\n1. run `cargo check`\n2. read `Cargo.toml`\n3. fix";
        let d = ReviewDecision {
            decision: "create".into(),
            skill_id: Some("fix-issue-123".into()),
            name: Some("name".into()),
            description: Some("x".into()),
            body: Some(body.into()),
            old_approx: None,
            new_text: None,
            rationale: None,
            reuse_scenarios: vec![
                "scenario one ............ enough length".into(),
                "scenario two ............ enough length".into(),
                "scenario three .......... enough length".into(),
            ],
            reuse_probability: Some(0.9),
            class_level_name: Some(true),
        };
        let r = apply_create("sid", &cfg, d).unwrap();
        assert_eq!(r.outcome, "skipped");
        assert_eq!(
            r.reject_reason.as_deref(),
            Some(crate::skills::auto_review::heuristics::REASON_SESSION_ARTIFACT_NAME)
        );
    }
}
