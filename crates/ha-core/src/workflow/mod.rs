//! Durable workflow run store for Phase 2 script-first coding workflows.
//!
//! This module intentionally stops at durable state, events, and recovery
//! decisions. The embedded JavaScript runtime lands later and must use these
//! APIs instead of inventing a parallel run/op/event store.

pub(crate) mod db;
pub(crate) mod events;
pub mod types;

pub(crate) use db::ensure_tables;
pub use types::{
    CreateWorkflowRunInput, StartedOpRecoveryAction, UpsertWorkflowOpInput, WorkflowEffectClass,
    WorkflowEvent, WorkflowOp, WorkflowOpState, WorkflowRun, WorkflowRunSnapshot, WorkflowRunState,
};

#[cfg(test)]
mod tests;
