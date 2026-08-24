#![cfg_attr(test, allow(clippy::needless_return))]

//! Kernel-owned Workflow preview types and feature execution port.

#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plan::{check_workflow_script_draft, GateReport, ScriptGateOptions};
use crate::session::SessionDB;

use super::runtime::{workflow_session_context, WorkflowSessionContext};
use super::types::WorkflowRun;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPermissionPreview {
    pub summary: WorkflowPermissionPreviewSummary,
    pub calls: Vec<WorkflowPermissionPreviewCall>,
    pub truncated: bool,
}

impl WorkflowPermissionPreview {
    pub fn requires_user_approval(&self) -> bool {
        self.summary.ask > 0 || self.summary.dynamic > 0
    }

    pub fn has_denials(&self) -> bool {
        self.summary.deny > 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPermissionPreviewSummary {
    pub total: usize,
    pub allow: usize,
    pub ask: usize,
    pub deny: usize,
    pub dynamic: usize,
    pub strict: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowPermissionPreviewCall {
    pub api: String,
    pub line: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    pub decision: String,
    pub strict: bool,
    pub dynamic: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowScriptPreview {
    pub gate: GateReport,
    pub gate_passed: bool,
    pub gate_feedback: String,
    pub permission: WorkflowPermissionPreview,
    pub can_create: bool,
    pub can_run_immediately: bool,
    pub requires_approval: bool,
    pub has_denials: bool,
}

#[derive(Clone, Copy)]
pub struct WorkflowPreviewRuntime {
    pub preview_script: fn(&str, &str, &WorkflowSessionContext) -> WorkflowPermissionPreview,
}

static WORKFLOW_PREVIEW_RUNTIME: OnceLock<WorkflowPreviewRuntime> = OnceLock::new();
#[cfg(not(test))]
static WARNED_WORKFLOW_PREVIEW_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

pub fn register_workflow_preview_runtime(
    runtime: WorkflowPreviewRuntime,
) -> std::result::Result<(), crate::AlreadyRegistered> {
    WORKFLOW_PREVIEW_RUNTIME
        .set(runtime)
        .map_err(|_| crate::AlreadyRegistered("workflow preview runtime"))
}

pub fn preview_workflow_run(db: &SessionDB, run: &WorkflowRun) -> WorkflowPermissionPreview {
    let session_context = workflow_session_context(db, &run.session_id);
    preview_workflow_script(&run.script_source, &run.session_id, &session_context)
}

pub fn preview_workflow_script_for_session(
    db: &SessionDB,
    session_id: &str,
    script: &str,
    execution_mode: Option<&str>,
) -> WorkflowScriptPreview {
    let gate = check_workflow_script_draft(
        script,
        script_gate_options_for_execution_mode(execution_mode.unwrap_or("guarded")),
    );
    let gate_passed = gate.passed();
    let gate_feedback = gate.render_feedback("Workflow Script Gate");
    let session_context = workflow_session_context(db, session_id);
    let permission = preview_workflow_script(script, session_id, &session_context);
    let requires_approval = permission.requires_user_approval();
    let has_denials = permission.has_denials();
    let can_create = gate_passed && !has_denials;

    WorkflowScriptPreview {
        gate,
        gate_passed,
        gate_feedback,
        permission,
        can_create,
        can_run_immediately: can_create,
        requires_approval,
        has_denials,
    }
}

pub fn ensure_workflow_script_can_create(
    db: &SessionDB,
    session_id: &str,
    script: &str,
    execution_mode: Option<&str>,
) -> Result<WorkflowScriptPreview> {
    let preview = preview_workflow_script_for_session(db, session_id, script, execution_mode);
    if !preview.gate_passed {
        return Err(anyhow!(preview.gate_feedback.clone()));
    }
    if preview.has_denials {
        return Err(anyhow!(
            "Workflow permission preview denied; inspect the permission checklist before creating this run"
        ));
    }
    Ok(preview)
}

pub(crate) fn script_gate_options_for_execution_mode(execution_mode: &str) -> ScriptGateOptions {
    ScriptGateOptions {
        autonomous: execution_mode == "autonomous",
    }
}

#[cfg(test)]
#[path = "../../../ha-workflow/src/preview.rs"]
mod test_preview;

pub(crate) fn preview_workflow_script(
    script: &str,
    session_id: &str,
    session_context: &WorkflowSessionContext,
) -> WorkflowPermissionPreview {
    if let Some(runtime) = WORKFLOW_PREVIEW_RUNTIME.get() {
        return (runtime.preview_script)(script, session_id, session_context);
    }

    #[cfg(test)]
    {
        return test_preview::preview_workflow_script(script, session_id, session_context);
    }

    #[cfg(not(test))]
    {
        if !WARNED_WORKFLOW_PREVIEW_UNAVAILABLE.swap(true, Ordering::Relaxed) {
            app_warn!(
                "workflow",
                "preview_runtime_unavailable",
                "Workflow preview runtime is not wired; script execution is denied"
            );
        }
        WorkflowPermissionPreview {
            summary: WorkflowPermissionPreviewSummary {
                total: 1,
                deny: 1,
                strict: 1,
                ..Default::default()
            },
            calls: vec![WorkflowPermissionPreviewCall {
                api: "workflow.runtime".to_string(),
                line: 0,
                tool_name: None,
                decision: "deny".to_string(),
                strict: true,
                dynamic: false,
                reason: Some("workflow_preview_runtime_unavailable".to_string()),
                label: Some("Workflow preview runtime unavailable".to_string()),
                args: None,
            }],
            truncated: false,
        }
    }
}
