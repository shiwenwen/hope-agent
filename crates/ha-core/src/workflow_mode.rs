use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowMode {
    Off,
    On,
    Ultracode,
}

impl Default for WorkflowMode {
    fn default() -> Self {
        Self::Off
    }
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
    "- When Workflow Mode is active and a task needs multiple sub-agents, prefer workflow-owned children created with `workflow.spawnAgent`. Do not create an empty registration workflow and then run the real fan-out through unrelated `subagent.batch_spawn` calls outside it.\n",
    "- Choose child-result timing deliberately: `injectPolicy: \"checkpoint\"` for stage-by-stage parent awareness, `none` for explicit `agentStatus`/`agentResult` queries, and `final` when only the final synthesis should surface. Use `waitAny` to react to early results and `waitAll` for a deliberate final join. `waitAll(..., { resultMode: \"status\" })` observes progress without consuming child results.\n",
    "- You may steer a running child with `workflow.agentSteer`, cancel no-longer-useful children with `workflow.cancelAgent`, and spawn additional children after inspecting partial results. A partial join does not detach remaining children: cancel or consume them before finishing.\n",
    "- Before `workflow.finish`, consume every required terminal child result and ensure no workflow-owned child is still running. As a safety fallback, finish waits for owned children and attaches unconsumed results; it blocks on timeout instead of showing a misleading completed state.\n",
    "- Script shape: `export default async function main(workflow) { const task = await workflow.task.create({ title: \"...\" }); ... await workflow.task.update({ task, status: \"completed\" }); await workflow.finish({ summary, verification, residualRisk }); }`.\n",
    "- Core host APIs use options objects: `workflow.phase({ name, label?, expected?, criteriaIds?, injectPolicy? }, async (phase) => { ... })`, `workflow.progress({ phase?, phaseKey?, message, percent?, counters?, payload?, importance? })`, `workflow.checkpoint({ title, summary, phase?, importance?, inject?, findings?, evidence?, decisions?, next?, payload? })`, `workflow.report({ title?, summary, nextAction?, needsUser?, inject?, payload? })`, `workflow.fileSearch({ query, limit?, label? })`, `workflow.read({ path, label? })`, `workflow.grep({ pattern, path?, label? })`, `workflow.tool({ name, args, label? })`, `workflow.spawnAgent({ task, label?, agent_id?, timeout_secs?, files?, injectPolicy?, resultMode? })`, `workflow.agentStatus(handles, { label? })`, `workflow.agentResult(handle, { mode?, label? })`, `workflow.waitAny(handles, { min?, timeout?, label? })`, `workflow.waitAll(handles, { timeout?, partial?, resultMode?, label? })`, `workflow.agentSteer(handle, { message, label? })`, `workflow.cancelAgent(handles, { reason?, label? })`, `workflow.validate({ commands, reason?, label? })`, `workflow.review({ profiles?, focusPaths?, label? })`, `workflow.verify({ scope?, focusPaths?, maxCommands?, label? })`, `workflow.diff({ label? })`, `workflow.askUser({ question, context?, label? })` or `workflow.askUser({ questions, context? })`, `workflow.trace({ label?, payload })`, `workflow.block({ reason?, label?, payload? })`, `workflow.repairLoop({ label?, maxAttempts?, validationCommands?, focusPaths?, reviewProfiles? }, fn)`, `workflow.now()`, `workflow.random(seed)`, `workflow.finish({ summary, verification?, residualRisk? })`.\n",
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
        assert!(prompt.contains("workflow.verify({ scope?, focusPaths?, maxCommands?, label? })"));
        assert!(prompt.contains("workflow.phase({ name, label?, expected?"));
        assert!(prompt.contains("workflow.checkpoint({ title, summary"));
        assert!(prompt.contains("workflow.report({ title?, summary"));
        assert!(prompt.contains("workflow.agentStatus(handles, { label? })"));
        assert!(prompt.contains("workflow.agentResult(handle, { mode?, label? })"));
        assert!(prompt.contains("workflow.waitAny(handles, { min?, timeout?, label? })"));
        assert!(prompt.contains("workflow.waitAll(handles, { timeout?, partial?, resultMode?"));
        assert!(prompt.contains("workflow.agentSteer(handle, { message, label? })"));
        assert!(prompt.contains("workflow.cancelAgent(handles, { reason?, label? })"));
        assert!(prompt.contains("Do not create an empty registration workflow"));
        assert!(prompt.contains("workflow.askUser({ question, context?, label? })"));
        assert!(prompt.contains("workflow.askUser({ questions, context? })"));
        assert!(prompt.contains("workflow.block({ reason?, label?, payload? })"));
        assert!(prompt.contains("workflow.repairLoop("));
        assert!(prompt.contains("workflow.now()"));
        assert!(prompt.contains("workflow.random(seed)"));
        assert!(!prompt.contains("workflow.verify({ claims?"));
        assert!(!prompt.contains("workflow.block({ reason, next?"));
        assert!(!prompt.contains("workflow.askUser({ questions })"));
        assert_eq!(prompt, WORKFLOW_MODE_ON_PROMPT);
    }
}
