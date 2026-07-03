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
    "- You may call `workflow_run` when deterministic multi-step orchestration, fan-out, independent review, research sweeps, migration planning, or long-running verification would make the work more complete, observable, or recoverable.\n",
    "- Workflow is not coding-only. Use it for any domain where structured background orchestration is useful; coding-specific validation is only one template.\n",
    "- Prefer a hybrid pattern: scout inline first to learn the work-list, then author a compact JavaScript workflow script for the deterministic fan-out or verification phase.\n",
    "- Script shape: `export default async function main(workflow) { const task = await workflow.task.create({ title: \"...\" }); ... await workflow.task.update({ task, status: \"completed\" }); await workflow.finish({ summary, verification, residualRisk }); }`.\n",
    "- Core host APIs use options objects: `workflow.fileSearch({ query, limit?, label? })`, `workflow.read({ path, label? })`, `workflow.grep({ pattern, path?, label? })`, `workflow.tool({ name, args, label? })`, `workflow.spawnAgent({ task, label?, agent_id?, timeout_secs? })`, `workflow.waitAll(handles, { timeout?, label? })`, `workflow.validate({ commands, reason?, label? })`, `workflow.review({ profiles?, focusPaths?, label? })`, `workflow.verify({ scope?, focusPaths?, maxCommands?, label? })`, `workflow.diff({ label? })`, `workflow.askUser({ question, context?, label? })` or `workflow.askUser({ questions, context? })`, `workflow.trace({ label?, payload })`, `workflow.block({ reason?, label?, payload? })`, `workflow.repairLoop({ label?, maxAttempts?, validationCommands?, focusPaths?, reviewProfiles? }, fn)`, `workflow.now()`, `workflow.random(seed)`, `workflow.finish({ summary, verification?, residualRisk? })`.\n",
    "- `label` is display-only. Reuse handles returned by APIs (for example task handles and spawn handles) instead of inventing stable ids.\n",
    "- Keep workflows well scoped. For large tasks, run several smaller workflows across phases and inspect each result before deciding the next phase.\n",
    "- Do not use Workflow for trivial conversational turns or simple one-shot edits.\n",
    "- Workflow Mode never bypasses permission, approval, sandbox, hook, incognito, or project access policy. If a workflow blocks on approval or safety, report that clearly and use the Workflow Control Center trace."
);

const WORKFLOW_MODE_ULTRACODE_PROMPT: &str = concat!(
    "# Workflow Mode: Ultracode\n\n",
    "Ultracode is on for this session: optimize for exhaustive correctness, stability, and coverage. Token cost is not the primary constraint.\n",
    "- Use `workflow_run` by default for every substantive task where parallel readers, independent designers, adversarial verifiers, broad search, or staged migration can improve quality.\n",
    "- Solo inline work is appropriate only for conversational, tiny, or already-verified mechanical turns.\n",
    "- Prefer multi-phase orchestration: understand -> design -> implement/check -> adversarial review -> synthesize. Keep each workflow phase observable and bounded.\n",
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
        assert!(prompt.contains("workflow_run"));
        assert!(prompt.contains("workflow.verify({ scope?, focusPaths?, maxCommands?, label? })"));
        assert!(prompt.contains("workflow.waitAll(handles, { timeout?, label? })"));
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
