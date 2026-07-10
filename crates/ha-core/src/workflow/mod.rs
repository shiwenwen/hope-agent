//! Durable workflow run store for script-first dynamic workflows.
//!
//! Runtime execution uses these APIs instead of inventing a parallel
//! run/op/event store.

pub(crate) mod db;
pub(crate) mod events;
pub mod preview;
pub mod runtime;
pub mod types;

pub(crate) use db::ensure_tables;
pub use preview::{
    ensure_workflow_script_can_create, preview_workflow_run, preview_workflow_script_for_session,
    WorkflowPermissionPreview, WorkflowPermissionPreviewCall, WorkflowPermissionPreviewSummary,
    WorkflowScriptPreview,
};
pub(crate) use runtime::on_workflow_child_status_changed;
pub use runtime::{
    cancel_workflow_run_with_children, ensure_workflow_launcher_primary,
    recover_pending_workflow_runs, run_workflow_script, run_workflow_script_async,
    spawn_startup_recovery_if_primary, spawn_workflow_run_if_primary, WorkflowRecoveryReport,
    WorkflowRuntimeResult,
};
pub use types::{
    CreateWorkflowRunFromTemplateInput, CreateWorkflowRunInput, ListSavedWorkflowTemplatesInput,
    SaveWorkflowTemplateInput, SavedWorkflowTemplate, SavedWorkflowTemplateScope,
    StartedOpRecoveryAction, UpsertWorkflowOpInput, WorkflowEffectClass, WorkflowEvent, WorkflowOp,
    WorkflowOpState, WorkflowRun, WorkflowRunSnapshot, WorkflowRunState, WorkflowWatchdogFinding,
};

#[cfg(test)]
mod tests;
