//! Render a `AwarenessSnapshot` as a markdown block for the system prompt.

use crate::truncate_utf8;

use super::types::{ActivityState, AwarenessEntry, AwarenessSnapshot};

const FIELD_BUDGET: usize = 120;
const HEADER: &str = "# Cross-Session Context";

/// Render the snapshot to markdown. Returns `None` when there are no entries.
/// The output is capped at `max_chars` bytes (UTF-8 safe).
pub fn render_markdown(snap: &AwarenessSnapshot, max_chars: usize) -> Option<String> {
    if snap.entries.is_empty() {
        return None;
    }

    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str("\n\n");
    out.push_str(&render_intro(snap));
    out.push_str("\n\n");

    // Group by activity.
    let active: Vec<&AwarenessEntry> = snap
        .entries
        .iter()
        .filter(|e| e.activity == ActivityState::Active)
        .collect();
    let recent: Vec<&AwarenessEntry> = snap
        .entries
        .iter()
        .filter(|e| e.activity == ActivityState::Recent)
        .collect();
    let older: Vec<&AwarenessEntry> = snap
        .entries
        .iter()
        .filter(|e| e.activity == ActivityState::Older)
        .collect();

    if !active.is_empty() {
        out.push_str("## Currently active\n");
        for e in active {
            out.push_str(&render_entry(e));
            out.push('\n');
        }
        out.push('\n');
    }
    if !recent.is_empty() {
        out.push_str("## Recent (last hour)\n");
        for e in recent {
            out.push_str(&render_entry(e));
            out.push('\n');
        }
        out.push('\n');
    }
    if !older.is_empty() {
        out.push_str("## Earlier (within lookback)\n");
        for e in older {
            out.push_str(&render_entry(e));
            out.push('\n');
        }
        out.push('\n');
    }

    let trimmed = out.trim_end().to_string();
    Some(truncate_utf8(&trimmed, max_chars).to_string())
}

fn render_intro(snap: &AwarenessSnapshot) -> String {
    let total = snap.entries.len();
    let active = snap.active_count;
    format!(
        "The user has {} other relevant session(s) ({} currently active). Use this to \
understand references like \"the thing I was working on earlier\" and to avoid \
re-asking for context established elsewhere. Do NOT assume actions taken there are \
visible here unless the user confirms.",
        total, active
    )
}

fn render_entry(e: &AwarenessEntry) -> String {
    let mut line = String::new();
    line.push_str("- **");
    line.push_str(&truncate_utf8(&e.title, FIELD_BUDGET));
    line.push_str("**");
    if let Some(name) = &e.agent_name {
        line.push_str(" · ");
        line.push_str(name);
    }
    line.push_str(" · ");
    line.push_str(e.session_kind.as_str());
    line.push_str(" · ");
    line.push_str(&format_age(e.age_secs));

    if let Some(goal) = &e.underlying_goal {
        line.push_str("\n  goal: ");
        line.push_str(&truncate_utf8(goal, FIELD_BUDGET));
    }
    if let Some(outcome) = &e.outcome {
        line.push_str("; outcome: ");
        line.push_str(outcome);
    }
    if let Some(summary) = &e.brief_summary {
        line.push_str("\n  summary: ");
        line.push_str(&truncate_utf8(summary, FIELD_BUDGET));
    } else if let Some(preview) = &e.fallback_preview {
        line.push_str("\n  preview: ");
        line.push_str(&truncate_utf8(preview, FIELD_BUDGET));
    }
    line
}

/// Format age as a **coarse bucket** so that the rendered suffix stays
/// byte-identical across consecutive turns when nothing substantive changed.
/// Without this, "45s ago" → "67s ago" defeats the hash-based reuse.
fn format_age(age_secs: i64) -> &'static str {
    if age_secs < 0 {
        "just now"
    } else if age_secs < 60 {
        "<1 min ago"
    } else if age_secs < 300 {
        "<5 min ago"
    } else if age_secs < 900 {
        "<15 min ago"
    } else if age_secs < 3600 {
        "<1 hour ago"
    } else if age_secs < 14400 {
        "<4 hours ago"
    } else if age_secs < 86400 {
        "<1 day ago"
    } else {
        ">1 day ago"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::awareness::types::SessionKind;

    fn mk_entry(id: &str, title: &str, activity: ActivityState, age: i64) -> AwarenessEntry {
        AwarenessEntry {
            session_id: id.into(),
            title: title.into(),
            agent_id: "default".into(),
            agent_name: Some("Coder".into()),
            session_kind: SessionKind::Regular,
            updated_at: "2025-01-01T00:00:00Z".into(),
            age_secs: age,
            activity,
            brief_summary: Some("ran pytest twice, still flaky".into()),
            underlying_goal: Some("find CI flakiness root cause".into()),
            outcome: Some("partial".into()),
            goal_categories: vec!["debugging".into()],
            fallback_preview: None,
        }
    }

    #[test]
    fn renders_header_and_entries() {
        let snap = AwarenessSnapshot {
            entries: vec![
                mk_entry("a", "Debug CI", ActivityState::Active, 45),
                mk_entry("b", "Payment webhook", ActivityState::Recent, 300),
            ],
            active_count: 1,
            generated_at: "now".into(),
        };
        let out = render_markdown(&snap, 4000).unwrap();
        assert!(out.contains("# Cross-Session Context"));
        assert!(out.contains("Debug CI"));
        assert!(out.contains("Payment webhook"));
        assert!(out.contains("Currently active"));
        assert!(out.contains("Recent (last hour)"));
    }

    #[test]
    fn empty_snapshot_returns_none() {
        let snap = AwarenessSnapshot {
            entries: vec![],
            active_count: 0,
            generated_at: "now".into(),
        };
        assert!(render_markdown(&snap, 1000).is_none());
    }

    #[test]
    fn max_chars_is_respected() {
        let snap = AwarenessSnapshot {
            entries: (0..10)
                .map(|i| mk_entry(&format!("{}", i), "Very long title that repeats A A A A A A A A A A", ActivityState::Active, i * 10))
                .collect(),
            active_count: 10,
            generated_at: "now".into(),
        };
        let out = render_markdown(&snap, 200).unwrap();
        assert!(out.len() <= 200, "output length {} exceeds 200", out.len());
    }

    #[test]
    fn age_formatting_coarse_buckets() {
        assert_eq!(format_age(30), "<1 min ago");
        assert_eq!(format_age(90), "<5 min ago");
        assert_eq!(format_age(600), "<15 min ago");
        assert_eq!(format_age(7200), "<4 hours ago");
        assert_eq!(format_age(90_000), ">1 day ago");
        // Consecutive values in the same bucket must return the same string.
        assert_eq!(format_age(10), format_age(55));
        assert_eq!(format_age(100), format_age(250));
    }
}
