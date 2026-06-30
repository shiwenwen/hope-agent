//! Durable workflow run store for Phase 2 script-first coding workflows.
//!
//! Runtime execution uses these APIs instead of inventing a parallel
//! run/op/event store.

pub(crate) mod db;
pub(crate) mod events;
pub mod runtime;
pub mod types;

pub(crate) use db::ensure_tables;
pub use runtime::{run_workflow_script, run_workflow_script_async, WorkflowRuntimeResult};
pub use types::{
    CreateWorkflowRunInput, StartedOpRecoveryAction, UpsertWorkflowOpInput, WorkflowEffectClass,
    WorkflowEvent, WorkflowOp, WorkflowOpState, WorkflowRun, WorkflowRunSnapshot, WorkflowRunState,
};

#[cfg(test)]
mod tests;
