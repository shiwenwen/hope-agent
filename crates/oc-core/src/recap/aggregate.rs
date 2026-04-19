use std::collections::HashMap;

use super::types::{FacetSummary, Outcome, SessionFacet};

const MAX_TOP: usize = 8;

/// Roll up per-session facets into compact histograms used by both the
/// AI section prompts and the HTML renderer.
pub fn roll_up(facets: &[SessionFacet]) -> FacetSummary {
    let mut summary = FacetSummary {
        total_facets: facets.len() as u32,
        ..FacetSummary::default()
    };

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recap::types::FrictionCounts;

    fn facet(id: &str) -> SessionFacet {
        SessionFacet {
            session_id: id.to_string(),
            underlying_goal: String::new(),
            goal_categories: Vec::new(),
            outcome: Outcome::Unclear,
            user_satisfaction: None,
            agent_helpfulness: None,
            session_type: String::new(),
            friction_counts: FrictionCounts::default(),
            friction_detail: Vec::new(),
            primary_success: None,
            brief_summary: String::new(),
            user_instructions: Vec::new(),
        }
    }

    #[test]
    fn roll_up_empty_returns_all_zeroes() {
        let summary = roll_up(&[]);
        assert_eq!(summary.total_facets, 0);
        assert!(summary.goal_histogram.is_empty());
        assert!(summary.outcome_distribution.is_empty());
        assert!(summary.friction_top.is_empty());
        assert!(summary.satisfaction_distribution.is_empty());
        assert!(summary.success_examples.is_empty());
    }

    #[test]
    fn outcome_histogram_buckets_by_variant() {
        let facets = vec![
            SessionFacet {
                outcome: Outcome::FullyAchieved,
                ..facet("a")
            },
            SessionFacet {
                outcome: Outcome::FullyAchieved,
                ..facet("b")
            },
            SessionFacet {
                outcome: Outcome::Failed,
                ..facet("c")
            },
        ];
        let s = roll_up(&facets);
        assert_eq!(s.total_facets, 3);
        assert_eq!(
            s.outcome_distribution
                .iter()
                .find(|(k, _)| k == "fully_achieved")
                .map(|(_, n)| *n),
            Some(2)
        );
        assert_eq!(
            s.outcome_distribution
                .iter()
                .find(|(k, _)| k == "failed")
                .map(|(_, n)| *n),
            Some(1)
        );
    }

    #[test]
    fn outcome_histogram_sorted_by_count_desc() {
        let facets = vec![
            SessionFacet {
                outcome: Outcome::Failed,
                ..facet("a")
            },
            SessionFacet {
                outcome: Outcome::MostlyAchieved,
                ..facet("b")
            },
            SessionFacet {
                outcome: Outcome::MostlyAchieved,
                ..facet("c")
            },
            SessionFacet {
                outcome: Outcome::MostlyAchieved,
                ..facet("d")
            },
        ];
        let s = roll_up(&facets);
        assert_eq!(s.outcome_distribution[0].0, "mostly_achieved");
        assert_eq!(s.outcome_distribution[0].1, 3);
    }

    #[test]
    fn goal_histogram_accumulates_across_facets() {
        let facets = vec![
            SessionFacet {
                goal_categories: vec!["code".into(), "debug".into()],
                ..facet("a")
            },
            SessionFacet {
                goal_categories: vec!["code".into()],
                ..facet("b")
            },
        ];
        let s = roll_up(&facets);
        assert_eq!(
            s.goal_histogram
                .iter()
                .find(|(k, _)| k == "code")
                .map(|(_, n)| *n),
            Some(2)
        );
        assert_eq!(
            s.goal_histogram
                .iter()
                .find(|(k, _)| k == "debug")
                .map(|(_, n)| *n),
            Some(1)
        );
    }

    #[test]
    fn friction_counts_roll_into_buckets() {
        let facets = vec![
            SessionFacet {
                friction_counts: FrictionCounts {
                    tool_errors: 3,
                    misunderstanding: 1,
                    ..Default::default()
                },
                ..facet("a")
            },
            SessionFacet {
                friction_counts: FrictionCounts {
                    tool_errors: 2,
                    stuck: 4,
                    ..Default::default()
                },
                ..facet("b")
            },
        ];
        let s = roll_up(&facets);
        assert_eq!(
            s.friction_top
                .iter()
                .find(|(k, _)| k == "tool_errors")
                .map(|(_, n)| *n),
            Some(5)
        );
        assert_eq!(
            s.friction_top
                .iter()
                .find(|(k, _)| k == "stuck")
                .map(|(_, n)| *n),
            Some(4)
        );
        assert!(!s.friction_top.iter().any(|(k, _)| k == "repetition"));
    }

    #[test]
    fn satisfaction_distribution_sorted_ascending_by_score() {
        let facets = vec![
            SessionFacet {
                user_satisfaction: Some(5),
                ..facet("a")
            },
            SessionFacet {
                user_satisfaction: Some(2),
                ..facet("b")
            },
            SessionFacet {
                user_satisfaction: Some(5),
                ..facet("c")
            },
            SessionFacet {
                user_satisfaction: None,
                ..facet("d")
            },
        ];
        let s = roll_up(&facets);
        let scores: Vec<u8> = s.satisfaction_distribution.iter().map(|(k, _)| *k).collect();
        assert_eq!(scores, vec![2, 5]);
        assert_eq!(s.satisfaction_distribution[1].1, 2);
    }

    #[test]
    fn success_examples_capped_at_twelve() {
        let facets: Vec<SessionFacet> = (0..20)
            .map(|i| SessionFacet {
                primary_success: Some(format!("win {}", i)),
                ..facet(&format!("s{}", i))
            })
            .collect();
        let s = roll_up(&facets);
        assert_eq!(s.success_examples.len(), 12);
        assert_eq!(s.success_examples[0], "win 0");
    }

    #[test]
    fn friction_examples_capped_at_twelve_across_facets() {
        let facets: Vec<SessionFacet> = (0..5)
            .map(|i| SessionFacet {
                friction_detail: (0..5).map(|j| format!("f{}-{}", i, j)).collect(),
                ..facet(&format!("s{}", i))
            })
            .collect();
        let s = roll_up(&facets);
        assert_eq!(s.friction_examples.len(), 12);
    }

    #[test]
    fn repeat_user_instructions_drops_singletons_and_lowercases() {
        let facets = vec![
            SessionFacet {
                user_instructions: vec!["Be concise".into(), "Keep going".into()],
                ..facet("a")
            },
            SessionFacet {
                user_instructions: vec!["be concise".into()],
                ..facet("b")
            },
            SessionFacet {
                user_instructions: vec!["be concise".into()],
                ..facet("c")
            },
        ];
        let s = roll_up(&facets);
        assert_eq!(
            s.repeat_user_instructions
                .iter()
                .find(|(k, _)| k == "be concise")
                .map(|(_, n)| *n),
            Some(3)
        );
        // "keep going" only appears once → dropped by `>= 2` filter.
        assert!(!s
            .repeat_user_instructions
            .iter()
            .any(|(k, _)| k == "keep going"));
    }

    #[test]
    fn empty_session_type_does_not_pollute_distribution() {
        let facets = vec![facet("a"), facet("b")];
        let s = roll_up(&facets);
        assert!(s.session_type_distribution.is_empty());
    }
}
