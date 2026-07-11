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
pub(crate) enum TaskFlow {
    LightCoding,
    PlanImplement,
    EvidenceDebug,
    ReviewOnly,
    VerifyOnly,
    WorkflowScript,
}

impl TaskFlow {
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
    pub task_flow: TaskFlow,
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

        let has_coding_context = has_any(
            &text,
            &[
                "code",
                "coding",
                "source",
                "repo",
                "commit",
                "branch",
                "diff",
                "pull request",
                " pr ",
                "api",
                "frontend",
                "backend",
                "function",
                "class",
                "module",
                "crate",
                "runtime",
                "parser",
                "helper",
                "component",
                "button",
                "page",
                "service",
                "database",
                "file",
                "sql",
                "css",
                "react",
                "rust",
                "typescript",
                "python",
                "test",
                "bug",
                "代码",
                "编码",
                "源码",
                "仓库",
                "提交",
                "分支",
                "接口",
                "前端",
                "后端",
                "函数",
                "模块",
                "组件",
                "按钮",
                "页面",
                "界面",
                "服务",
                "数据库",
                "文件",
                "字段",
                "逻辑",
                "状态机",
                "应用",
                "测试",
                "改动",
                "改代码",
            ],
        );

        let has_explicit_workflow_script = has_any(
            &text,
            &[
                "workflow.js",
                "workflow script",
                "durable replay",
                "工作流脚本",
            ],
        );
        let has_generic_workflow = has_any(
            &text,
            &["workflow", "dynamic workflow", "工作流", "动态工作流"],
        );
        if has_explicit_workflow_script || (has_generic_workflow && has_coding_context) {
            return Some(Self::for_kind(CodingTaskKind::WorkflowScript));
        }

        let has_explicit_review = has_any(
            &text,
            &["code review", "检查未提交", "检查我未提交", "代码审查"],
        );
        let has_generic_review = has_any(
            &text,
            &["review", "复核", "审查", "检查当前改动", "检查更改"],
        );
        if has_explicit_review || (has_generic_review && has_coding_context) {
            return Some(Self::for_kind(CodingTaskKind::Review));
        }

        let has_explicit_debug = has_any(
            &text,
            &[
                "debug",
                "root cause",
                "bug",
                "crash",
                "stack trace",
                "regression",
                "flaky",
                "failing test",
                "回归",
            ],
        );
        let has_generic_debug = has_any(
            &text,
            &[
                "diagnose",
                "reproduce",
                "报错",
                "失败",
                "崩溃",
                "复现",
                "排查",
                "定位",
            ],
        );
        if has_explicit_debug || (has_generic_debug && has_coding_context) {
            return Some(Self::for_kind(CodingTaskKind::Debug));
        }

        let has_verify = has_any(
            &text,
            &[
                "verify",
                "verification",
                "test plan",
                "what should we run",
                "测试什么",
                "收尾检查",
                "是否完成",
                "还差什么",
                "验证",
            ],
        );
        if has_verify && has_coding_context {
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
        if has_feature && (has_coding_context || text.contains("改代码")) {
            return Some(Self::for_feature(feature_requires_plan(&text)));
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

    fn for_feature(requires_plan: bool) -> Self {
        let recommended_skills = if requires_plan {
            vec!["ha-coding-common", "ha-coding-plan", "ha-verify"]
        } else {
            vec!["ha-coding-common", "ha-test-strategy", "ha-verify"]
        };
        let task_flow = if requires_plan {
            TaskFlow::PlanImplement
        } else {
            TaskFlow::LightCoding
        };
        let first_discipline = if requires_plan {
            "Ground a concise implementation plan in the current code, then continue when execution is allowed."
        } else {
            "The change appears small and direct: inspect the owning path, then implement without plan ceremony."
        };

        Self {
            task_kind: CodingTaskKind::Feature,
            task_flow,
            requires_plan,
            requires_script: false,
            requires_task_truth: true,
            recommended_skills,
            verification_policy: "select test-first, regression-first, characterization, or direct verification according to risk; do not default to full suites",
            risk_level: if requires_plan { "medium" } else { "low" },
            discipline: vec![
                first_discipline,
                "Track progress truthfully for multi-step work and preserve unrelated user changes.",
                "Keep edits scoped and finish with direct verification evidence.",
            ],
        }
    }

    fn for_kind(task_kind: CodingTaskKind) -> Self {
        match task_kind {
            CodingTaskKind::Review => Self {
                task_kind,
                task_flow: TaskFlow::ReviewOnly,
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
                task_flow: TaskFlow::EvidenceDebug,
                requires_plan: false,
                requires_script: false,
                requires_task_truth: true,
                recommended_skills: vec!["ha-debug", "ha-test-strategy", "ha-verify"],
                verification_policy: "reproduce or characterize the failure first; verify with the narrowest regression check",
                risk_level: "medium",
                discipline: vec![
                    "Gather evidence before patching: failing output, logs, stack trace, or a minimal reproduction.",
                    "Patch the smallest credible root cause; avoid broad rewrites before proof.",
                    "State the targeted regression check and whether it ran.",
                ],
            },
            CodingTaskKind::Feature => Self::for_feature(true),
            CodingTaskKind::WorkflowScript => Self {
                task_kind,
                task_flow: TaskFlow::WorkflowScript,
                requires_plan: true,
                requires_script: true,
                requires_task_truth: true,
                recommended_skills: vec![
                    "ha-workflow-script",
                    "ha-multi-agent-coding",
                    "ha-verify",
                ],
                verification_policy: "review script gates, replay safety, stop conditions, and targeted validation commands",
                risk_level: "high",
                discipline: vec![
                    "Draft scripts as durable host-API orchestration, not raw fs/network/process code.",
                    "Use labels only for display; task updates must use handles and op identity must remain runtime-derived.",
                    "Keep repair cycles runtime-controlled with explicit stop conditions.",
                ],
            },
            CodingTaskKind::Verify => Self {
                task_kind,
                task_flow: TaskFlow::VerifyOnly,
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
                task_flow: TaskFlow::LightCoding,
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
        out.push_str(&format!("- task_flow: {}\n", self.task_flow.as_str()));
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

fn feature_requires_plan(text: &str) -> bool {
    text.chars().count() > 240
        || has_any(
            text,
            &[
                " v2",
                " v3",
                " v4",
                "phase",
                "multi-step",
                "cross-module",
                "cross-crate",
                "migration",
                "architecture",
                "end-to-end",
                "complete implementation",
                "复杂",
                "完整",
                "全面",
                "跨模块",
                "跨 crate",
                "迁移",
                "架构",
                "端到端",
                "阶段",
            ],
        )
}

#[cfg(test)]
mod tests {
    use super::{CodingSessionProfile, CodingTaskKind, TaskFlow};

    #[test]
    fn review_request_is_review_only() {
        let p = CodingSessionProfile::classify("请检查我未提交的更改").unwrap();
        assert_eq!(p.task_kind, CodingTaskKind::Review);
        assert_eq!(p.task_flow, TaskFlow::ReviewOnly);
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
        assert!(p.recommended_skills.contains(&"ha-coding-plan"));
        assert!(p
            .render_prompt_block()
            .contains("do not default to full suites"));
    }

    #[test]
    fn small_feature_skips_plan_ceremony() {
        let p = CodingSessionProfile::classify("修复按钮文案").unwrap();
        assert_eq!(p.task_kind, CodingTaskKind::Feature);
        assert_eq!(p.task_flow, TaskFlow::LightCoding);
        assert!(!p.requires_plan);
        assert!(p.recommended_skills.contains(&"ha-test-strategy"));
    }

    #[test]
    fn workflow_request_uses_script_profile() {
        let p = CodingSessionProfile::classify("设计 workflow.js 的执行模式").unwrap();
        assert_eq!(p.task_kind, CodingTaskKind::WorkflowScript);
        assert!(p.requires_script);
        assert!(p.recommended_skills.contains(&"ha-multi-agent-coding"));
        assert!(p.render_prompt_block().contains("runtime-derived"));
    }

    #[test]
    fn routing_fixture_covers_coding_and_non_coding_boundaries() {
        let cases = [
            ("review my uncommitted diff", Some(CodingTaskKind::Review)),
            ("请做代码审查", Some(CodingTaskKind::Review)),
            ("复核这个 Rust commit", Some(CodingTaskKind::Review)),
            ("检查当前改动", Some(CodingTaskKind::Review)),
            ("debug this crash", Some(CodingTaskKind::Debug)),
            ("the parser has a regression", Some(CodingTaskKind::Debug)),
            ("这个前端报错帮我排查", Some(CodingTaskKind::Debug)),
            ("复现这个失败测试", Some(CodingTaskKind::Debug)),
            ("实现一个 parser", Some(CodingTaskKind::Feature)),
            ("add a retry helper", Some(CodingTaskKind::Feature)),
            ("修复按钮文案", Some(CodingTaskKind::Feature)),
            ("refactor this module", Some(CodingTaskKind::Feature)),
            ("完成 Phase 2 的跨模块迁移", Some(CodingTaskKind::Feature)),
            (
                "build a complete frontend feature",
                Some(CodingTaskKind::Feature),
            ),
            ("verify this code change", Some(CodingTaskKind::Verify)),
            ("这个 crate 还差什么", Some(CodingTaskKind::Verify)),
            ("测试什么才能证明修复", Some(CodingTaskKind::Verify)),
            ("对当前代码做收尾检查", Some(CodingTaskKind::Verify)),
            ("draft a workflow.js", Some(CodingTaskKind::WorkflowScript)),
            ("设计动态工作流脚本", Some(CodingTaskKind::WorkflowScript)),
            (
                "review the workflow runtime",
                Some(CodingTaskKind::WorkflowScript),
            ),
            (
                "给代码工作流增加 typed result",
                Some(CodingTaskKind::WorkflowScript),
            ),
            ("show the current git branch", Some(CodingTaskKind::General)),
            ("解释这段 code", Some(CodingTaskKind::General)),
            ("提交当前仓库", Some(CodingTaskKind::General)),
            ("inspect the repo", Some(CodingTaskKind::General)),
            ("设计一个请假审批工作流", None),
            ("设计一个动态审批工作流", None),
            ("复核这份合同", None),
            ("这个打印机报错，帮我排查", None),
            ("修复打印机故障", None),
            ("优化旅行路线", None),
            ("实现新的审批制度", None),
            ("验证这份旅行计划是否完整", None),
            ("每周循环提醒我喝水", None),
            ("写一份项目计划", None),
        ];

        for (input, expected) in cases {
            let actual = CodingSessionProfile::classify(input).map(|p| p.task_kind);
            assert_eq!(actual, expected, "unexpected routing for {input:?}");
        }
    }

    #[test]
    fn every_profile_recommends_at_most_three_distinct_skills() {
        for kind in [
            CodingTaskKind::General,
            CodingTaskKind::Feature,
            CodingTaskKind::Debug,
            CodingTaskKind::Review,
            CodingTaskKind::Verify,
            CodingTaskKind::WorkflowScript,
        ] {
            let profile = CodingSessionProfile::for_kind(kind);
            let distinct: std::collections::HashSet<_> =
                profile.recommended_skills.iter().copied().collect();
            assert_eq!(distinct.len(), profile.recommended_skills.len());
            assert!(profile.recommended_skills.len() <= 3);
        }
    }
}
