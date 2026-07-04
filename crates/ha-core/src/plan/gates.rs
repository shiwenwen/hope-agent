//! Phase 2.3 quality gates for natural-language plans and workflow script drafts.
//!
//! These gates are intentionally pure and deterministic. Plan submission can
//! fail fast with actionable feedback, while the future workflow runtime can
//! reuse the script gate before trusting or executing a draft.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateIssue {
    pub severity: GateSeverity,
    pub code: &'static str,
    pub message: String,
    pub suggestion: String,
}

impl GateIssue {
    fn error(code: &'static str, message: &str, suggestion: &str) -> Self {
        Self {
            severity: GateSeverity::Error,
            code,
            message: message.to_string(),
            suggestion: suggestion.to_string(),
        }
    }

    fn warning(code: &'static str, message: &str, suggestion: &str) -> Self {
        Self {
            severity: GateSeverity::Warning,
            code,
            message: message.to_string(),
            suggestion: suggestion.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateReport {
    pub issues: Vec<GateIssue>,
}

impl GateReport {
    pub fn passed(&self) -> bool {
        !self
            .issues
            .iter()
            .any(|issue| issue.severity == GateSeverity::Error)
    }

    pub fn has_warnings(&self) -> bool {
        self.issues
            .iter()
            .any(|issue| issue.severity == GateSeverity::Warning)
    }

    pub fn render_feedback(&self, gate_name: &str) -> String {
        if self.issues.is_empty() {
            return format!("{gate_name} passed.");
        }
        let mut out = format!("{gate_name} failed with actionable feedback:\n");
        for issue in &self.issues {
            let label = match issue.severity {
                GateSeverity::Error => "ERROR",
                GateSeverity::Warning => "WARN",
            };
            out.push_str(&format!(
                "- [{label}:{}] {} Fix: {}\n",
                issue.code, issue.message, issue.suggestion
            ));
        }
        out.trim_end().to_string()
    }
}

pub fn check_plan_quality(content: &str) -> GateReport {
    let mut issues = Vec::new();
    let trimmed = content.trim();
    if trimmed.len() < 80 {
        issues.push(GateIssue::error(
            "plan_too_short",
            "Plan content is too short to be execution-ready.",
            "Add concise Context, Steps, Critical Files/Files for code tasks, and Verification sections.",
        ));
    }

    if !has_heading(trimmed, &["context", "上下文", "背景"]) {
        issues.push(GateIssue::error(
            "missing_context",
            "Plan is missing a Context section.",
            "Add a short Context section explaining the chosen approach and why it solves the task.",
        ));
    }

    if !has_heading(
        trimmed,
        &[
            "steps",
            "approach",
            "implementation",
            "步骤",
            "方案",
            "实施",
        ],
    ) {
        issues.push(GateIssue::error(
            "missing_steps",
            "Plan is missing execution steps.",
            "Add a Steps section with concrete ordered work items.",
        ));
    }

    if !has_heading(trimmed, &["verification", "验证", "校验", "验收"]) {
        issues.push(GateIssue::error(
            "missing_verification",
            "Plan is missing a Verification section.",
            "Add concrete verification commands, manual checks, or review criteria.",
        ));
    }

    if looks_like_code_plan(trimmed)
        && !has_heading(trimmed, &["critical files", "files", "关键文件", "文件"])
    {
        issues.push(GateIssue::error(
            "missing_critical_files",
            "Code plan is missing a Critical Files / Files section.",
            "List the main files, modules, or paths expected to change or be inspected.",
        ));
    }

    if !has_heading(trimmed, &["reuse", "复用", "已有"]) {
        issues.push(GateIssue::warning(
            "missing_reuse",
            "Plan does not call out existing code or helpers to reuse.",
            "Add a Reuse section when the task depends on existing functions, modules, or contracts.",
        ));
    }

    if !has_heading(trimmed, &["risk", "risks", "风险"]) {
        issues.push(GateIssue::warning(
            "missing_risks",
            "Plan does not call out risks or edge cases.",
            "Add a Risks section for non-trivial changes, or explicitly state that risk is low.",
        ));
    }

    GateReport { issues }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ScriptGateOptions {
    pub autonomous: bool,
}

pub fn check_workflow_script_draft(script: &str, options: ScriptGateOptions) -> GateReport {
    let mut issues = Vec::new();
    let trimmed = script.trim();
    let compact = compact_ascii(trimmed);
    let lower = trimmed.to_ascii_lowercase();

    if trimmed.len() < 80 {
        issues.push(GateIssue::error(
            "script_too_short",
            "Workflow script draft is too short to review safely.",
            "Provide a complete workflow.js draft with tasks, bounded work, validation, and finish output.",
        ));
    }

    for (needle, replacement) in [
        (
            "eval(",
            "Remove eval and express behavior through host APIs.",
        ),
        ("Function(", "Remove Function constructor usage."),
        (
            "import(",
            "Remove dynamic import; workflow scripts cannot import arbitrary modules.",
        ),
        ("Date.now(", "Use workflow.now()."),
        ("Math.random(", "Use workflow.random(seed)."),
        ("new Date()", "Use workflow.now()."),
    ] {
        if trimmed.contains(needle) {
            issues.push(GateIssue::error(
                "nondeterministic_or_dynamic_js",
                &format!("Script uses forbidden JavaScript construct `{needle}`."),
                replacement,
            ));
        }
    }

    for (needle, label) in [
        ("require(", "require"),
        ("process.", "process"),
        ("child_process", "child_process"),
        ("fs.", "raw fs"),
        ("fetch(", "raw fetch"),
        ("XMLHttpRequest", "XMLHttpRequest"),
        ("Deno.", "Deno"),
    ] {
        if trimmed.contains(needle) {
            issues.push(GateIssue::error(
                "raw_capability",
                &format!("Script attempts to use raw capability `{label}`."),
                "Use the workflow host API instead; raw filesystem, process, network, and environment access are not available.",
            ));
        }
    }

    for api in [
        "workflow.tool(",
        "workflow.fileSearch(",
        "workflow.read(",
        "workflow.grep(",
        "workflow.spawnAgent(",
        "workflow.validate(",
        "workflow.review(",
        "workflow.verify(",
        "workflow.repairLoop(",
        "workflow.evidence.record(",
        "workflow.block(",
        "workflow.askUser(",
        "workflow.trace(",
        "workflow.diff(",
        "workflow.task.create(",
        "workflow.task.update(",
    ] {
        let legacy = format!("{}\"", api);
        if compact.contains(&legacy) {
            issues.push(GateIssue::error(
                "legacy_host_call_id",
                &format!("Script uses legacy host API shape `{api}\"id\", ...)`."),
                "Use object arguments like workflow.fileSearch({ query, label }) and treat label as display-only.",
            ));
        }
    }

    if compact.contains("workflow.task.update({label:") {
        issues.push(GateIssue::error(
            "task_update_by_label",
            "Script updates a task by label.",
            "Keep the handle returned by workflow.task.create(...) and call workflow.task.update({ task: handle, status }).",
        ));
    }

    if !compact.contains("workflow.task.create(") || !compact.contains("workflow.task.update(") {
        issues.push(GateIssue::error(
            "missing_task_truth",
            "Script does not use task.create/task.update as progress truth.",
            "Create user-visible task handles and update them as the workflow advances.",
        ));
    }

    if !compact.contains("workflow.finish(") {
        issues.push(GateIssue::error(
            "missing_finish",
            "Script does not finish through workflow.finish(...).",
            "Return a structured final result with summary, changed files, verification, and residual risk.",
        ));
    }

    if !compact.contains("workflow.validate(")
        && !compact.contains("workflow.verify(")
        && !compact.contains("workflow.repairLoop(")
    {
        issues.push(GateIssue::warning(
            "missing_validate",
            "Script has no targeted validation step.",
            "Add workflow.validate({ commands, reason, label }) to execute checks, workflow.verify({ focusPaths }) to create a Smart Verification plan, or workflow.repairLoop(...) for a bounded repair loop.",
        ));
    }

    if lacks_budget_hint(&lower) {
        issues.push(GateIssue::warning(
            "missing_budget",
            "Script does not mention a runtime, op, subagent, or repair budget.",
            "Add explicit bounded work controls such as max runtime, max ops, max subagents, or max repair attempts.",
        ));
    }

    if options.autonomous && (!has_output_token_budget(&lower) || !has_runtime_budget(&lower)) {
        issues.push(GateIssue::error(
            "autonomous_budget_required",
            "Autonomous workflow scripts require explicit token and runtime budgets.",
            "Set max_output_tokens/maxOutputTokens and max_runtime_secs/maxRuntimeSecs before allowing autonomous execution.",
        ));
    }

    GateReport { issues }
}

fn has_heading(markdown: &str, aliases: &[&str]) -> bool {
    markdown.lines().any(|line| {
        let trimmed = line.trim_start();
        if !trimmed.starts_with('#') {
            return false;
        }
        let heading = trimmed
            .trim_start_matches('#')
            .trim()
            .trim_matches(|c: char| matches!(c, ':' | '-' | '—'))
            .to_ascii_lowercase();
        aliases.iter().any(|alias| heading.contains(alias))
    })
}

fn looks_like_code_plan(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    lower.contains("code")
        || lower.contains("implementation")
        || lower.contains("implement")
        || lower.contains("bug")
        || lower.contains("refactor")
        || lower.contains("module")
        || lower.contains("crate")
        || lower.contains("component")
        || lower.contains(".rs")
        || lower.contains(".ts")
        || lower.contains(".tsx")
        || lower.contains(".js")
        || lower.contains("代码")
        || lower.contains("实现")
        || lower.contains("修复")
        || lower.contains("重构")
}

fn compact_ascii(input: &str) -> String {
    input.chars().filter(|c| !c.is_whitespace()).collect()
}

fn lacks_budget_hint(lower: &str) -> bool {
    ![
        "budget",
        "max runtime",
        "max_runtime",
        "max ops",
        "max_ops",
        "max subagents",
        "max_subagents",
        "max repair",
        "预算",
        "上限",
        "停止条件",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn has_output_token_budget(lower: &str) -> bool {
    lower.contains("max_output_tokens") || lower.contains("maxoutputtokens")
}

fn has_runtime_budget(lower: &str) -> bool {
    lower.contains("max_runtime") || lower.contains("maxruntime") || lower.contains("maxscripts")
}

#[cfg(test)]
mod tests {
    use super::{check_plan_quality, check_workflow_script_draft, GateSeverity, ScriptGateOptions};

    fn has_error(report: &super::GateReport, code: &str) -> bool {
        report
            .issues
            .iter()
            .any(|issue| issue.severity == GateSeverity::Error && issue.code == code)
    }

    #[test]
    fn plan_gate_blocks_code_plan_without_critical_files() {
        let plan = r#"
## Context
Implement a small code change in the search pipeline.

## Steps
1. Update the scorer to account for exact filename matches.

## Verification
cargo check -p ha-core
"#;

        let report = check_plan_quality(plan);
        assert!(!report.passed());
        assert!(has_error(&report, "missing_critical_files"));
    }

    #[test]
    fn plan_gate_blocks_missing_verification() {
        let plan = r#"
## Context
Implement a small code change in the search pipeline.

## Critical Files
- crates/ha-core/src/filesystem/search.rs

## Steps
1. Update the scorer to account for exact filename matches.
"#;

        let report = check_plan_quality(plan);
        assert!(!report.passed());
        assert!(has_error(&report, "missing_verification"));
    }

    #[test]
    fn plan_gate_accepts_execution_ready_code_plan_with_warnings() {
        let plan = r#"
## Context
Implement a small code change in the search pipeline.

## Critical Files
- crates/ha-core/src/filesystem/search.rs

## Steps
1. Update the scorer to account for exact filename matches.

## Verification
cargo check -p ha-core
"#;

        let report = check_plan_quality(plan);
        assert!(report.passed(), "{}", report.render_feedback("Plan Gate"));
        assert!(report.has_warnings());
    }

    #[test]
    fn script_gate_blocks_legacy_host_call_id_and_task_label_identity() {
        let script = r#"
export default async function main(workflow) {
  await workflow.task.create({ label: "observe", title: "Observe" });
  const files = await workflow.fileSearch("find-files", { query: "search" });
  await workflow.task.update({ label: "observe", status: "completed" });
  return workflow.finish({ summary: "done" });
}
"#;

        let report = check_workflow_script_draft(script, ScriptGateOptions::default());
        assert!(!report.passed());
        assert!(has_error(&report, "legacy_host_call_id"));
        assert!(has_error(&report, "task_update_by_label"));
    }

    #[test]
    fn script_gate_blocks_nondeterminism_and_raw_capabilities() {
        let script = r#"
export default async function main(workflow) {
  const task = await workflow.task.create({ title: "Run" });
  const now = Date.now();
  const data = await fetch("https://example.com");
  await workflow.task.update({ task, status: "completed" });
  return workflow.finish({ now, data });
}
"#;

        let report = check_workflow_script_draft(script, ScriptGateOptions::default());
        assert!(!report.passed());
        assert!(has_error(&report, "nondeterministic_or_dynamic_js"));
        assert!(has_error(&report, "raw_capability"));
    }

    #[test]
    fn script_gate_accepts_position_key_object_api_shape() {
        let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 300, max_ops: 20 };
  const observe = await workflow.task.create({ title: "Observe", label: "observe" });
  const files = await workflow.fileSearch({ query: "search", label: "find-files" });
  await workflow.validate({
    commands: ["cargo check -p ha-core"],
    reason: "search scorer changed",
    label: "targeted-check"
  });
  await workflow.task.update({ task: observe, status: "completed" });
  return workflow.finish({ summary: "done", files, budget });
}
"#;

        let report = check_workflow_script_draft(script, ScriptGateOptions::default());
        assert!(report.passed(), "{}", report.render_feedback("Script Gate"));
    }

    #[test]
    fn script_gate_accepts_review_and_verify_host_apis() {
        let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 300, max_ops: 20 };
  const observe = await workflow.task.create({ title: "Observe", label: "observe" });
  const review = await workflow.review({
    focusPaths: ["crates/ha-core/src/workflow/runtime.rs"],
    label: "focused-review"
  });
  const verification = await workflow.verify({
    focusPaths: ["crates/ha-core/src/workflow/runtime.rs"],
    maxCommands: 2,
    label: "focused-verify"
  });
  await workflow.task.update({ task: observe, status: "completed" });
  return workflow.finish({ summary: "done", review, verification, budget });
}
"#;

        let report = check_workflow_script_draft(script, ScriptGateOptions::default());
        assert!(report.passed(), "{}", report.render_feedback("Script Gate"));
        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "missing_validate"));
    }

    #[test]
    fn script_gate_accepts_repair_loop_host_api() {
        let script = r#"
export default async function main(workflow) {
  const budget = { max_runtime_secs: 300, max_ops: 20, max_repair_attempts: 2 };
  const observe = await workflow.task.create({ title: "Repair", label: "repair" });
  const repair = await workflow.repairLoop({
    label: "focused-repair",
    maxAttempts: 2,
    validationCommands: ["cargo check -p ha-core --locked"],
    focusPaths: ["crates/ha-core/src/workflow/runtime.rs"],
  }, async ({ attempt }) => {
    await workflow.trace({ label: "repair-attempt", payload: { attempt } });
  });
  await workflow.task.update({ task: observe, status: "completed" });
  return workflow.finish({ summary: "done", repair, budget });
}
"#;

        let report = check_workflow_script_draft(script, ScriptGateOptions::default());
        assert!(report.passed(), "{}", report.render_feedback("Script Gate"));
        assert!(!report
            .issues
            .iter()
            .any(|issue| issue.code == "missing_validate"));
    }

    #[test]
    fn script_gate_autonomous_accepts_camel_case_budget_keys() {
        let script = r#"
export default async function main(workflow) {
  const budget = { maxScriptSecs: 300, maxOps: 24, maxOutputTokens: 12000 };
  const observe = await workflow.task.create({ title: "Observe", label: "observe" });
  await workflow.validate({
    commands: ["pnpm typecheck"],
    reason: "targeted validation",
    label: "targeted-check"
  });
  await workflow.task.update({ task: observe, status: "completed" });
  return workflow.finish({ summary: "done", budget });
}
"#;

        let report = check_workflow_script_draft(script, ScriptGateOptions { autonomous: true });
        assert!(report.passed(), "{}", report.render_feedback("Script Gate"));
    }

    #[test]
    fn script_gate_autonomous_requires_output_token_budget() {
        let script = r#"
export default async function main(workflow) {
  const budget = { maxScriptSecs: 300, maxOps: 24 };
  const observe = await workflow.task.create({ title: "Observe", label: "observe" });
  await workflow.validate({
    commands: ["pnpm typecheck"],
    reason: "targeted validation",
    label: "targeted-check"
  });
  await workflow.task.update({ task: observe, status: "completed" });
  return workflow.finish({ summary: "done", budget });
}
"#;

        let report = check_workflow_script_draft(script, ScriptGateOptions { autonomous: true });
        assert!(!report.passed());
        assert!(has_error(&report, "autonomous_budget_required"));
    }
}
