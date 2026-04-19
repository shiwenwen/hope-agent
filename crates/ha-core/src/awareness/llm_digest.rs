//! LLM-based digest extraction for behavior awareness.
//!
//! This module is focused on shaping the extraction prompt; the actual
//! `side_query` invocation and timeout handling lives on `AssistantAgent` so
//! it can access the provider machinery directly. Extraction runs **inline**
//! with a tight timeout, so it never blocks a turn for more than a few
//! seconds — and the result is available on the same turn that triggered it.

use super::config::AwarenessConfig;
use super::types::AwarenessEntry;
use crate::session::SessionDB;

const EXTRACTION_SYSTEM_PREAMBLE: &str = "\
You are generating a compact behavior snapshot for another \
conversation. The snapshot describes what the user is CURRENTLY doing in \
other parallel sessions so the main agent can understand references like \
\"that thing I was working on\".";

const EXTRACTION_INSTRUCTIONS: &str = "\n\nInstructions:\n\
1. Output one bullet per candidate session, format:\n\
   - **<title>** (<relative time>): <one-sentence concrete action>, <progress or blocker>\n\
2. Every bullet MUST contain a verb + concrete noun (e.g. \"just finished the Stripe \
v2 webhook unit test\"). Forbidden: vague labels like \"focused on\", \"working on\", \
\"dealing with\", domain/skill tags.\n\
3. Include a relative time anchor (\"30 seconds ago\", \"5 minutes ago\", \"2 hours ago\").\n\
4. If a candidate session is plausibly the SAME topic as the main conversation, \
append the literal marker \"**possibly same topic**\".\n\
5. If there is not enough evidence, write \"(insufficient info — only title known)\" \
for that bullet. Never fabricate progress.\n\
6. Keep each bullet under 60 characters of prose (not counting the title).\n\
7. Order bullets: currently active → recent → earlier.\n\
8. Do NOT include any preamble, headings, or trailing commentary — emit only the bullet list.\n";

/// Build the LLM prompt for the extraction side_query. Called from
/// `AssistantAgent::run_extraction_inline`.
pub(crate) fn build_extraction_prompt(
    candidates: &[AwarenessEntry],
    cfg: &AwarenessConfig,
    session_db: &SessionDB,
) -> anyhow::Result<String> {
    let mut out = String::new();
    out.push_str(EXTRACTION_SYSTEM_PREAMBLE);
    out.push_str("\n\nCandidate sessions:\n\n");

    let since = chrono::Utc::now()
        - chrono::Duration::hours(cfg.llm_extraction.input_lookback_hours.max(1));
    let since_rfc = since.to_rfc3339();

    for (i, entry) in candidates.iter().enumerate() {
        let mut block = String::new();
        block.push_str(&format!("{}. **{}**", i + 1, entry.title));
        if let Some(name) = &entry.agent_name {
            block.push_str(&format!(" · agent={}", name));
        }
        block.push_str(&format!(" · {}", entry.session_kind.as_str()));
        block.push_str(&format!(" · {}s ago\n", entry.age_secs));
        if let Some(goal) = &entry.underlying_goal {
            block.push_str(&format!("   goal: {}\n", goal));
        }
        if let Some(summary) = &entry.brief_summary {
            block.push_str(&format!("   summary: {}\n", summary));
        }
        // Recent user messages.
        let msgs_result: anyhow::Result<Vec<String>> = session_db.recent_user_messages_for_preview(
            &entry.session_id,
            &since_rfc,
            3,
            (cfg.llm_extraction.per_session_input_chars / 3).max(120),
        );
        if let Ok(msgs) = msgs_result {
            if !msgs.is_empty() {
                block.push_str("   recent user messages:\n");
                for m in msgs {
                    block.push_str("     - ");
                    block.push_str(&m.replace('\n', " "));
                    block.push('\n');
                }
            }
        }
        // Enforce single-block budget.
        let truncated =
            crate::truncate_utf8(&block, cfg.llm_extraction.per_session_input_chars).to_string();
        out.push_str(&truncated);
        out.push('\n');
    }

    out.push_str(EXTRACTION_INSTRUCTIONS);
    Ok(out)
}

/// Approximate token budget for `digest_max_chars` characters of output.
/// Used by `AssistantAgent::run_extraction_inline`.
pub fn token_budget_for_chars(chars: usize) -> u32 {
    ((chars / 3) as u32).max(256).min(2048)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_budget_has_floor_and_ceiling() {
        assert_eq!(token_budget_for_chars(0), 256);
        assert_eq!(token_budget_for_chars(1200), 400);
        assert_eq!(token_budget_for_chars(100_000), 2048);
    }

    #[test]
    fn extraction_instructions_include_key_rules() {
        // Smoke test: the constants shouldn't drift silently.
        assert!(EXTRACTION_INSTRUCTIONS.contains("possibly same topic"));
        assert!(EXTRACTION_INSTRUCTIONS.contains("insufficient info"));
    }
}
