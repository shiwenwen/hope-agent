//! Goal execution machines wired above `ha-core`.

mod policy;
mod runner;
pub mod tools;

/// Register the Goal execution port before `ha_core::init_runtime` freezes
/// assembly. Repeated calls from shared shell wiring are safe.
pub fn wire() {
    static WIRED: std::sync::Once = std::sync::Once::new();
    WIRED.call_once(|| {
        ha_core::goal::register_goal_runtime(ha_core::goal::GoalRuntime {
            maybe_schedule_continuation: runner::maybe_schedule_goal_continuation,
            should_evaluate: policy::runner_should_evaluate,
            should_continue: policy::runner_should_continue,
        })
        .expect("ha-goal runtime must be registered exactly once");
        ha_core::tools::registry::register_external_tools(tools::goal_dispatch_entries())
            .expect("ha-goal tool handlers must register before registry freeze");
    });
}
