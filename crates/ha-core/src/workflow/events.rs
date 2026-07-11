use serde::Serialize;
use serde_json::json;

use super::types::{WorkflowEvent, WorkflowOp, WorkflowRun};

fn emit<T: Serialize>(name: &str, payload: &T) {
    if let Some(bus) = crate::get_event_bus() {
        bus.emit(name, json!(payload));
    }
}

pub(crate) fn emit_run_changed(name: &str, run: &WorkflowRun) {
    emit(name, run);
}

pub(crate) fn emit_op_changed(name: &str, op: &WorkflowOp) {
    emit(name, op);
}

pub(crate) fn emit_event(name: &str, event: &WorkflowEvent) {
    emit(name, event);
}
