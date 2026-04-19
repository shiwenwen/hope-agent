use std::sync::Arc;

use anyhow::Result;
use tokio_util::sync::CancellationToken;

use crate::agent::AssistantAgent;
use crate::session::{MessageRole, SessionDB, SessionMessage};
use crate::truncate_utf8;

use super::db::RecapDb;
use super::types::{
    FrictionCounts, GenerateMode, Outcome, RecapFilters, RecapProgress, SessionFacet,
};

/// Soft per-section text budget passed to the analysis LLM.
const TRANSCRIPT_BUDGET_BYTES: usize = 30_000;
/// Per-tool-result truncation budget (large outputs can dominate transcripts).
const TOOL_RESULT_BUDGET: usize = 2_048;
/// Max bytes per chunk when transcripts exceed the budget.
const CHUNK_BYTES: usize = 22_000;
const FACET_MAX_TOKENS: u32 = 2_048;

/// Information about a session needed for the facet pipeline.
#[derive(Clone)]
pub struct CandidateSession {
    pub session_id: String,
    pub last_message_ts: String,
    pub message_count: i64,
}

/// Resolve which sessions fall in the requested window.
///
/// `Incremental` rolls forward from the previous report's `range_end` (or the
/// configured default window when no prior report exists). `Full` consults the
/// caller-supplied `RecapFilters`.
pub fn resolve_candidates(
    session_db: &Arc<SessionDB>,
    cache: &RecapDb,
    mode: &GenerateMode,
    default_range_days: u32,
    max_sessions: u32,
) -> Result<(Vec<CandidateSession>, RecapFilters)> {
    let (start, end, filters) = match mode {
        GenerateMode::Incremental => {
            let end = chrono::Utc::now();
            let start = match cache.latest_report_range_end()? {
                Some(prev) => chrono::DateTime::parse_from_rfc3339(&prev)
                    .map(|d| d.with_timezone(&chrono::Utc))
                    .unwrap_or_else(|_| end - chrono::Duration::days(default_range_days as i64)),
                None => end - chrono::Duration::days(default_range_days as i64),
            };
            let filters = RecapFilters {
                start_date: Some(format_date(&start)),
                end_date: Some(format_date(&end)),
                agent_id: None,
                provider_id: None,
                model_id: None,
            };
            (start, end, filters)
        }
        GenerateMode::Full { filters } => {
            let start = filters
                .start_date
                .as_deref()
                .and_then(parse_loose_date)
                .unwrap_or_else(|| {
                    chrono::Utc::now() - chrono::Duration::days(default_range_days as i64)
                });
            let end = filters
                .end_date
                .as_deref()
                .and_then(parse_loose_date)
                .unwrap_or_else(chrono::Utc::now);
            (start, end, filters.clone())
        }
    };

    let sessions = session_db.list_sessions(filters.agent_id.as_deref())?;
    let mut candidates: Vec<CandidateSession> = sessions
        .into_iter()
        .filter(|s| {
            // Only consider sessions whose updated_at falls within window.
            match parse_loose_date(&s.updated_at) {
                Some(ts) => ts >= start && ts <= end,
                None => false,
            }
        })
        .filter(|s| s.message_count >= 2)
        .map(|s| CandidateSession {
            session_id: s.id,
            last_message_ts: s.updated_at,
            message_count: s.message_count,
        })
        .collect();

    candidates.sort_by(|a, b| b.last_message_ts.cmp(&a.last_message_ts));
    if candidates.len() > max_sessions as usize {
        candidates.truncate(max_sessions as usize);
    }

    Ok((candidates, filters))
}

fn format_date(d: &chrono::DateTime<chrono::Utc>) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn parse_loose_date(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    // Accept RFC3339, "YYYY-MM-DD HH:MM:SS", or "YYYY-MM-DD".
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(chrono::DateTime::from_naive_utc_and_offset(dt, chrono::Utc));
    }
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let datetime = dt.and_hms_opt(0, 0, 0)?;
        return Some(chrono::DateTime::from_naive_utc_and_offset(
            datetime,
            chrono::Utc,
        ));
    }
    None
}

/// Stream-progress facet extractor. Persists newly-extracted facets to `cache`.
///
/// Uses `futures::stream::buffer_unordered` to bound concurrency without
/// requiring `'static` futures (the agent reference is borrowed by the
/// surrounding async function, so per-item futures are tied to its lifetime).
pub async fn extract_facets_for_candidates<F>(
    session_db: &Arc<SessionDB>,
    cache: &Arc<RecapDb>,
    agent: &AssistantAgent,
    analysis_model: &str,
    candidates: Vec<CandidateSession>,
    concurrency: u8,
    progress: F,
    cancel: CancellationToken,
) -> Result<Vec<SessionFacet>>
where
    F: Fn(RecapProgress) + Send + Sync,
{
    use futures_util::stream::{self, StreamExt};

    let total = candidates.len() as u32;
    progress(RecapProgress::ExtractingFacets {
        completed: 0,
        total,
    });

    let analysis_model_arc: Arc<str> = Arc::from(analysis_model);

    let stream = stream::iter(candidates).map(|cand| {
        let session_db = session_db.clone();
        let cache = cache.clone();
        let analysis_model = analysis_model_arc.clone();
        let cancel = cancel.clone();
        async move {
            if cancel.is_cancelled() {
                anyhow::bail!("cancelled");
            }
            // Cache hit: return immediately without an LLM call.
            if let Ok(Some(facet)) =
                cache.get_cached_facet(&cand.session_id, &cand.last_message_ts, &analysis_model)
            {
                return Ok(facet);
            }
            let messages = session_db.load_session_messages(&cand.session_id)?;
            let transcript = serialize_transcript(&messages);
            let facet = extract_one(agent, &cand.session_id, &transcript).await?;
            if let Err(e) = cache.save_facet(
                &facet,
                &cand.last_message_ts,
                cand.message_count,
                &analysis_model,
            ) {
                app_debug!("recap", "facets", "cache save failed: {}", e);
            }
            Ok::<_, anyhow::Error>(facet)
        }
    });

    let mut buffered = stream.buffer_unordered(concurrency.max(1) as usize);
    let mut completed: u32 = 0;
    let mut facets = Vec::with_capacity(total as usize);
    while let Some(res) = buffered.next().await {
        completed += 1;
        match res {
            Ok(facet) => facets.push(facet),
            Err(e) => {
                app_warn!(
                    "recap",
                    "facets",
                    "facet extraction failed: {}",
                    truncate_utf8(&e.to_string(), 256)
                );
            }
        }
        progress(RecapProgress::ExtractingFacets { completed, total });
        if cancel.is_cancelled() {
            anyhow::bail!("cancelled");
        }
    }

    app_info!(
        "recap",
        "facets",
        "extracted {} facets ({} candidates)",
        facets.len(),
        total
    );
    Ok(facets)
}

/// Extract a session facet. Public so `report.rs` can also persist results
/// with the right cache metadata.
pub async fn extract_one(
    agent: &AssistantAgent,
    session_id: &str,
    transcript: &str,
) -> Result<SessionFacet> {
    let chunks = chunk_transcript(transcript);
    if chunks.len() == 1 {
        let json = run_facet_call(agent, &chunks[0]).await?;
        return parse_or_default(&json, session_id);
    }
    // Long transcript: extract per chunk, then merge.
    let mut chunk_jsons = Vec::with_capacity(chunks.len());
    for chunk in &chunks {
        match run_facet_call(agent, chunk).await {
            Ok(j) => chunk_jsons.push(j),
            Err(e) => app_debug!("recap", "facets", "chunk extraction failed: {}", e),
        }
    }
    if chunk_jsons.is_empty() {
        anyhow::bail!("all chunks failed");
    }
    let merge_input = chunk_jsons.join("\n---\n");
    let merged = run_merge_call(agent, &merge_input).await?;
    parse_or_default(&merged, session_id)
}

/// Partition the transcript so each chunk fits within `CHUNK_BYTES`.
fn chunk_transcript(transcript: &str) -> Vec<String> {
    if transcript.len() <= TRANSCRIPT_BUDGET_BYTES {
        return vec![transcript.to_string()];
    }
    // Bias: keep first window + last window; sample the middle.
    let head_end = chunk_split(transcript, CHUNK_BYTES);
    let head = &transcript[..head_end];

    let tail_len = CHUNK_BYTES.min(transcript.len().saturating_sub(head_end));
    let tail_start = chunk_split(transcript, transcript.len() - tail_len);
    let tail = &transcript[tail_start..];

    if head_end >= tail_start {
        return vec![head.to_string()];
    }

    let mid_total = tail_start - head_end;
    let mut chunks = vec![head.to_string()];
    if mid_total > CHUNK_BYTES {
        // Single deterministic middle slice (centered).
        let mid_center = head_end + mid_total / 2;
        let mid_start = chunk_split(transcript, mid_center.saturating_sub(CHUNK_BYTES / 2));
        let mid_end = chunk_split(transcript, mid_start + CHUNK_BYTES);
        chunks.push(transcript[mid_start..mid_end].to_string());
    }
    chunks.push(tail.to_string());
    chunks
}

/// Round `pos` down to the nearest UTF-8 char boundary that is `<= pos`.
fn chunk_split(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut p = pos;
    while p > 0 && !s.is_char_boundary(p) {
        p -= 1;
    }
    p
}

/// Serialize a session's messages into a compact transcript.
pub fn serialize_transcript(messages: &[SessionMessage]) -> String {
    let mut buf = String::new();
    for m in messages {
        let role = match m.role {
            MessageRole::User => "USER",
            MessageRole::Assistant => "ASSISTANT",
            MessageRole::Tool => "TOOL",
            MessageRole::TextBlock => "ASSISTANT",
            MessageRole::ThinkingBlock => continue,
            MessageRole::Event => continue,
        };
        if !m.content.is_empty() {
            let content = truncate_utf8(&m.content, 4_096);
            buf.push_str(role);
            buf.push_str(": ");
            buf.push_str(content);
            buf.push('\n');
        }
        if let Some(name) = &m.tool_name {
            buf.push_str("TOOL_CALL: ");
            buf.push_str(name);
            buf.push('\n');
        }
        if let Some(result) = &m.tool_result {
            let trimmed = truncate_utf8(result, TOOL_RESULT_BUDGET);
            buf.push_str("TOOL_RESULT: ");
            buf.push_str(trimmed);
            buf.push('\n');
        }
    }
    buf
}

async fn run_facet_call(agent: &AssistantAgent, transcript: &str) -> Result<String> {
    let prompt = format!(
        "You are an analyst extracting structured facets from an AI-assistant chat session.\n\
        Read the transcript below and return a single JSON object with this exact shape:\n\
        {{\n\
          \"underlyingGoal\": string,\n\
          \"goalCategories\": [string],         // e.g. [\"debug\",\"refactor\"]\n\
          \"outcome\": \"fully_achieved\"|\"mostly_achieved\"|\"partial\"|\"failed\"|\"unclear\",\n\
          \"userSatisfaction\": 1-5 | null,\n\
          \"agentHelpfulness\": 1-5 | null,\n\
          \"sessionType\": \"coding\"|\"research\"|\"writing\"|\"ops\"|\"qa\"|\"other\",\n\
          \"frictionCounts\": {{\n\
              \"toolErrors\": number, \"misunderstanding\": number, \"repetition\": number,\n\
              \"userCorrection\": number, \"stuck\": number, \"other\": number\n\
          }},\n\
          \"frictionDetail\": [string],\n\
          \"primarySuccess\": string | null,\n\
          \"briefSummary\": string,\n\
          \"userInstructions\": [string]       // recurring style/process instructions\n\
        }}\n\
        Output ONLY the JSON object — no commentary, no code fences.\n\n\
        TRANSCRIPT:\n{}",
        transcript
    );
    let res = agent.side_query(&prompt, FACET_MAX_TOKENS).await?;
    Ok(res.text)
}

async fn run_merge_call(agent: &AssistantAgent, partials: &str) -> Result<String> {
    let prompt = format!(
        "You will receive several JSON objects — each is a partial facet extraction\n\
         from a chunk of one chat session. Merge them into a single facet JSON\n\
         using the same shape, prioritising signals from the FIRST and LAST chunk for goal/outcome.\n\
         Output ONLY the merged JSON, no commentary.\n\n\
         PARTIALS:\n{}",
        partials
    );
    let res = agent.side_query(&prompt, FACET_MAX_TOKENS).await?;
    Ok(res.text)
}

fn parse_or_default(raw: &str, session_id: &str) -> Result<SessionFacet> {
    let json_str = strip_code_fence(raw.trim());
    let value: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
        anyhow::anyhow!(
            "facet JSON parse error: {} (raw: {})",
            e,
            truncate_utf8(json_str, 256)
        )
    })?;

    let outcome = match value.get("outcome").and_then(|v| v.as_str()).unwrap_or("") {
        "fully_achieved" => Outcome::FullyAchieved,
        "mostly_achieved" => Outcome::MostlyAchieved,
        "partial" => Outcome::Partial,
        "failed" => Outcome::Failed,
        _ => Outcome::Unclear,
    };

    let goal_categories: Vec<String> = value
        .get("goalCategories")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let friction_detail: Vec<String> = value
        .get("frictionDetail")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let user_instructions: Vec<String> = value
        .get("userInstructions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let counts = value.get("frictionCounts");
    let friction_counts = FrictionCounts {
        tool_errors: counts
            .and_then(|c| c.get("toolErrors"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        misunderstanding: counts
            .and_then(|c| c.get("misunderstanding"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        repetition: counts
            .and_then(|c| c.get("repetition"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        user_correction: counts
            .and_then(|c| c.get("userCorrection"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        stuck: counts
            .and_then(|c| c.get("stuck"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        other: counts
            .and_then(|c| c.get("other"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
    };

    Ok(SessionFacet {
        session_id: session_id.to_string(),
        underlying_goal: value
            .get("underlyingGoal")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        goal_categories,
        outcome,
        user_satisfaction: value
            .get("userSatisfaction")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8),
        agent_helpfulness: value
            .get("agentHelpfulness")
            .and_then(|v| v.as_u64())
            .map(|n| n as u8),
        session_type: value
            .get("sessionType")
            .and_then(|v| v.as_str())
            .unwrap_or("other")
            .to_string(),
        friction_counts,
        friction_detail,
        primary_success: value
            .get("primarySuccess")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        brief_summary: value
            .get("briefSummary")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        user_instructions,
    })
}

fn strip_code_fence(s: &str) -> &str {
    let s = s.trim();
    let s = s.strip_prefix("```json").unwrap_or(s);
    let s = s.strip_prefix("```").unwrap_or(s);
    s.strip_suffix("```").unwrap_or(s).trim()
}
