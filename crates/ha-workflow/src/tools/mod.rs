//! Workflow agent-tool handler. The name and schema remain kernel contracts.

mod workflow;

use ha_core::tools::registry::{tool_handler, BuiltinToolEntry};

pub fn workflow_dispatch_entries() -> Vec<BuiltinToolEntry> {
    vec![BuiltinToolEntry {
        name: ha_core::tools::TOOL_WORKFLOW,
        aliases: &[],
        handler: tool_handler!(|args, ctx| workflow::tool_workflow(args, ctx).await),
    }]
}
