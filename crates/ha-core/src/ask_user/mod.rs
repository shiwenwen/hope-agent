//! Independent ask-user question module.
//!
//! Provides a general-purpose structured Q&A tool that allows the LLM to send
//! interactive questions to the user in any conversation (not only Plan Mode).

mod questions;
mod types;

// ── Re-exports ──────────────────────────────────────────────────

pub use types::{
    AskUserI18nText, AskUserQuestion, AskUserQuestionAnswer, AskUserQuestionGroup,
    AskUserQuestionOption, AskUserText,
};

pub use questions::{
    cancel_pending_ask_user_question, find_live_pending_group_for_session,
    is_ask_user_question_live, mark_group_answered, persist_pending_group,
    register_ask_user_question, submit_ask_user_question_response, EVENT_ASK_USER_REQUEST,
};

/// Parse an `ask_user_question::execute` reply and return true iff any
/// selected answer matches one of the affirmative `labels` (case-insensitive,
/// trimmed). A `timedOut: true` reply NEVER counts as affirmative — even
/// if a default option happens to match an affirmative label, treating a
/// timeout as consent would be a silent privilege escalation.
///
/// Use this for every Yes/No (or Continue/Cancel) gate that wraps a
/// dangerous tool action — `control.evaluate`, `app_update install/rollback`,
/// etc. — so the affirmative-label scan is identical across callsites.
pub fn was_affirmative(raw: &str, labels: &[&str]) -> bool {
    let v: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if v.get("timedOut").and_then(|t| t.as_bool()).unwrap_or(false) {
        return false;
    }
    let Some(answers) = v.get("answers").and_then(|a| a.as_array()) else {
        return false;
    };
    let needles: Vec<String> = labels
        .iter()
        .map(|s| s.trim().to_ascii_lowercase())
        .collect();
    for a in answers {
        let Some(selected) = a.get("selected").and_then(|s| s.as_array()) else {
            continue;
        };
        for sel in selected {
            if let Some(s) = sel.as_str() {
                let lower = s.trim().to_ascii_lowercase();
                if needles.iter().any(|n| n == &lower) {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::was_affirmative;

    #[test]
    fn matches_exact_label() {
        let raw = r#"{"answers":[{"question":"Run?","selected":["Run it"],"customInput":null}]}"#;
        assert!(was_affirmative(raw, &["Run it"]));
    }

    #[test]
    fn matches_case_insensitive_and_trimmed() {
        let raw =
            r#"{"answers":[{"question":"x","selected":["  UPGRADE NOW  "],"customInput":null}]}"#;
        assert!(was_affirmative(raw, &["upgrade now"]));
    }

    #[test]
    fn multi_label_either_match() {
        let raw =
            r#"{"answers":[{"question":"x","selected":["Roll back now"],"customInput":null}]}"#;
        assert!(was_affirmative(raw, &["Upgrade now", "Roll back now"]));
    }

    #[test]
    fn rejects_other_label() {
        let raw = r#"{"answers":[{"question":"x","selected":["Cancel"],"customInput":null}]}"#;
        assert!(!was_affirmative(raw, &["Run it"]));
    }

    #[test]
    fn rejects_timed_out_even_with_matching_label() {
        // Defensive: if the question times out *and* the default happens
        // to be the affirmative label, still treat as deny.
        let raw = r#"{"answers":[{"question":"x","selected":["Run it"],"customInput":null}],"timedOut":true}"#;
        assert!(!was_affirmative(raw, &["Run it"]));
    }

    #[test]
    fn rejects_garbage_input() {
        assert!(!was_affirmative("", &["x"]));
        assert!(!was_affirmative(
            "Error: no session context available",
            &["x"]
        ));
        assert!(!was_affirmative(r#"{"answers":[]}"#, &["x"]));
    }

    #[test]
    fn rejects_value_string_not_label() {
        // Defence-in-depth: `selected` carries the LABEL, never the value;
        // matching against the value (`"confirm"`) must NOT trigger
        // affirmation.
        let raw = r#"{"answers":[{"question":"x","selected":["confirm"],"customInput":null}]}"#;
        assert!(!was_affirmative(raw, &["Run it"]));
    }
}
