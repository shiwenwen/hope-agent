//! Workflow parsing and execution machines wired above `ha-core`.

mod preview;
mod runtime_machine;
mod tools;
mod typed_result;

/// Register Workflow feature ports before `ha_core::init_runtime` freezes
/// assembly. Repeated calls from shared shell wiring are safe.
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::workflow::preview::register_workflow_preview_runtime(
            ha_core::workflow::preview::WorkflowPreviewRuntime {
                preview_script: preview::preview_workflow_script,
            },
        )
        .expect("ha-workflow preview runtime must be registered exactly once");
        ha_core::workflow::runtime::register_workflow_typed_result_runtime(
            ha_core::workflow::runtime::WorkflowTypedResultRuntime {
                output_schema: typed_result::workflow_output_schema,
                extract_result: typed_result::extract_workflow_typed_result,
                validate_value: typed_result::validate_workflow_typed_value,
            },
        )
        .expect("ha-workflow typed-result runtime must be registered exactly once");
        ha_core::workflow::runtime::register_workflow_machine_runtime(
            ha_core::workflow::runtime::WorkflowMachineRuntime {
                execute_script: runtime_machine::execute_script,
                has_required_autonomous_budget: runtime_machine::has_required_autonomous_budget,
                spawn_agent_tool_args: runtime_machine::spawn_agent_tool_args,
                wait_all_tool_args: runtime_machine::wait_all_tool_args,
                ensure_visible_agent_runs: runtime_machine::ensure_workflow_visible_agent_run_ids,
                wait_all_output_consumes_results: runtime_machine::wait_all_output_consumes_results,
                ask_user_tool_args: runtime_machine::ask_user_tool_args,
                validation_exit_code: runtime_machine::validation_exit_code,
                validation_child_job_ids: runtime_machine::validation_child_job_ids,
            },
        )
        .expect("ha-workflow QuickJS runtime must be registered exactly once");
        ha_core::tools::registry::register_external_tools(tools::workflow_dispatch_entries())
            .expect("ha-workflow tools must be registered before the registry freezes");
    });
}
