//! `run_review_cycle` — analyze recent messages, invoke the review side_query,
//! parse the JSON decision, and route to `skills::author` for CRUD.

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::agent::AssistantAgent;
use crate::config::cached_config;
use crate::skills::author::{
    create_skill, patch_skill_fuzzy, security_scan, CreateOpts, FuzzyOpts, PatchResult,
};
use crate::skills::{load_all_skills_with_extra, SkillStatus};
use crate::truncate_utf8;

use super::config::{AutoReviewPromotion, SkillsAutoReviewConfig};
use super::prompts::{render_review_user_prompt, REVIEW_SYSTEM};
use super::triggers::AutoReviewGate;

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
/// Prompt uses snake_case keys (`skill_id`, `old_approx`, `new_text`). Aliased
/// camelCase is accepted as a forgiveness path for models that cargo-cult the
/// common JS convention.
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
}

/// Summary emitted to the EventBus + used for logging/tests.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewReport {
    pub trigger: ReviewTrigger,
    pub session_id: String,
    pub outcome: String, // "created" | "patched" | "skipped" | "error"
    pub skill_id: Option<String>,
    pub similarity: Option<f32>,
    pub rationale: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Entry point: run the review pipeline for `session_id`. The caller is
/// expected to hand in an `AutoReviewGate` acquired from
/// `triggers::touch_and_maybe_trigger` (PostTurn) or `triggers::acquire_manual`
/// (Manual). Keeping gate acquisition outside this function lets the
/// chat_engine skip spawning the background task entirely when thresholds
/// aren't met, which is the common case on short turns.
pub async fn run_review_cycle(
    session_id: &str,
    trigger: ReviewTrigger,
    gate: AutoReviewGate,
    main_agent: Option<&AssistantAgent>,
) -> Result<ReviewReport> {
    let started = Instant::now();
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
            duration_ms: started.elapsed().as_millis() as u64,
            error: Some(err.to_string()),
        },
    };

    let with_trigger = ReviewReport {
        trigger,
        duration_ms: started.elapsed().as_millis() as u64,
        ..outcome
    };

    if let Some(bus) = crate::get_event_bus() {
        bus.emit(
            "skills:auto_review_complete",
            serde_json::to_value(&with_trigger).unwrap_or(Value::Null),
        );
    }

    Ok(with_trigger)
}

async fn run_inner(
    session_id: &str,
    cfg: &SkillsAutoReviewConfig,
    main_agent: Option<&AssistantAgent>,
) -> Result<ReviewReport> {
    // 1. Collect recent conversation.
    let conversation = collect_recent_messages(session_id, cfg.candidate_limit)
        .context("collect recent messages")?;
    if conversation.trim().is_empty() {
        return Ok(ReviewReport {
            trigger: ReviewTrigger::PostTurn,
            session_id: session_id.to_string(),
            outcome: "skipped".to_string(),
            skill_id: None,
            similarity: None,
            rationale: Some("no recent messages".to_string()),
            duration_ms: 0,
            error: None,
        });
    }

    // 2. Existing skills (non-bundled only; bundled skills should never be
    // patched by the model, and we deliberately let bundled names *shadow*
    // an accidental create_skill collision via author::validate_skill_id).
    let config = cached_config();
    let existing_skills = load_all_skills_with_extra(&config.extra_skills_dirs)
        .into_iter()
        .filter(|s| s.source != "bundled")
        .map(|s| {
            format!(
                "- {} — {}",
                s.name,
                truncate_utf8(s.description.as_str(), 80)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // 3. Build prompt; route to cached side_query if we have the main agent.
    let user_prompt = render_review_user_prompt(&existing_skills, &conversation);
    let instruction = format!("{}\n\n{}", REVIEW_SYSTEM, user_prompt);

    let response_text = query_review_agent(&instruction, cfg, main_agent).await?;

    // 4. Parse.
    let decision = parse_review_response(&response_text).context("parse review decision JSON")?;

    // 5. Route.
    match decision.decision.as_str() {
        "create" => apply_create(session_id, cfg, decision),
        "patch" => apply_patch(session_id, decision),
        _ => Ok(ReviewReport {
            trigger: ReviewTrigger::PostTurn,
            session_id: session_id.to_string(),
            outcome: "skipped".to_string(),
            skill_id: decision.skill_id,
            similarity: None,
            rationale: decision.rationale,
            duration_ms: 0,
            error: None,
        }),
    }
}

async fn query_review_agent(
    instruction: &str,
    cfg: &SkillsAutoReviewConfig,
    main_agent: Option<&AssistantAgent>,
) -> Result<String> {
    let timeout = Duration::from_secs(cfg.timeout_secs);

    // Precedence: explicit `review_model` override > main agent's cached prefix
    // > recap's analysis agent fallback. The override path intentionally
    // skips main_agent so users pinning a cheap model for review aren't
    // double-charged via the main chat's cache.
    if let Some(model_ref) = cfg
        .review_model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(agent) = build_review_agent_from_model_ref(model_ref) {
            let fut = agent.side_query(instruction, 4096);
            let res = tokio::time::timeout(timeout, fut)
                .await
                .map_err(|_| anyhow::anyhow!("review side_query timed out (override agent)"))??;
            return Ok(res.text);
        }
        app_warn!(
            "skills",
            "auto_review",
            "review_model '{}' not found in providers; falling back",
            model_ref
        );
    }

    if let Some(agent) = main_agent {
        let fut = agent.side_query(instruction, 4096);
        let res = tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| anyhow::anyhow!("review side_query timed out"))??;
        return Ok(res.text);
    }

    let config = cached_config();
    let (agent, _model_id) = crate::recap::report::build_analysis_agent(&config)
        .context("build fallback analysis agent for auto-review")?;
    let fut = agent.side_query(instruction, 4096);
    let res = tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| anyhow::anyhow!("review side_query timed out (fallback agent)"))??;
    Ok(res.text)
}

/// Parse a `providerId:modelId` override (e.g. `"anthropic:claude-haiku-4-5"`)
/// and build a fresh `AssistantAgent` for it. Returns `None` when the provider
/// / model isn't configured; callers fall back to the usual chain.
fn build_review_agent_from_model_ref(model_ref: &str) -> Option<AssistantAgent> {
    let (provider_id, model_id) = model_ref.split_once(':')?;
    let config = cached_config();
    let prov = crate::provider::find_provider(&config.providers, provider_id.trim())?;
    Some(
        AssistantAgent::new_from_provider(prov, model_id.trim())
            .with_failover_context(prov),
    )
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

    let status = match cfg.promotion {
        AutoReviewPromotion::Draft => SkillStatus::Draft,
        AutoReviewPromotion::Auto => SkillStatus::Active,
    };
    let opts = CreateOpts {
        status,
        authored_by: "auto-review".to_string(),
        rationale: d.rationale.clone(),
        fail_if_exists: true,
    };

    let _ = create_skill(&skill_id, &description, &rebody(&body, name), opts)?;
    Ok(ReviewReport {
        trigger: ReviewTrigger::PostTurn, // set by caller
        session_id: session_id.to_string(),
        outcome: "created".to_string(),
        skill_id: Some(skill_id),
        similarity: None,
        rationale: d.rationale,
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
            duration_ms: 0,
            error: None,
        }),
    }
}

/// Turn a potentially free-form body into one that always has a top-level `#
/// {name}` header. The author layer will inject YAML frontmatter for us.
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

/// Grab the most recent N messages from `session.db` and format them as a
/// plain-text transcript for the prompt. Role-preserving but tool-call heavy
/// turns are compacted to short stubs to keep the prompt bounded.
fn collect_recent_messages(session_id: &str, limit: usize) -> Result<String> {
    let db =
        crate::get_session_db().ok_or_else(|| anyhow::anyhow!("session DB not initialized"))?;
    let raw = match db.load_context(session_id)? {
        Some(s) => s,
        None => return Ok(String::new()),
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
    Ok(lines.join("\n\n"))
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
{"decision":"create","skill_id":"foo-bar","name":"Foo Bar","description":"desc","body":"# Body\n","rationale":"reusable"}
```"##;
        let d = parse_review_response(text).unwrap();
        assert_eq!(d.decision, "create");
        assert_eq!(d.skill_id.as_deref(), Some("foo-bar"));
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
}
