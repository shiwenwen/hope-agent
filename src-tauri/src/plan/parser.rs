use super::types::{PlanStep, PlanStepStatus};

// ── Markdown Checklist Parser ───────────────────────────────────

/// Parse a markdown plan into structured PlanStep items.
/// Expected format:
/// ```text
/// ### Phase 1: Analysis
/// - [ ] Read config files
/// - [x] Analyze CSS variables
/// ### Phase 2: Implementation
/// - [ ] Add ThemeProvider
/// ```
pub fn parse_plan_steps(markdown: &str) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let mut current_phase = String::new();
    let mut index = 0;

    for line in markdown.lines() {
        let trimmed = line.trim();

        // Match phase headers: "### Phase N: title" or "### title"
        if trimmed.starts_with("### ") {
            current_phase = trimmed.trim_start_matches("### ").to_string();
            continue;
        }

        // Match checklist items: "- [ ] text" or "- [x] text"
        if let Some(rest) = trimmed.strip_prefix("- [") {
            let (checked, text) = if let Some(t) = rest
                .strip_prefix("x] ")
                .or_else(|| rest.strip_prefix("X] "))
            {
                (true, t)
            } else if let Some(t) = rest.strip_prefix(" ] ") {
                (false, t)
            } else {
                continue;
            };

            let status = if checked {
                PlanStepStatus::Completed
            } else {
                PlanStepStatus::Pending
            };

            steps.push(PlanStep {
                index,
                phase: current_phase.clone(),
                title: text.to_string(),
                description: String::new(),
                status,
                duration_ms: None,
            });
            index += 1;
        }
    }

    steps
}
