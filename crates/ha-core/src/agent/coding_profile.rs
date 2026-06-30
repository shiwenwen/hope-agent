//! Lightweight Coding Mode profile classification.
//!
//! Phase 2.2 deliberately starts with deterministic rules instead of a
//! side-query classifier: the profile changes per user turn, so it must stay
//! out of the static system-prompt prefix and it should be cheap enough to run
//! for every request.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodingTaskKind {
    General,
    Feature,
    Debug,
    Review,
    Verify,
    WorkflowScript,
}

impl CodingTaskKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::General => "general",
            Self::Feature => "feature",
            Self::Debug => "debug",
            Self::Review => "review",
            Self::Verify => "verify",
            Self::WorkflowScript => "workflow_script",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoopMode {
    LightCoding,
    PlanImplement,
    EvidenceDebug,
    ReviewOnly,
    VerifyOnly,
    WorkflowScript,
}

impl LoopMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::LightCoding => "light_coding",
            Self::PlanImplement => "plan_implement",
            Self::EvidenceDebug => "evidence_debug",
            Self::ReviewOnly => "review_only",
            Self::VerifyOnly => "verify_only",
            Self::WorkflowScript => "workflow_script",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodingSessionProfile {
    pub task_kind: CodingTaskKind,
    pub loop_mode: LoopMode,
    pub requires_plan: bool,
    pub requires_script: bool,
    pub requires_task_truth: bool,
    pub recommended_skills: Vec<&'static str>,
    pub verification_policy: &'static str,
    pub risk_level: &'static str,
    pub discipline: Vec<&'static str>,
}

impl CodingSessionProfile {
    pub(crate) fn classify(user_text: &str) -> Option<Self> {
        let text = normalize(user_text);
        if text.trim().is_empty() {
            return None;
        }

        let has_workflow = has_any(
            &text,
            &[
                "workflow.js",
                "workflow script",
                "dynamic workflow",
                "durable replay",
                "loop mode",
                "工作流",
                "动态工作流",
                "工作流脚本",
                "loop 模式",
            ],
        );
        if has_workflow {
            return Some(Self::for_kind(CodingTaskKind::WorkflowScript));
        }

        let has_review = has_any(
            &text,
            &[
                "code review",
                "review",
                "review 当前",
                "review my",
                "检查未提交",
                "检查我未提交",
                "检查当前改动",
                "检查更改",
                "代码审查",
                "复核",
                "审查",
            ],
        );
        if has_review {
            return Some(Self::for_kind(CodingTaskKind::Review));
        }

        let has_debug = has_any(
            &text,
            &[
                "debug",
                "diagnose",
                "root cause",
                "reproduce",
                "bug",
                "crash",
                "stack trace",
                "regression",
                "flaky",
                "failing test",
                "报错",
                "失败",
                "崩溃",
                "复现",
                "排查",
                "定位",
                "回归",
            ],
        );
        if has_debug {
            return Some(Self::for_kind(CodingTaskKind::Debug));
        }

        let has_verify = has_any(
            &text,
            &[
                "verify",
                "verification",
                "test plan",
                "what should we run",
                "是否完成",
                "还差什么",
                "验证",
                "测试什么",
                "收尾检查",
            ],
        );
        if has_verify {
            return Some(Self::for_kind(CodingTaskKind::Verify));
        }

        let has_feature = has_any(
            &text,
            &[
                "implement",
                "implementation",
                "feature",
                "add ",
                "build ",
                "fix ",
                "refactor",
                "optimize",
                "实现",
                "新增",
                "修复",
                "优化",
                "重构",
                "功能",
                "改代码",
                "完成phase",
                "完成 phase",
            ],
        );
        if has_feature {
            return Some(Self::for_kind(CodingTaskKind::Feature));
        }

        let has_general_coding = has_any(
            &text,
            &[
                "code", "coding", "commit", "branch", "diff", "repo", "代码", "编码", "提交",
                "分支",
            ],
        );
        has_general_coding.then(|| Self::for_kind(CodingTaskKind::General))
    }

    fn for_kind(task_kind: CodingTaskKind) -> Self {
        match task_kind {
            CodingTaskKind::Review => Self {
                task_kind,
                loop_mode: LoopMode::ReviewOnly,
                requires_plan: false,
                requires_script: false,
                requires_task_truth: false,
                recommended_skills: vec!["ha-code-review", "ha-verify"],
                verification_policy: "inspect the review target; run only cheap targeted checks if they materially improve confidence",
                risk_level: "medium",
                discipline: vec![
                    "Review-only mode: do not implement fixes unless the user explicitly asks for repair.",
                    "Findings first; prefer no finding over speculative feedback.",
                    "Tie each actionable issue to changed behavior and the smallest useful file/line reference.",
                ],
            },
            CodingTaskKind::Debug => Self {
                task_kind,
                loop_mode: LoopMode::EvidenceDebug,
                requires_plan: false,
                requires_script: false,
                requires_task_truth: true,
                recommended_skills: vec!["ha-debug", "ha-verify", "ha-coding-common"],
                verification_policy: "reproduce or characterize the failure first; verify with the narrowest regression check",
                risk_level: "medium",
                discipline: vec![
                    "Gather evidence before patching: failing output, logs, stack trace, or a minimal reproduction.",
                    "Patch the smallest credible root cause; avoid broad rewrites before proof.",
                    "State the targeted regression check and whether it ran.",
                ],
            },
            CodingTaskKind::Feature => Self {
                task_kind,
                loop_mode: LoopMode::PlanImplement,
                requires_plan: true,
                requires_script: false,
                requires_task_truth: true,
                recommended_skills: vec!["ha-coding-common", "ha-verify"],
                verification_policy: "define a targeted verification path before or during implementation; do not default to full suites",
                risk_level: "medium",
                discipline: vec![
                    "Start from the current code and produce a concise implementation plan when the change is non-trivial.",
                    "Track progress with task truth for multi-step work.",
                    "Keep edits scoped and finish with targeted verification evidence.",
                ],
            },
            CodingTaskKind::WorkflowScript => Self {
                task_kind,
                loop_mode: LoopMode::WorkflowScript,
                requires_plan: true,
                requires_script: true,
                requires_task_truth: true,
                recommended_skills: vec!["ha-workflow-script", "ha-verify", "ha-coding-common"],
                verification_policy: "review script gates, replay safety, stop conditions, and targeted validation commands",
                risk_level: "high",
                discipline: vec![
                    "Draft scripts as durable host-API orchestration, not raw fs/network/process code.",
                    "Use labels only for display; task updates must use handles and op identity must remain runtime-derived.",
                    "Keep repair loops runtime-controlled with explicit stop conditions.",
                ],
            },
            CodingTaskKind::Verify => Self {
                task_kind,
                loop_mode: LoopMode::VerifyOnly,
                requires_plan: false,
                requires_script: false,
                requires_task_truth: false,
                recommended_skills: vec!["ha-verify"],
                verification_policy: "map each requirement to direct evidence; run the smallest allowed checks",
                risk_level: "low",
                discipline: vec![
                    "Do not treat weak or indirect evidence as completion.",
                    "Choose checks based on changed behavior and project instructions.",
                    "Ask before full suites unless the repo requires them or the user requested them.",
                ],
            },
            CodingTaskKind::General => Self {
                task_kind,
                loop_mode: LoopMode::LightCoding,
                requires_plan: false,
                requires_script: false,
                requires_task_truth: true,
                recommended_skills: vec!["ha-coding-common", "ha-verify"],
                verification_policy: "use targeted verification that matches the touched surface",
                risk_level: "low",
                discipline: vec![
                    "Inspect existing code before editing.",
                    "Keep the diff narrow and preserve unrelated user changes.",
                    "Explain verification or why it was skipped.",
                ],
            },
        }
    }

    pub(crate) fn render_prompt_block(&self) -> String {
        let mut out = String::new();
        out.push_str("## Coding Session Profile\n\n");
        out.push_str("This is a per-turn coding policy hint. It does not override user instructions or project AGENTS.md.\n\n");
        out.push_str(&format!("- task_kind: {}\n", self.task_kind.as_str()));
        out.push_str(&format!("- loop_mode: {}\n", self.loop_mode.as_str()));
        out.push_str(&format!("- requires_plan: {}\n", self.requires_plan));
        out.push_str(&format!("- requires_script: {}\n", self.requires_script));
        out.push_str(&format!(
            "- requires_task_truth: {}\n",
            self.requires_task_truth
        ));
        out.push_str(&format!(
            "- recommended_skills: {}\n",
            self.recommended_skills.join(", ")
        ));
        out.push_str(&format!(
            "- verification_policy: {}\n",
            self.verification_policy
        ));
        out.push_str(&format!("- risk_level: {}\n", self.risk_level));
        out.push_str("- discipline:\n");
        for item in &self.discipline {
            out.push_str("  - ");
            out.push_str(item);
            out.push('\n');
        }
        out
    }
}

fn normalize(input: &str) -> String {
    input.to_ascii_lowercase()
}

fn has_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{CodingSessionProfile, CodingTaskKind, LoopMode};

    #[test]
    fn review_request_is_review_only() {
        let p = CodingSessionProfile::classify("请检查我未提交的更改").unwrap();
        assert_eq!(p.task_kind, CodingTaskKind::Review);
        assert_eq!(p.loop_mode, LoopMode::ReviewOnly);
        assert!(!p.requires_plan);
        assert!(p.render_prompt_block().contains("do not implement fixes"));
    }

    #[test]
    fn debug_request_requires_evidence() {
        let p = CodingSessionProfile::classify("这个测试失败了，帮我 debug").unwrap();
        assert_eq!(p.task_kind, CodingTaskKind::Debug);
        assert!(p.requires_task_truth);
        assert!(p.render_prompt_block().contains("Gather evidence"));
    }

    #[test]
    fn feature_request_requires_plan_and_verification() {
        let p = CodingSessionProfile::classify("实现 file search v2").unwrap();
        assert_eq!(p.task_kind, CodingTaskKind::Feature);
        assert!(p.requires_plan);
        assert!(p.render_prompt_block().contains("targeted verification"));
    }

    #[test]
    fn workflow_request_uses_script_profile() {
        let p = CodingSessionProfile::classify("设计 workflow.js 的 loop 模式").unwrap();
        assert_eq!(p.task_kind, CodingTaskKind::WorkflowScript);
        assert!(p.requires_script);
        assert!(p.render_prompt_block().contains("runtime-derived"));
    }
}
