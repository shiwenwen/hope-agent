use serde_json::json;

use super::super::{
    TOOL_GOAL_BLOCK_REQUEST, TOOL_GOAL_CHECKPOINT, TOOL_GOAL_EVALUATE, TOOL_GOAL_FINISH_REQUEST,
    TOOL_GOAL_RECORD_EVIDENCE, TOOL_GOAL_STATUS,
};
use super::types::{CoreSubclass, ToolDefinition, ToolTier};

fn goal_core_tool(name: &str, description: &str, parameters: serde_json::Value) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: description.into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::Interaction,
        },
        internal: true,
        concurrent_safe: false,
        async_capable: false,
        parameters,
    }
}

pub fn get_goal_status_tool() -> ToolDefinition {
    ToolDefinition {
        name: TOOL_GOAL_STATUS.into(),
        description: "Read the active durable Goal for this session. Use this when you need to \
check the objective, revision, criteria, evidence, budget, completion audit, or whether the \
goal changed while a long task was running. This is read-only and safe to call before deciding \
whether to continue, evaluate, finish, or ask the user."
            .into(),
        tier: ToolTier::Core {
            subclass: CoreSubclass::Interaction,
        },
        internal: true,
        concurrent_safe: true,
        async_capable: false,
        parameters: json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
    }
}

pub fn get_goal_checkpoint_tool() -> ToolDefinition {
    goal_core_tool(
        TOOL_GOAL_CHECKPOINT,
        "Record a lightweight progress checkpoint for the active durable Goal. Use this during \
long tasks after meaningful milestones, failed attempts, scope discoveries, or before handing \
off between turns. A checkpoint is not completion evidence by itself; it helps future turns \
resume with less drift.",
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "Concise progress summary. Include what changed, what was learned, or what was attempted."
                },
                "status": {
                    "type": "string",
                    "enum": ["progress", "milestone", "handoff", "risk", "blocked_attempt"],
                    "description": "Checkpoint kind. Use blocked_attempt for a failed attempt that is not enough to stop the goal yet."
                },
                "next": {
                    "type": "string",
                    "description": "Optional next action or resumption hint."
                },
                "evidence": {
                    "type": "array",
                    "description": "Optional short evidence labels or ids already produced elsewhere.",
                    "items": { "type": "string" }
                },
                "confidence": {
                    "type": "string",
                    "enum": ["low", "medium", "high"],
                    "description": "How confident you are that this checkpoint moves the goal forward."
                }
            },
            "required": ["summary"],
            "additionalProperties": false
        }),
    )
}

pub fn get_goal_record_evidence_tool() -> ToolDefinition {
    goal_core_tool(
        TOOL_GOAL_RECORD_EVIDENCE,
        "Attach a general-domain evidence item to the active durable Goal. Use this for \
non-coding proof such as cited sources, checked claims, user decisions, reviewed artifacts, \
data quality checks, approved drafts, or meeting context. Do not fabricate evidence: only \
record facts you actually observed, produced, or received in this conversation/tool run. \
For coding evidence, prefer workflows/tasks/validation tools that attach stronger evidence \
automatically.",
        json!({
            "type": "object",
            "properties": {
                "relation": {
                    "type": "string",
                    "enum": [
                        "source_cited",
                        "claim_checked",
                        "user_decision",
                        "artifact_reviewed",
                        "data_quality_checked",
                        "citation_audited",
                        "message_draft_approved",
                        "meeting_context_collected",
                        "review_completed",
                        "review_passed",
                        "review_finding"
                    ],
                    "description": "Evidence relation. Choose the most specific truthful relation."
                },
                "title": {
                    "type": "string",
                    "description": "Short evidence title shown in audits."
                },
                "summary": {
                    "type": "string",
                    "description": "What the evidence proves or why it matters."
                },
                "sourceId": {
                    "type": "string",
                    "description": "Stable source id if one exists, e.g. URL, document id, artifact path, or decision id. If omitted, a generated id is used."
                },
                "goalCriterionId": {
                    "type": "string",
                    "description": "Optional criterion id from goal_status/Active Goal when this evidence supports a specific criterion."
                },
                "metadata": {
                    "type": "object",
                    "description": "Small structured details. Do not include secrets or large raw documents.",
                    "additionalProperties": true
                }
            },
            "required": ["relation", "title", "summary"],
            "additionalProperties": false
        }),
    )
}

pub fn get_goal_evaluate_tool() -> ToolDefinition {
    goal_core_tool(
        TOOL_GOAL_EVALUATE,
        "Run the deterministic completion audit for the active durable Goal. Use this before \
claiming completion, when evidence changed, or when you need a precise list of missing items. \
If required evidence is missing, the goal remains open but may be marked blocked until more \
evidence is produced.",
        json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Why you are running the audit now."
                }
            },
            "additionalProperties": false
        }),
    )
}

pub fn get_goal_finish_request_tool() -> ToolDefinition {
    goal_core_tool(
        TOOL_GOAL_FINISH_REQUEST,
        "Request final completion of the active durable Goal. This tool will re-run/validate \
the current audit and only closes the goal when the deterministic evidence gate says completed \
for the current revision. Call this after you believe the objective and required completion \
criteria are satisfied, immediately before your final concise summary to the user.",
        json!({
            "type": "object",
            "properties": {
                "summary": {
                    "type": "string",
                    "description": "User-facing final summary of what was achieved."
                },
                "followUpItems": {
                    "type": "array",
                    "description": "Optional non-blocking follow-ups discovered during the goal.",
                    "items": { "type": "string" }
                },
                "remainingRisk": {
                    "type": "string",
                    "description": "Optional honest residual risk or evidence limitation."
                }
            },
            "additionalProperties": false
        }),
    )
}

pub fn get_goal_block_request_tool() -> ToolDefinition {
    goal_core_tool(
        TOOL_GOAL_BLOCK_REQUEST,
        "Request that the active durable Goal be marked blocked. Do not use this just because \
work is hard. Use it only after repeated failed attempts, a required user decision, exhausted \
budget, or an external state change that you cannot safely perform. The runtime requires \
clear reason and attempted actions, and repeated same-fingerprint blockers are needed unless \
the block explicitly requires user input or external state.",
        json!({
            "type": "object",
            "properties": {
                "reason": {
                    "type": "string",
                    "description": "Concrete blocking condition."
                },
                "attempted": {
                    "type": "array",
                    "description": "Actions already attempted before stopping.",
                    "items": { "type": "string" }
                },
                "needed": {
                    "type": "string",
                    "description": "What user input or external state would unblock progress."
                },
                "fingerprint": {
                    "type": "string",
                    "description": "Stable id for the repeated blocker. Omit to derive from reason."
                },
                "needsUserInput": {
                    "type": "boolean",
                    "description": "True when progress cannot continue without a user decision or missing information."
                },
                "externalStateRequired": {
                    "type": "boolean",
                    "description": "True when a required external state change or access grant is outside the model/runtime's control."
                }
            },
            "required": ["reason", "attempted"],
            "additionalProperties": false
        }),
    )
}
