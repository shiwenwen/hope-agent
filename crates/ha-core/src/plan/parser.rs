use super::types::{PlanStep, PlanStepStatus};

// ── Markdown Plan Parser ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Ordered,
    Unordered,
}

/// Parse a markdown plan into structured, coarse-grained PlanStep items.
///
/// New plans should use headings or regular ordered/unordered lists for major
/// execution steps. Checkbox items are intentionally treated as a legacy
/// fallback only, so detail lists inside a plan do not explode into dozens of
/// progress entries.
pub fn parse_plan_steps(markdown: &str) -> Vec<PlanStep> {
    let heading_steps = parse_heading_steps(markdown);
    if !heading_steps.is_empty() {
        return heading_steps;
    }

    let ordered_steps = parse_list_steps(markdown, ListKind::Ordered);
    if !ordered_steps.is_empty() {
        return ordered_steps;
    }

    let unordered_steps = parse_list_steps(markdown, ListKind::Unordered);
    if !unordered_steps.is_empty() {
        return unordered_steps;
    }

    parse_legacy_checklist_steps(markdown)
}

fn parse_heading_steps(markdown: &str) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let mut current_section = String::new();
    let mut index = 0;
    let mut in_code_fence = false;

    for line in markdown.lines() {
        let trimmed = line.trim();

        if toggle_code_fence(trimmed, &mut in_code_fence) || in_code_fence {
            continue;
        }

        if let Some((level, title)) = parse_heading(trimmed) {
            if level == 2 {
                if is_verification_heading(title) && !steps.is_empty() {
                    steps.push(PlanStep {
                        index,
                        phase: title.to_string(),
                        title: title.to_string(),
                        description: String::new(),
                        status: PlanStepStatus::Pending,
                        duration_ms: None,
                    });
                    index += 1;
                }
                current_section = title.to_string();
                continue;
            }

            if level == 3 && is_executable_heading(title, &current_section) {
                steps.push(PlanStep {
                    index,
                    phase: section_name(&current_section),
                    title: title.to_string(),
                    description: String::new(),
                    status: PlanStepStatus::Pending,
                    duration_ms: None,
                });
                index += 1;
            }
        }
    }

    steps
}

fn parse_list_steps(markdown: &str, kind: ListKind) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let mut current_section = String::new();
    let mut index = 0;
    let mut in_code_fence = false;

    for line in markdown.lines() {
        let trimmed = line.trim();

        if toggle_code_fence(trimmed, &mut in_code_fence) || in_code_fence {
            continue;
        }

        if let Some((level, title)) = parse_heading(trimmed) {
            if level <= 2 {
                current_section = title.to_string();
            }
            continue;
        }

        if !is_step_list_section(&current_section) || leading_whitespace(line) > 2 {
            continue;
        }

        let title = match kind {
            ListKind::Ordered => strip_ordered_marker(trimmed),
            ListKind::Unordered => strip_unordered_marker(trimmed),
        };

        if let Some(title) = title.filter(|s| !s.is_empty()) {
            steps.push(PlanStep {
                index,
                phase: section_name(&current_section),
                title,
                description: String::new(),
                status: PlanStepStatus::Pending,
                duration_ms: None,
            });
            index += 1;
        }
    }

    steps
}

fn parse_legacy_checklist_steps(markdown: &str) -> Vec<PlanStep> {
    let mut steps = Vec::new();
    let mut current_phase = String::new();
    let mut index = 0;
    let mut in_code_fence = false;

    for line in markdown.lines() {
        let trimmed = line.trim();

        if toggle_code_fence(trimmed, &mut in_code_fence) || in_code_fence {
            continue;
        }

        if let Some((level, title)) = parse_heading(trimmed) {
            if level <= 3 {
                current_phase = title.to_string();
            }
            continue;
        }

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

fn parse_heading(trimmed: &str) -> Option<(usize, &str)> {
    let hashes = trimmed.chars().take_while(|&c| c == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }

    let rest = trimmed.get(hashes..)?;
    if !rest.starts_with(' ') {
        return None;
    }

    let title = rest.trim().trim_matches('#').trim();
    if title.is_empty() {
        None
    } else {
        Some((hashes, title))
    }
}

fn toggle_code_fence(trimmed: &str, in_code_fence: &mut bool) -> bool {
    if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
        *in_code_fence = !*in_code_fence;
        true
    } else {
        false
    }
}

fn normalize_heading(s: &str) -> String {
    s.to_ascii_lowercase()
}

fn is_context_heading(title: &str) -> bool {
    let title = normalize_heading(title);
    title.contains("context")
        || title.contains("background")
        || title.contains("overview")
        || title.contains("背景")
        || title.contains("上下文")
        || title.contains("概览")
}

fn is_step_list_section(section: &str) -> bool {
    if section.is_empty() {
        return true;
    }
    if is_context_heading(section) {
        return false;
    }

    let section = normalize_heading(section);
    section.contains("step")
        || section.contains("plan")
        || section.contains("implementation")
        || section.contains("execution")
        || section.contains("verify")
        || section.contains("verification")
        || section.contains("步骤")
        || section.contains("计划")
        || section.contains("方案")
        || section.contains("实施")
        || section.contains("执行")
        || section.contains("验证")
        || section.contains("验收")
}

fn is_verification_heading(title: &str) -> bool {
    let title = normalize_heading(title);
    title.contains("verify")
        || title.contains("verification")
        || title.contains("验证")
        || title.contains("验收")
}

fn is_executable_heading(title: &str, section: &str) -> bool {
    if is_context_heading(section) || is_context_heading(title) {
        return false;
    }

    let title_norm = normalize_heading(title);
    title_norm.starts_with("step ")
        || title_norm.starts_with("phase ")
        || title_norm.starts_with("verification")
        || title_norm.starts_with("verify")
        || title_norm.starts_with("步骤")
        || title_norm.starts_with("阶段")
        || title_norm.starts_with("验证")
        || title_norm.starts_with("验收")
        || is_step_list_section(section)
}

fn section_name(section: &str) -> String {
    if section.trim().is_empty() {
        "Steps".to_string()
    } else {
        section.to_string()
    }
}

fn leading_whitespace(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

fn strip_ordered_marker(trimmed: &str) -> Option<String> {
    let bytes = trimmed.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i == 0 || i >= bytes.len() || (bytes[i] != b'.' && bytes[i] != b')') {
        return None;
    }
    i += 1;
    if i >= bytes.len() || !bytes[i].is_ascii_whitespace() {
        return None;
    }
    Some(trimmed[i..].trim().to_string())
}

fn strip_unordered_marker(trimmed: &str) -> Option<String> {
    if trimmed.starts_with("- [") {
        return None;
    }
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return Some(rest.trim().to_string());
        }
    }
    None
}
