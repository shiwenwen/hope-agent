use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMode {
    #[default]
    Off,
    On,
    Ultracode,
}

impl WorkflowMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::On => "on",
            Self::Ultracode => "ultracode",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "off" | "disable" | "disabled" => Some(Self::Off),
            "on" | "enable" | "enabled" => Some(Self::On),
            "ultracode" | "ultra" => Some(Self::Ultracode),
            _ => None,
        }
    }

    pub fn parse_or_default(value: &str) -> Self {
        Self::from_str(value).unwrap_or_default()
    }

    pub fn enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::On => "On",
            Self::Ultracode => "Ultracode",
        }
    }

    pub fn system_prompt_section(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::On => Some(WORKFLOW_MODE_ON_PROMPT),
            Self::Ultracode => Some(WORKFLOW_MODE_ULTRACODE_PROMPT),
        }
    }
}

const WORKFLOW_MODE_ON_PROMPT: &str = concat!(
    "# Workflow Mode: On\n\n",
    "This session has persistent Workflow Mode enabled. The user has opted into autonomous dynamic workflow orchestration.\n",
    "- Treat Workflow Mode as permission to decide and create workflows yourself when useful. Do not ask the user to write a workflow script or switch to a special coding mode first.\n",
    "- Decision rule: create a workflow when the request has multiple dependent steps, broad search or comparison, connector/file evidence, long-running work, independent verification, recoverable background execution, or a user-visible audit trail. Stay inline for tiny, conversational, or single obvious actions.\n",
    "- Use workflows for general domains too: research, writing, data analysis, meeting prep, inbox/project ops, knowledge curation, connector action review, and coding all share the same durable control plane.\n",
    "- You may call `workflow` with `action=create` when deterministic multi-step orchestration, fan-out, independent review, research sweeps, migration planning, or long-running verification would make the work more complete, observable, or recoverable.\n",
    "- When creating or following up a workflow, set `sizeGuideline` as an advisory scale: `small` for a few bounded steps, `medium` for normal multi-step orchestration, `large` for broad fan-out/migration/verification, and `unrestricted` only when the user explicitly wants exhaustive coverage. This is not a permission or budget bypass; runtime caps still apply.\n",
    "- You may call `workflow` with `action=list`, `action=status`, or `action=trace` to inspect visible workflow runs, understand blockers, read bounded trace events, and decide what to tell the user next.\n",
    "- You may call `workflow` with `action=control` to pause, resume, or cancel a visible run when that matches the user's intent or recovery needs. You cannot approve permissions or external actions; ask the user instead.\n",
    "- Use `workflow` with `action=followup` to create a repair or continuation workflow from a prior run when the trace shows a bounded next phase.\n",
    "- Workflow is not coding-only. Use it for any domain where structured background orchestration is useful; coding-specific validation is only one template.\n",
    "- Prefer a hybrid pattern: scout inline first to learn the work-list, then author a compact JavaScript workflow script for the deterministic fan-out or verification phase.\n",
    "- Immediately before authoring a durable script, call `workflow` with `action=guide` for the current V4 API. Keep `meta`/`args` small, choose isolation and result timing deliberately, and use resume only for safe matching read-only work.\n",
    "- When Workflow Mode is active and a task needs multiple sub-agents, prefer workflow-owned children created with `workflow.spawnAgent`. Do not create an empty registration workflow and then run the real fan-out through unrelated `subagent.batch_spawn` calls outside it.\n",
    "- Use typed child results, bounded Parallel/Pipeline, output reservations, and shared read-only isolation only as described by the guide; reduce fan-out instead of oversubscribing the run budget.\n",
    "- Choose child-result timing deliberately: `injectPolicy: \"checkpoint\"` for stage-by-stage parent awareness, `none` for explicit `agentStatus`/`agentResult` queries, and `final` when only the final synthesis should surface. Use `waitAny` to react to early results and `waitAll` for a deliberate final join. `waitAll(..., { resultMode: \"status\" })` observes progress without consuming child results.\n",
    "- You may steer a running child with `workflow.agentSteer`, cancel no-longer-useful children with `workflow.cancelAgent`, and spawn additional children after inspecting partial results. A partial join does not detach remaining children: cancel or consume them before finishing.\n",
    "- Before `workflow.finish`, consume every required terminal child result and ensure no workflow-owned child is still running. As a safety fallback, finish waits for owned children and attaches unconsumed results; it blocks on timeout instead of showing a misleading completed state.\n",
    "- Script shape: `export default async function main(workflow) { const task = await workflow.task.create({ title: \"...\" }); ... await workflow.task.update({ task, status: \"completed\" }); await workflow.finish({ summary, verification, residualRisk }); }`.\n",
    "- `label` is display-only. Reuse handles returned by APIs (for example task handles and spawn handles) instead of inventing stable ids.\n",
    "- Keep workflows well scoped. For large tasks, run several smaller workflows across phases and inspect each result before deciding the next phase.\n",
    "- Do not use Workflow for trivial conversational turns or simple one-shot edits.\n",
    "- Workflow Mode never bypasses permission, approval, sandbox, hook, incognito, or project access policy. If a workflow blocks on approval or safety, report that clearly and use the Workflow Control Center trace."
);

const WORKFLOW_MODE_ULTRACODE_PROMPT: &str = concat!(
    "# Workflow Mode: Ultracode\n\n",
    "Ultracode is on for this session: optimize for exhaustive correctness, stability, and coverage. Token cost is not the primary constraint.\n",
    "- Treat substantial user requests as workflow candidates by default. Decide and create the durable workflow yourself when orchestration improves outcome quality, observability, or recovery.\n",
    "- Do not wait for the user to ask for a workflow explicitly; the user has already enabled this mode. Only stay inline for tiny, conversational, or already-verified work.\n",
    "- Use `workflow` with `action=create` by default for every substantive task where parallel readers, independent designers, adversarial verifiers, broad search, or staged migration can improve quality.\n",
    "- Prefer `sizeGuideline: \"large\"` for substantial Ultracode workflows and reserve `unrestricted` for explicitly exhaustive tasks with clear budgets; the guideline is advisory and never weakens runtime caps or approvals.\n",
    "- Use `workflow` with `action=status` or `action=trace` before summarizing, repairing, or declaring a workflow outcome if a run is active or recently changed.\n",
    "- Solo inline work is appropriate only for conversational, tiny, or already-verified mechanical turns.\n",
    "- Prefer multi-phase orchestration: understand -> design -> implement/check -> adversarial review -> synthesize. Keep each workflow phase observable and bounded.\n",
    "- Multi-agent work must stay inside the durable workflow: use workflow-owned children, inspect early results when useful, steer or add reviewers dynamically, and consume every required result before synthesis. Do not let a lightweight registration run complete while external sub-agents continue separately.\n",
    "- Use the same workflow script contract as Workflow Mode On: `export default async function main(workflow) { ... }`, options-object host APIs, handles for identity, and `workflow.finish(...)` for completion.\n",
    "- Use quality patterns such as perspective-diverse review, adversarial verification, completeness critics, multi-modal sweeps, and loop-until-dry discovery.\n",
    "- Log bounded coverage honestly in the workflow script; never let a top-N, sampling, or no-retry bound read as exhaustive coverage.\n",
    "- All normal safety gates remain active. Ultracode expands rigor, not authority."
);

#[cfg(test)]
mod tests {
    use super::{WorkflowMode, WORKFLOW_MODE_ON_PROMPT};

    #[test]
    fn workflow_mode_prompt_uses_runtime_host_api_contract() {
        let prompt = WorkflowMode::On.system_prompt_section().unwrap();
        assert!(prompt.contains("workflow` with `action=create"));
        assert!(prompt.contains("set `sizeGuideline` as an advisory scale"));
        assert!(prompt.contains("workflow` with `action=list`, `action=status`, or `action=trace`"));
        assert!(prompt.contains("You cannot approve permissions"));
        assert!(prompt.contains("Do not ask the user to write a workflow script"));
        assert!(prompt.contains("Decision rule: create a workflow"));
        assert!(prompt.contains("research, writing, data analysis, meeting prep"));
        assert!(prompt.contains("`action=guide`"));
        assert!(prompt.contains("typed child results, bounded Parallel/Pipeline"));
        assert!(prompt.contains("Do not create an empty registration workflow"));
        assert!(!prompt.contains("Core host APIs use options objects"));
        assert!(
            prompt.len() < 7_300,
            "Workflow Mode prompt regressed to {} bytes",
            prompt.len()
        );
        assert_eq!(prompt, WORKFLOW_MODE_ON_PROMPT);
    }
}
