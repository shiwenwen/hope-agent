use std::collections::HashMap;

use super::types::{FacetSummary, Outcome, SessionFacet};

const MAX_TOP: usize = 8;

/// Roll up per-session facets into compact histograms used by both the
/// AI section prompts and the HTML renderer.
pub fn roll_up(facets: &[SessionFacet]) -> FacetSummary {
    let mut summary = FacetSummary::default();
    summary.total_facets = facets.len() as u32;

    let mut goal = HashMap::<String, u32>::new();
    let mut outcome = HashMap::<String, u32>::new();
    let mut session_type = HashMap::<String, u32>::new();
    let mut friction_buckets: HashMap<String, u32> = HashMap::new();
    let mut satisfaction = HashMap::<u8, u32>::new();
    let mut user_instructions = HashMap::<String, u32>::new();
    let mut friction_examples = Vec::new();
    let mut success_examples = Vec::new();

    for f in facets {
        for cat in &f.goal_categories {
            *goal.entry(cat.to_string()).or_insert(0) += 1;
        }
        let o = match f.outcome {
            Outcome::FullyAchieved => "fully_achieved",
            Outcome::MostlyAchieved => "mostly_achieved",
            Outcome::Partial => "partial",
            Outcome::Failed => "failed",
            Outcome::Unclear => "unclear",
        };
        *outcome.entry(o.to_string()).or_insert(0) += 1;

        if !f.session_type.is_empty() {
            *session_type.entry(f.session_type.clone()).or_insert(0) += 1;
        }

        let counts = &f.friction_counts;
        if counts.tool_errors > 0 {
            *friction_buckets.entry("tool_errors".into()).or_insert(0) += counts.tool_errors;
        }
        if counts.misunderstanding > 0 {
            *friction_buckets
                .entry("misunderstanding".into())
                .or_insert(0) += counts.misunderstanding;
        }
        if counts.repetition > 0 {
            *friction_buckets.entry("repetition".into()).or_insert(0) += counts.repetition;
        }
        if counts.user_correction > 0 {
            *friction_buckets
                .entry("user_correction".into())
                .or_insert(0) += counts.user_correction;
        }
        if counts.stuck > 0 {
            *friction_buckets.entry("stuck".into()).or_insert(0) += counts.stuck;
        }
        if counts.other > 0 {
            *friction_buckets.entry("other".into()).or_insert(0) += counts.other;
        }

        if let Some(s) = f.user_satisfaction {
            *satisfaction.entry(s).or_insert(0) += 1;
        }

        for inst in &f.user_instructions {
            let key = inst.trim().to_lowercase();
            if key.is_empty() {
                continue;
            }
            *user_instructions.entry(key).or_insert(0) += 1;
        }

        if let Some(s) = &f.primary_success {
            if !s.is_empty() && success_examples.len() < 12 {
                success_examples.push(s.clone());
            }
        }
        for d in &f.friction_detail {
            if friction_examples.len() < 12 {
                friction_examples.push(d.clone());
            }
        }
    }

    summary.goal_histogram = top_n_map(goal, MAX_TOP);
    summary.outcome_distribution = top_n_map(outcome, MAX_TOP);
    summary.session_type_distribution = top_n_map(session_type, MAX_TOP);
    summary.friction_top = top_n_map(friction_buckets, MAX_TOP);

    let mut satisfaction_vec: Vec<(u8, u32)> = satisfaction.into_iter().collect();
    satisfaction_vec.sort_by_key(|(k, _)| *k);
    summary.satisfaction_distribution = satisfaction_vec;

    summary.repeat_user_instructions = top_n_map(
        user_instructions
            .into_iter()
            .filter(|(_, n)| *n >= 2)
            .collect(),
        MAX_TOP,
    );

    summary.success_examples = success_examples;
    summary.friction_examples = friction_examples;
    summary
}

fn top_n_map(map: HashMap<String, u32>, n: usize) -> Vec<(String, u32)> {
    let mut v: Vec<(String, u32)> = map.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.truncate(n);
    v
}
