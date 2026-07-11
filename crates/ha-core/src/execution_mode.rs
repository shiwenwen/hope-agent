use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    #[default]
    Off,
    Guarded,
    Deep,
    Autonomous,
}

impl ExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Guarded => "guarded",
            Self::Deep => "deep",
            Self::Autonomous => "autonomous",
        }
    }

    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "guarded" => Some(Self::Guarded),
            "deep" => Some(Self::Deep),
            "autonomous" => Some(Self::Autonomous),
            _ => None,
        }
    }

    pub fn parse_or_default(value: &str) -> Self {
        Self::from_str(value).unwrap_or_default()
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Guarded => "Guarded",
            Self::Deep => "Deep",
            Self::Autonomous => "Autonomous",
        }
    }

    pub fn system_prompt_section(self) -> Option<&'static str> {
        match self {
            Self::Off => None,
            Self::Guarded => Some(GUARDED_PROMPT),
            Self::Deep => Some(DEEP_PROMPT),
            Self::Autonomous => Some(AUTONOMOUS_PROMPT),
        }
    }
}

const GUARDED_PROMPT: &str = concat!(
    "# Execution Mode: Guarded\n\n",
    "This session has persistent execution mode `guarded` enabled.\n",
    "- Treat non-trivial coding work as observe -> plan -> edit -> targeted validate -> report.\n",
    "- Use task progress as the visible source of truth when the work spans multiple steps.\n",
    "- Prefer workflow scripts or subagents only when they make the task more stable or observable.\n",
    "- After edits, run the smallest project-appropriate validation that proves the touched behavior.\n",
    "- If validation fails, attempt at most one focused repair, then validate once more.\n",
    "- Stop and ask the user when the same validation failure repeats, two consecutive repair attempts produce no effective diff, required approval is unavailable, or the next step would exceed the agreed safety/performance envelope."
);

const DEEP_PROMPT: &str = concat!(
    "# Execution Mode: Deep\n\n",
    "This session has persistent execution mode `deep` enabled.\n",
    "- For substantial coding work, spend extra effort on repository reconnaissance, invariants, and regression risk before editing.\n",
    "- Prefer script-first workflows for durable multi-step work, and use read-only subagents for independent review or analysis when useful.\n",
    "- Keep user-facing progress explicit through tasks, workflow trace, or concise status updates.\n",
    "- Validate with targeted checks first; broaden only when the change crosses shared contracts or multiple modules.\n",
    "- Allow up to two focused repair attempts after validation feedback.\n",
    "- Stop and ask the user when validation failures repeat with the same fingerprint, two consecutive attempts have no effective diff, or the remaining path is ambiguous enough that continuing would risk churn."
);

const AUTONOMOUS_PROMPT: &str = concat!(
    "# Execution Mode: Autonomous\n\n",
    "This session has persistent execution mode `autonomous` enabled.\n",
    "- Continue safe, bounded coding progress without waiting for user confirmation between ordinary observe/edit/validate steps.\n",
    "- Keep all normal permission, hook, sandbox, project, and approval policies intact; this mode never bypasses approval gates.\n",
    "- Use durable workflow scripts for long-running or multi-agent work so progress can be inspected, paused, resumed, and recovered.\n",
    "- Prefer targeted validation after each meaningful edit batch and summarize residual risk before moving on.\n",
    "- Stop and ask the user when a strict approval is needed, validation failures repeat with the same fingerprint, two consecutive attempts have no effective diff, or the work would require destructive or broad changes outside the requested scope."
);
